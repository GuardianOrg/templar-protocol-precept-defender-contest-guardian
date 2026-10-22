//! Smoke for the optimized artifact at the frozen path, gated on `wasm-smoke`.
//! Gate: `just test-optimized-wasm` (depends on `optimize`, which depends on `build`),
//! so the artifact exists before this reads it.
//! Behavioral checks use the crate's exported client against the deployed optimized Wasm.
#![cfg(feature = "wasm-smoke")]
#![allow(
    clippy::needless_pass_by_value,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::fs;
use std::vec::Vec as StdVec;

use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{storage::Instance as _, Address as _, Ledger},
    Address, Env, Symbol, Vec,
};
use templar_proxy_oracle_soroban_common::{
    Asset, ContractError, NormalizedPrice, PriceData, ProxyOracleTrait, DEFAULT_TTL_EXTEND_TO,
    DEFAULT_TTL_THRESHOLD,
};

use pool_oracle_adapter::{PoolOracleAdapterClient, PoolOracleConfig};

const OPTIMIZED_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/target/wasm32v1-none/release-soroban-checked/pool_oracle_adapter.optimized.wasm"
);

fn artifact() -> StdVec<u8> {
    fs::read(OPTIMIZED_WASM)
        .expect("optimized wasm missing; run the release-soroban-checked build + optimize first")
}

#[derive(Clone)]
#[contracttype]
enum Key {
    Base,
    Owner,
    Assets,
    Latest(Asset),
    History(Asset),
    Trap,
}

#[contract]
struct MockParent;

fn trap(env: &Env) {
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&Key::Trap)
        .unwrap_or(false)
    {
        soroban_sdk::panic_with_error!(env, ContractError::SourceUnavailable);
    }
}

#[contractimpl]
impl MockParent {
    pub fn __constructor(env: Env, base: Asset, owner: Address, assets: Vec<Asset>) {
        env.storage().instance().set(&Key::Base, &base);
        env.storage().instance().set(&Key::Owner, &owner);
        env.storage().persistent().set(&Key::Assets, &assets);
    }

    pub fn set_assets(env: Env, assets: Vec<Asset>) {
        env.storage().persistent().set(&Key::Assets, &assets);
    }

    pub fn set_quotes(
        env: Env,
        asset: Asset,
        latest: Option<NormalizedPrice>,
        history: Vec<NormalizedPrice>,
    ) {
        env.storage()
            .persistent()
            .set(&Key::Latest(asset.clone()), &latest);
        env.storage()
            .persistent()
            .set(&Key::History(asset), &history);
    }

    pub fn set_trap(env: Env, enabled: bool) {
        env.storage().persistent().set(&Key::Trap, &enabled);
    }

    pub fn registered_assets(env: Env) -> Result<Vec<Asset>, ContractError> {
        trap(&env);
        Ok(env.storage().persistent().get(&Key::Assets).unwrap())
    }

    pub fn get_owner(env: Env) -> Option<Address> {
        trap(&env);
        env.storage().instance().get(&Key::Owner)
    }
}

/// Quotes only reach assets the parent still registers, so deregistration is what
/// removes a price — not the absence of a stored quote.
fn registered(env: &Env, asset: &Asset) -> bool {
    env.storage()
        .persistent()
        .get::<_, Vec<Asset>>(&Key::Assets)
        .unwrap()
        .contains(asset)
}

#[contractimpl]
impl ProxyOracleTrait for MockParent {
    fn source_base(env: Env) -> Option<Asset> {
        trap(&env);
        env.storage().instance().get(&Key::Base)
    }

    fn aggregated_latest(env: Env, asset: Asset) -> Option<NormalizedPrice> {
        trap(&env);
        if !registered(&env, &asset) {
            return None;
        }
        env.storage()
            .persistent()
            .get::<_, Option<NormalizedPrice>>(&Key::Latest(asset))
            .flatten()
    }

    fn aggregated_history(env: Env, asset: Asset, records: u32) -> Option<Vec<NormalizedPrice>> {
        trap(&env);
        if records == 0 || !registered(&env, &asset) {
            return None;
        }
        let history: Vec<NormalizedPrice> = env.storage().persistent().get(&Key::History(asset))?;
        if history.is_empty() {
            return None;
        }
        Some(history.slice(history.len().saturating_sub(records)..))
    }
}

struct Fixture {
    env: Env,
    parent: MockParentClient<'static>,
    adapter: PoolOracleAdapterClient<'static>,
    config: PoolOracleConfig,
    a: Asset,
    b: Asset,
}

fn fixture(decimals: u32, resolution: u32) -> Fixture {
    let wasm = artifact();
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| {
        ledger.protocol_version = 25;
        // Quotes are stamped at 107; keep them before ledger time so `lastprice`'s
        // future-dated guard (L-05) does not reject them.
        ledger.timestamp = 110;
        ledger.sequence_number = 100;
        // +1: the host clamps an extension to `sequence + max_entry_ttl - 1`.
        ledger.max_entry_ttl = DEFAULT_TTL_EXTEND_TO + 1;
    });

    let base = Asset::Other(Symbol::new(&env, "USD"));
    let owner = Address::generate(&env);
    let a = Asset::Stellar(Address::generate(&env));
    let b = Asset::Stellar(Address::generate(&env));
    let parent_id = env.register(
        MockParent,
        (&base, &owner, Vec::from_array(&env, [b.clone(), a.clone()])),
    );
    // Deploys the optimized artifact through the test deployer (which uploads it)
    // and runs its constructor, so the whole path is the Wasm one.
    let adapter_id = env.register_at(
        &Address::generate(&env),
        &wasm[..],
        (
            owner.clone(),
            parent_id.clone(),
            decimals,
            resolution,
            base.clone(),
        ),
    );
    let config = PoolOracleConfig {
        parent_oracle: parent_id.clone(),
        deployment_owner: owner,
        decimals,
        resolution,
        base,
    };
    Fixture {
        env: env.clone(),
        parent: MockParentClient::new(&env, &parent_id),
        adapter: PoolOracleAdapterClient::new(&env, &adapter_id),
        config,
        a,
        b,
    }
}

fn quote(env: &Env, f: &Fixture, asset: &Asset, mantissa: i64, expo: i32) {
    f.parent.set_quotes(
        asset,
        &Some(NormalizedPrice {
            mantissa,
            expo,
            timestamp: 107,
        }),
        &Vec::from_array(
            env,
            [NormalizedPrice {
                mantissa,
                expo,
                timestamp: 107,
            }],
        ),
    );
}

#[test]
fn deployed_wasm_constructs_reports_metadata_and_prices_two_assets() {
    let f = fixture(8, 10);
    assert_eq!(&f.adapter.config(), &f.config);
    assert_eq!(f.adapter.interface_version(), 1);
    assert_eq!(f.adapter.storage_version(), 1);
    assert_eq!(f.adapter.base(), f.config.base.clone());
    assert_eq!(f.adapter.decimals(), 8);
    assert_eq!(f.adapter.resolution(), 10);

    quote(&f.env, &f, &f.a, 123, -2);
    quote(&f.env, &f, &f.b, 5, -1);
    let expected_a = PriceData {
        price: 123_000_000,
        timestamp: 100,
    };
    let expected_b = PriceData {
        price: 50_000_000,
        timestamp: 100,
    };
    for (asset, expected) in [(&f.a, expected_a), (&f.b, expected_b)] {
        assert_eq!(
            f.adapter.lastprice(asset),
            Some(expected.clone()),
            "{asset:?} latest"
        );
        assert_eq!(
            f.adapter.price(asset, &100),
            Some(expected.clone()),
            "{asset:?} by bucket"
        );
        assert_eq!(
            f.adapter.prices(asset, &32),
            Some(Vec::from_array(&f.env, [expected])),
            "{asset:?} history"
        );
    }
}

#[test]
fn deployed_wasm_mirrors_parent_registry_order_and_removal() {
    let f = fixture(8, 10);
    assert_eq!(
        f.adapter.assets(),
        Vec::from_array(&f.env, [f.b.clone(), f.a.clone()])
    );

    let c = Asset::Stellar(Address::generate(&f.env));
    f.parent.set_assets(&Vec::from_array(
        &f.env,
        [f.b.clone(), c.clone(), f.a.clone()],
    ));
    assert_eq!(
        f.adapter.assets(),
        Vec::from_array(&f.env, [f.b.clone(), c, f.a.clone()])
    );

    // Quote both assets first, so the removed asset going `None` can only come from
    // the parent registry, not from it having never been priced.
    quote(&f.env, &f, &f.a, 123, -2);
    quote(&f.env, &f, &f.b, 5, -1);
    let expected_a = PriceData {
        price: 123_000_000,
        timestamp: 100,
    };
    let expected_b = PriceData {
        price: 50_000_000,
        timestamp: 100,
    };
    assert_eq!(f.adapter.lastprice(&f.a), Some(expected_a));
    f.parent.set_assets(&Vec::from_array(&f.env, [f.b.clone()]));
    assert_eq!(f.adapter.assets(), Vec::from_array(&f.env, [f.b.clone()]));
    assert_eq!(f.adapter.lastprice(&f.b), Some(expected_b));
    assert_eq!(
        f.adapter.lastprice(&f.a),
        None,
        "deregistered asset must stop pricing"
    );
    assert_eq!(f.adapter.price(&f.a, &100), None);
    assert_eq!(f.adapter.prices(&f.a, &32), None);
}

#[test]
fn deployed_wasm_reads_fail_closed_on_parent_failure_and_ignore_latest_for_history() {
    let f = fixture(8, 10);
    quote(&f.env, &f, &f.a, 123, -2);
    let expected = PriceData {
        price: 123_000_000,
        timestamp: 100,
    };
    assert_eq!(f.adapter.lastprice(&f.a), Some(expected.clone()));

    f.parent.set_trap(&true);
    assert_eq!(f.adapter.lastprice(&f.a), None);
    assert_eq!(f.adapter.price(&f.a, &100), None);
    assert_eq!(f.adapter.prices(&f.a, &32), None);
    assert!(
        f.adapter.try_assets().is_err(),
        "a broken registry must not look empty"
    );
    f.parent.set_trap(&false);
    assert_eq!(f.adapter.lastprice(&f.a), Some(expected.clone()));

    // Accepted history stays readable when the latest slot is empty: the adapter
    // must not synthesize or suppress freshness from `aggregated_latest`.
    f.parent.set_quotes(
        &f.a,
        &None,
        &Vec::from_array(
            &f.env,
            [NormalizedPrice {
                mantissa: 123,
                expo: -2,
                timestamp: 107,
            }],
        ),
    );
    assert_eq!(f.adapter.lastprice(&f.a), None);
    assert_eq!(f.adapter.price(&f.a, &100), Some(expected.clone()));
    assert_eq!(
        f.adapter.prices(&f.a, &32),
        Some(Vec::from_array(&f.env, [expected]))
    );
    assert_eq!(f.adapter.prices(&f.a, &0), None);
    assert_eq!(f.adapter.price(&f.a, &101), None);
}

#[test]
fn deployed_wasm_extends_ttl_and_keeps_config() {
    let f = fixture(8, 10);
    let ttl =
        |env: &Env| env.as_contract(&f.adapter.address, || env.storage().instance().get_ttl());
    // Compute the delta outside `with_mut`: calling `ttl()` inside the closure
    // re-borrows the ledger mutably and traps the host.
    let near = ttl(&f.env) - (DEFAULT_TTL_THRESHOLD - 1);
    f.env
        .ledger()
        .with_mut(|ledger| ledger.sequence_number += near);
    assert_eq!(ttl(&f.env), DEFAULT_TTL_THRESHOLD - 1);
    // Permissionlessness itself is asserted natively, where the auth manager is live.
    f.adapter.extend_ttl();
    assert_eq!(ttl(&f.env), DEFAULT_TTL_EXTEND_TO);
    assert_eq!(&f.adapter.config(), &f.config);
    assert_eq!(f.adapter.decimals(), 8);
    assert_eq!(&f.adapter.config(), &f.config);
}
