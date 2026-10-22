#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::should_panic_without_expect)]

extern crate std;

use super::*;

use soroban_sdk::{
    testutils::{storage::Persistent as _, Address as _, Events as _, Ledger, LedgerInfo},
    Address, Env, Vec,
};
use templar_proxy_oracle_soroban_common::{
    Asset, ContractError, PriceData, DEFAULT_TTL_EXTEND_TO, DEFAULT_TTL_THRESHOLD,
};

fn usd(env: &Env) -> Asset {
    Asset::Other(soroban_sdk::Symbol::new(env, "USD"))
}

fn asset(env: &Env, name: &str) -> Asset {
    Asset::Other(soroban_sdk::Symbol::new(env, name))
}

fn set_ledger(env: &Env, timestamp: u64) {
    env.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 25,
        sequence_number: 100,
        max_entry_ttl: 3_110_400,
        ..Default::default()
    });
}

struct Fixture {
    env: Env,
    admin: Address,
    source_id: Address,
    source: PriceSourceClient<'static>,
}

fn fixture() -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    set_ledger(&env, 1_000);
    let admin = Address::generate(&env);
    let source_id = env.register(PriceSource, (&admin,));
    let source = PriceSourceClient::new(&env, &source_id);
    Fixture {
        env,
        admin,
        source_id,
        source,
    }
}

#[test]
fn constructor_sets_owner_and_metadata() {
    let f = fixture();
    assert_eq!(f.source.get_owner(), Some(f.admin));
    assert_eq!(f.source.base(), usd(&f.env));
    assert_eq!(f.source.decimals(), 7);
    assert_eq!(f.source.resolution(), 1);
    assert!(f.source.assets().is_empty());
    assert!(f.source.lastprice(&asset(&f.env, "BTC")).is_none());
}

#[test]
fn set_data_roundtrip_and_reads() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    f.source.set_data(&btc, &12_345_670, &1_000);

    assert_eq!(
        f.source.lastprice(&btc),
        Some(PriceData {
            price: 12_345_670,
            timestamp: 1_000,
        })
    );
    assert_eq!(f.source.assets().len(), 1);
    assert_eq!(f.source.assets().get(0), Some(btc.clone()));
    assert_eq!(f.source.decimals(), 7);
    assert_eq!(f.source.base(), usd(&f.env));
    assert_eq!(
        f.source.price(&btc, &1_000).map(|p| p.price),
        Some(12_345_670)
    );
    assert!(f.source.price(&btc, &999).is_none());
    let history = f.source.prices(&btc, &3).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0), f.source.lastprice(&btc));
    assert!(f.source.prices(&btc, &0).is_none());
    assert!(f.source.lastprice(&asset(&f.env, "ETH")).is_none());
}

#[test]
fn set_data_requires_strictly_increasing_timestamps() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    f.source.set_data(&btc, &10_000_000, &1_000);
    // Equal or older timestamps are rejected: a slot already consumed by
    // the proxy must never be rewritten retroactively.
    assert_eq!(
        f.source.try_set_data(&btc, &11_000_000, &1_000),
        Err(Ok(ContractError::InvalidInput))
    );
    assert_eq!(
        f.source.try_set_data(&btc, &12_000_000, &999),
        Err(Ok(ContractError::InvalidInput))
    );
    f.source.set_data(&btc, &12_000_000, &1_001);
    assert_eq!(
        f.source.lastprice(&btc),
        Some(PriceData {
            price: 12_000_000,
            timestamp: 1_001,
        })
    );
    assert_eq!(f.source.assets().len(), 1);
}

#[test]
#[should_panic]
fn renounce_ownership_cannot_orphan_the_feed() {
    let f = fixture();
    f.source.renounce_ownership();
}

#[test]
fn failed_renounce_leaves_admin_and_feed_intact() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    f.source.set_data(&btc, &1_000_000, &1_000);
    assert!(f.source.try_renounce_ownership().is_err());
    assert_eq!(f.source.get_owner(), Some(f.admin));
    f.source.set_data(&btc, &2_000_000, &1_001);
    assert_eq!(f.source.lastprice(&btc).map(|p| p.price), Some(2_000_000));
}

#[test]
fn two_step_transfer_is_the_admin_change_path() {
    let f = fixture();
    let new_admin = Address::generate(&f.env);
    f.source.transfer_ownership(&new_admin, &200);
    f.source.accept_ownership();
    assert_eq!(f.source.get_owner(), Some(new_admin));
    let btc = asset(&f.env, "BTC");
    f.source.set_data(&btc, &1_000_000, &1_000);
    assert_eq!(f.source.lastprice(&btc).map(|p| p.price), Some(1_000_000));
}

#[test]
#[should_panic]
fn set_data_requires_admin_authorization() {
    let env = Env::default();
    set_ledger(&env, 1_000);
    let admin = Address::generate(&env);
    let source_id = env.register(PriceSource, (&admin,));
    let source = PriceSourceClient::new(&env, &source_id);
    source.set_data(&asset(&env, "BTC"), &1, &1_000);
}

#[test]
fn set_data_rejects_nonpositive_and_zero_timestamp() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    assert_eq!(
        f.source.try_set_data(&btc, &0, &1_000),
        Err(Ok(ContractError::InvalidInput))
    );
    assert_eq!(
        f.source.try_set_data(&btc, &-5, &1_000),
        Err(Ok(ContractError::InvalidInput))
    );
    assert_eq!(
        f.source.try_set_data(&btc, &1, &0),
        Err(Ok(ContractError::InvalidInput))
    );
}

#[test]
fn set_data_bounds_future_timestamps_at_drift_limit() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    assert_eq!(
        f.source.try_set_data(&btc, &1, &4_601),
        Err(Ok(ContractError::InvalidInput))
    );
    f.source.set_data(&btc, &1, &4_600);
    assert_eq!(f.source.lastprice(&btc).map(|p| p.timestamp), Some(4_600));
}

#[test]
fn set_data_rejects_timestamp_regression() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    f.source.set_data(&btc, &1, &2_000);
    assert_eq!(
        f.source.try_set_data(&btc, &2, &1_999),
        Err(Ok(ContractError::InvalidInput))
    );
    assert_eq!(f.source.lastprice(&btc).map(|p| p.timestamp), Some(2_000));
}

#[test]
fn asset_registration_is_capped() {
    let f = fixture();
    for index in 0..64_u32 {
        let name = std::format!("A{index:03}");
        f.source.set_data(
            &asset(&f.env, &name),
            &1_000_000,
            &(1_000 + u64::from(index)),
        );
    }
    assert_eq!(f.source.assets().len(), 64);
    assert_eq!(
        f.source
            .try_set_data(&asset(&f.env, "A064"), &1_000_000, &1_100),
        Err(Ok(ContractError::TooManyAssets))
    );
}

#[test]
fn set_data_publishes_event_and_reads_do_not() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    let before = f.env.events().all();
    assert!(before.filter_by_contract(&f.source_id).events().is_empty());
    f.source.set_data(&btc, &1_000_000, &1_000);
    let events = f.env.events().all();
    assert_eq!(
        events.filter_by_contract(&f.source_id).events().len(),
        1,
        "set_data must emit exactly one PricePublished event"
    );
    f.source.lastprice(&btc);
    f.source.base();
    f.source.assets();
    let after_reads = f.env.events().all();
    assert!(
        after_reads
            .filter_by_contract(&f.source_id)
            .events()
            .is_empty(),
        "read paths must not emit events"
    );
}

#[test]
fn extend_ttl_keeps_entries_alive() {
    let f = fixture();
    let btc = asset(&f.env, "BTC");
    let key = DataKey::Price(btc.clone());
    // The fixture's ledger settings leave the persistent-entry bounds at the
    // SDK defaults, which cannot express this scenario. Name them explicitly:
    // a freshly published entry outlives the extension threshold, and the
    // extend-to target is reachable under this network's maximum entry TTL.
    f.env.ledger().with_mut(|ledger| {
        ledger.min_persistent_entry_ttl = DEFAULT_TTL_THRESHOLD + 1;
        ledger.max_entry_ttl = DEFAULT_TTL_EXTEND_TO + 1;
    });
    f.source.set_data(&btc, &1_000_000, &1_000);
    let initial_ttl = f
        .env
        .as_contract(&f.source_id, || f.env.storage().persistent().get_ttl(&key));
    f.env.ledger().with_mut(|ledger| {
        ledger.sequence_number += initial_ttl - (DEFAULT_TTL_THRESHOLD - 1);
    });
    let before = f
        .env
        .as_contract(&f.source_id, || f.env.storage().persistent().get_ttl(&key));
    assert_eq!(before, DEFAULT_TTL_THRESHOLD - 1);
    f.source.extend_ttl(&btc);
    let after = f
        .env
        .as_contract(&f.source_id, || f.env.storage().persistent().get_ttl(&key));
    assert!(after > before);
    assert_eq!(after, DEFAULT_TTL_EXTEND_TO);
    assert_eq!(f.source.lastprice(&btc).map(|p| p.price), Some(1_000_000));
}

#[test]
fn assets_vec_is_empty_before_first_publish_and_per_asset() {
    let f = fixture();
    let mut none = Vec::new(&f.env);
    none.push_back(asset(&f.env, "BTC"));
    assert!(f.source.assets().is_empty());
    for a in none.iter() {
        assert!(f.source.lastprice(&a).is_none());
    }
    f.source.set_data(&asset(&f.env, "BTC"), &1, &1_000);
    assert_eq!(f.source.assets().len(), 1);
}
