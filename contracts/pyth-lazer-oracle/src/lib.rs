#![no_std]
// Soroban contract entry points require `env: Env` and `Address` by value;
// every `#[contractimpl]` method in this crate is an ABI entry point.
#![allow(clippy::needless_pass_by_value)]

use pyth_lazer_stellar_sdk::{Channel, PythLazerClient, Update};
use soroban_sdk::{
    contract, contracterror, contractimpl, contractmeta, contracttype, symbol_short, Address,
    Bytes, Env, Vec,
};

contractmeta!(key = "sep", val = "40");

/// Testnet-only receiver. The network id is the known SHA-256 of the
/// canonical testnet passphrase; tests derive it independently via the host
/// `crypto::sha256` rather than trusting the constant.
pub(crate) const TESTNET_NETWORK_ID: [u8; 32] = [
    0xce, 0xe0, 0x30, 0x2d, 0x59, 0x84, 0x4d, 0x32, 0xbd, 0xca, 0x91, 0x5c, 0x82, 0x03, 0xdd, 0x44,
    0xb3, 0x3f, 0xbb, 0x7e, 0xdc, 0x19, 0x05, 0x1e, 0xa3, 0x7a, 0xbe, 0xdf, 0x28, 0xec, 0xd4, 0x72,
];

/// Official Pyth Lazer Stellar verifier on testnet. The binding is fixed;
/// there is no constructor argument or setter for it.
pub(crate) const TESTNET_VERIFIER: &str =
    "CAYFT5JE3UQTKT4Q6ZOZK4FXVYVT6RE3MFC7STA4UB6WAEGBT65MRU52";

/// Signed updates must arrive inside this window; stale signed payloads are
/// refused even though the signature itself is still valid.
const MAX_UPDATE_AGE_SECS: u64 = 60;
/// Read-side freshness bound, matching the previous backend oracle.
const MAX_AGE_SECS: u64 = 300;
/// Clock-drift allowance into the future for update/read timestamps.
const MAX_FUTURE_SECS: u64 = 15;
pub(crate) const US_PER_SEC: u64 = 1_000_000;
const REQUIRED_EXPONENT: i16 = -8;
/// 1e-8 -> 1e-7 SEP-40 scaling: divide the mantissa by 10 (floor).
const SCALING: i128 = 10;

const DECIMALS: u32 = 7;
const RESOLUTION: u32 = 1;
const TTL_THRESHOLD: u32 = 518_400;
const TTL_EXTEND_TO: u32 = 3_110_400;

/// XLM feed 23 / testnet SAC.
const XLM_FEED_ID: u32 = 23;
pub(crate) const XLM_ADDRESS: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
/// USDC feed 7 / testnet SAC.
const USDC_FEED_ID: u32 = 7;
pub(crate) const USDC_ADDRESS: &str = "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA";

fn base_asset() -> Asset {
    Asset::Other(symbol_short!("USD"))
}

fn canonical_assets(env: &Env) -> Vec<Asset> {
    let mut assets = Vec::new(env);
    assets.push_back(Asset::Stellar(Address::from_str(env, XLM_ADDRESS)));
    assets.push_back(Asset::Stellar(Address::from_str(env, USDC_ADDRESS)));
    assets
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum Asset {
    Other(soroban_sdk::Symbol),
    Stellar(Address),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
enum StorageKey {
    Price(u32),
}

#[contracterror]
#[repr(u32)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LazerOracleError {
    UnsupportedNetwork = 2,
    SignatureVerificationFailed = 3,
    WrongChannel = 4,
    StaleUpdate = 5,
    UpdateTimestampInFuture = 6,
    MissingFeed = 7,
    DuplicateFeed = 8,
    ForeignFeed = 9,
    MissingProperty = 10,
    WrongExponent = 11,
    NonPositivePrice = 12,
    SameTimestampConflict = 13,
}

#[contract]
pub struct PythLazerOracle;

#[contractimpl]
impl PythLazerOracle {
    /// Binds this instance to the canonical testnet network. The verifier is
    /// a fixed constant; callers cannot substitute one.
    pub fn __constructor(env: Env) -> Result<(), LazerOracleError> {
        if env.ledger().network_id().to_array() != TESTNET_NETWORK_ID {
            return Err(LazerOracleError::UnsupportedNetwork);
        }
        extend_instance_ttl(&env);
        Ok(())
    }

    /// The fixed official verifier this instance authenticates updates
    /// against. Exposed so the keeper can assert its binding matches.
    pub fn verifier(env: Env) -> Address {
        load_verifier(&env)
    }

    /// Permissionless, replay-safe update entry point. `payload` is the raw
    /// LE-ECDSA envelope from the Lazer API (`leEcdsa.data`). The envelope is
    /// authenticated by the official verifier contract; every policy check
    /// (feed set, freshness, monotonicity, scaling) is validated before any
    /// storage write, so a rejected update preserves the previously accepted
    /// prices exactly.
    pub fn update(env: Env, payload: Bytes) -> Result<(), LazerOracleError> {
        let verifier = load_verifier(&env);
        let verified = PythLazerClient::new(&env, &verifier)
            .verify_update(&payload)
            .map_err(|_| LazerOracleError::SignatureVerificationFailed)?;
        let update = verified.into_inner();
        let now = env.ledger().timestamp();
        let accepted = validate_update(&update, now * US_PER_SEC)?;

        // Validate both feeds before changing either accepted price.
        let mut changed = [false; 2];
        for (index, feed) in accepted.iter().enumerate() {
            let current = stored_price(&env, feed.feed_id);
            check_monotonic(current.as_ref(), feed)?;
            changed[index] = current.is_none_or(|price| feed.feed_ts_us > price.timestamp_us);
        }
        for (feed, changed) in accepted.iter().zip(changed) {
            if !changed {
                continue;
            }
            let key = StorageKey::Price(feed.feed_id);
            env.storage().persistent().set(
                &key,
                &StoredPrice {
                    price: feed.price,
                    timestamp_us: feed.feed_ts_us,
                },
            );
            extend_persistent_ttl(&env, &key);
        }
        extend_instance_ttl(&env);
        Ok(())
    }

    // ---- SEP-40 views (latest only; this feed invents no history) ----

    pub fn base(env: Env) -> Asset {
        extend_instance_ttl(&env);
        base_asset()
    }

    pub fn assets(env: Env) -> Vec<Asset> {
        extend_instance_ttl(&env);
        canonical_assets(&env)
    }

    pub fn decimals() -> u32 {
        DECIMALS
    }

    pub fn resolution() -> u32 {
        RESOLUTION
    }

    /// Latest stored price if fresh; older than `MAX_AGE_SECS`, further in
    /// the future than `MAX_FUTURE_SECS`, or never updated yields `None`.
    /// The stored microsecond timestamp is exposed here as floor(seconds).
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        extend_instance_ttl(&env);
        let feed_id = feed_id_of(&env, &asset)?;
        let now_us = env.ledger().timestamp() * US_PER_SEC;
        let data = stored_price(&env, feed_id)?;
        if data.timestamp_us < now_us.saturating_sub(MAX_AGE_SECS * US_PER_SEC)
            || data.timestamp_us > now_us.saturating_add(MAX_FUTURE_SECS * US_PER_SEC)
        {
            return None;
        }
        Some(PriceData {
            price: data.price,
            timestamp: data.timestamp_us / US_PER_SEC,
        })
    }

    /// Latest-only: `prices` returns at most the single validated latest
    /// quote. `records == 0` is `None` by the SEP-40 parent ABI.
    pub fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        if records == 0 {
            return None;
        }
        let latest = Self::lastprice(env.clone(), asset)?;
        let mut out = Vec::new(&env);
        out.push_back(latest);
        Some(out)
    }

    /// No history is kept, so there is no point-in-time price to return.
    pub fn price(_env: Env, _asset: Asset, _timestamp: u64) -> Option<PriceData> {
        None
    }
}

/// A policy-validated feed update ready for storage, keyed by feed id.
struct FeedUpdate {
    feed_id: u32,
    price: i128,
    feed_ts_us: u64,
}

/// Latest accepted state, kept in full microsecond precision. The public
/// SEP-40 `PriceData` exposes floor(seconds) only.
#[contracttype]
#[derive(Clone)]
struct StoredPrice {
    price: i128,
    timestamp_us: u64,
}

/// The complete policy check on a *verified* update. `now_us` is ledger time
/// in microseconds. Only the two canonical feeds, exactly once each, on the
/// fixed-rate@200ms channel, each with required `feed_update_timestamp`.
fn validate_update(update: &Update, now_us: u64) -> Result<[FeedUpdate; 2], LazerOracleError> {
    if update.channel != Channel::FixedRate200ms {
        return Err(LazerOracleError::WrongChannel);
    }
    check_age(update.timestamp, now_us)?;
    let mut accepted = [
        FeedUpdate {
            feed_id: 0,
            price: 0,
            feed_ts_us: 0,
        },
        FeedUpdate {
            feed_id: 0,
            price: 0,
            feed_ts_us: 0,
        },
    ];
    let mut n = 0usize;
    for feed in update.feeds.iter() {
        if feed.feed_id != XLM_FEED_ID && feed.feed_id != USDC_FEED_ID {
            return Err(LazerOracleError::ForeignFeed);
        }
        if accepted[..n].iter().any(|f| f.feed_id == feed.feed_id) {
            return Err(LazerOracleError::DuplicateFeed);
        }
        let exponent = feed.exponent.ok_or(LazerOracleError::MissingProperty)?;
        if exponent != REQUIRED_EXPONENT {
            return Err(LazerOracleError::WrongExponent);
        }
        let raw = feed.price.ok_or(LazerOracleError::MissingProperty)?;
        let price = i128::from(raw) / SCALING;
        if price <= 0 {
            return Err(LazerOracleError::NonPositivePrice);
        }
        // Feed observations may predate the bundle, but cannot occur after it.
        // Their own timestamps, not the bundle timestamp, determine freshness.
        let feed_ts_us = feed
            .feed_update_timestamp
            .ok_or(LazerOracleError::MissingProperty)?;
        if feed_ts_us > update.timestamp {
            return Err(LazerOracleError::UpdateTimestampInFuture);
        }
        check_age(feed_ts_us, now_us)?;
        accepted[n] = FeedUpdate {
            feed_id: feed.feed_id,
            price,
            feed_ts_us,
        };
        n += 1;
    }
    // Exactly the canonical set: each id once, no subset.
    if n != 2 {
        return Err(LazerOracleError::MissingFeed);
    }
    Ok(accepted)
}

fn check_age(ts_us: u64, now_us: u64) -> Result<(), LazerOracleError> {
    if ts_us == 0 || ts_us < now_us.saturating_sub(MAX_UPDATE_AGE_SECS * US_PER_SEC) {
        return Err(LazerOracleError::StaleUpdate);
    }
    if ts_us > now_us.saturating_add(MAX_FUTURE_SECS * US_PER_SEC) {
        return Err(LazerOracleError::UpdateTimestampInFuture);
    }
    Ok(())
}

/// Rejects whole updates that would regress stored data: older feed
/// microsecond timestamps, and different prices conflicting at the exact
/// same microsecond. An identical (timestamp, price) pair is a no-op —
/// safe against replay and permissionless front-running by another
/// publisher of the same payload.
fn check_monotonic(
    current: Option<&StoredPrice>,
    feed: &FeedUpdate,
) -> Result<(), LazerOracleError> {
    let Some(current) = current else {
        return Ok(());
    };
    if feed.feed_ts_us < current.timestamp_us {
        return Err(LazerOracleError::StaleUpdate);
    }
    if feed.feed_ts_us == current.timestamp_us && feed.price != current.price {
        return Err(LazerOracleError::SameTimestampConflict);
    }
    Ok(())
}

fn stored_price(env: &Env, feed_id: u32) -> Option<StoredPrice> {
    let key = StorageKey::Price(feed_id);
    let data: Option<StoredPrice> = env.storage().persistent().get(&key);
    if data.is_some() {
        extend_persistent_ttl(env, &key);
    }
    data
}

/// Canonical SEP-40 asset -> Lazer feed id. Anything else is unlisted.
fn feed_id_of(env: &Env, asset: &Asset) -> Option<u32> {
    match asset {
        Asset::Stellar(a) if *a == Address::from_str(env, XLM_ADDRESS) => Some(XLM_FEED_ID),
        Asset::Stellar(a) if *a == Address::from_str(env, USDC_ADDRESS) => Some(USDC_FEED_ID),
        _ => None,
    }
}

fn load_verifier(env: &Env) -> Address {
    extend_instance_ttl(env);
    Address::from_str(env, TESTNET_VERIFIER)
}

fn extend_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn extend_persistent_ttl(env: &Env, key: &StorageKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

#[cfg(test)]
mod test;
