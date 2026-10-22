#![no_std]
// Soroban contract entry points require `env: Env` and reference types by value;
// every `#[contractimpl]` method in this crate is an ABI entry point.
#![allow(clippy::needless_pass_by_value)]

use soroban_sdk::{
    contract, contractevent, contractimpl, contracttype, symbol_short, Address, Env, Vec,
};
use stellar_access::ownable::{set_owner, Ownable};
use stellar_macros::only_owner;
use templar_proxy_oracle_soroban_common::{
    extend_instance_ttl, Asset, ContractError, PriceData, PriceFeedTrait, DEFAULT_TTL_EXTEND_TO,
    DEFAULT_TTL_THRESHOLD,
};

/// Quote base every price on this feed is denominated in. The proxy-oracle
/// rejects any source whose `base()` differs from its registered
/// `source_base`, so this must stay in sync with the proxy deployment it is
/// registered against.
const BASE_USD: soroban_sdk::Symbol = symbol_short!("USD");

/// Fixed decimals of every published price (SEP-40 fixed-point scale).
/// The market plan registers synthetic sources at exactly 7 decimals; the
/// proxy rejects source sets whose feeds report more than 18.
const DECIMALS: u32 = 7;

/// Bucket size in seconds for `price`/`prices` lookups. This publisher emits
/// latest-only data, so the smallest legal resolution applies.
const RESOLUTION: u32 = 1;

/// Upper bound on distinct assets this publisher will track, matching the
/// proxy-oracle's `MAX_REGISTERED_ASSETS`.
const MAX_ASSETS: u32 = 64;

/// A published price may not be dated further in the future than this clock
/// drift. The proxy's `FreshnessFilter` drops future-dated source prices
/// beyond its configured drift bound; rejecting writes here keeps every
/// stored price publishable.
const MAX_FUTURE_SECS: u64 = 3_600;

/// Latest published price per asset, plus the registered-asset list.
#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Price(Asset),
    Assets,
}

#[contractevent]
#[derive(Clone)]
pub struct PricePublished {
    pub asset: Asset,
    pub price: i128,
    pub timestamp: u64,
}

/// Minimal admin-gated price publisher.
///
/// This is a **synthetic single-admin source**: one administrator signs every
/// update and there is no claim of external independence or provenance. It is
/// intended solely as a registered quorum source for the protected testnet
/// market, where the operator deploys three instances under the same admin.
///
/// The read surface implements SEP-40 `PriceFeedTrait`, matching what the
/// proxy-oracle's aggregation path reads from every registered source:
/// `base()`, `decimals()`, and `lastprice(asset)`. Only the latest price per
/// asset is retained.
///
/// Build requirement: this crate pulls `stellar-access`, which enables
/// soroban-sdk's `experimental_spec_shaking_v2` feature, so deployable WASM
/// must be produced with `stellar contract build` from stellar-cli v25.2.0+
/// (not plain `cargo build`) — the same policy as the existing contracts in
/// `contract/proxy-oracle/soroban/`.
#[contract]
pub struct PriceSource;

#[contractimpl]
impl PriceSource {
    /// Initializes the publisher. `owner` is the sole administrator allowed to
    /// publish prices and is expected to be the market operator's admin.
    pub fn __constructor(env: Env, owner: Address) -> Result<(), ContractError> {
        extend_instance_ttl(&env);
        set_owner(&env, &owner);
        Ok(())
    }

    /// Publishes the latest price for `asset` at `timestamp` (ledger seconds).
    ///
    /// Admin-only. Rejects non-positive prices, timestamps further than
    /// [`MAX_FUTURE_SECS`] ahead of the current ledger time, and timestamps
    /// not strictly greater than the previously stored price for the asset
    /// (publication time strictly increases, so a slot already consumed by
    /// the proxy can never be retroactively rewritten). The first publish
    /// for an asset registers it in `assets()`.
    #[only_owner]
    pub fn set_data(
        env: Env,
        asset: Asset,
        price: i128,
        timestamp: u64,
    ) -> Result<(), ContractError> {
        extend_instance_ttl(&env);
        if price <= 0 || timestamp == 0 {
            return Err(ContractError::InvalidInput);
        }
        let now = env.ledger().timestamp();
        if timestamp > now.saturating_add(MAX_FUTURE_SECS) {
            return Err(ContractError::InvalidInput);
        }
        let key = DataKey::Price(asset.clone());
        let previous: Option<PriceData> = env.storage().persistent().get(&key);
        if previous
            .as_ref()
            .is_some_and(|prev| timestamp <= prev.timestamp)
        {
            return Err(ContractError::InvalidInput);
        }
        if previous.is_none() {
            let mut assets = load_assets(&env);
            if assets.len() >= MAX_ASSETS {
                return Err(ContractError::TooManyAssets);
            }
            assets.push_back(asset.clone());
            env.storage().instance().set(&DataKey::Assets, &assets);
        }
        let published = PriceData { price, timestamp };
        env.storage().persistent().set(&key, &published);
        env.storage()
            .persistent()
            .extend_ttl(&key, DEFAULT_TTL_THRESHOLD, DEFAULT_TTL_EXTEND_TO);
        PricePublished {
            asset,
            price,
            timestamp,
        }
        .publish(&env);
        Ok(())
    }

    /// Permissionless TTL maintenance for `asset`'s persistent price entry.
    pub fn extend_ttl(env: Env, asset: Asset) {
        extend_instance_ttl(&env);
        let key = DataKey::Price(asset);
        let storage = env.storage().persistent();
        if storage.has(&key) {
            storage.extend_ttl(&key, DEFAULT_TTL_THRESHOLD, DEFAULT_TTL_EXTEND_TO);
        }
    }
}

#[contractimpl(contracttrait)]
impl Ownable for PriceSource {
    /// The stock renounce removes the owner key irreversibly (`set_owner` is
    /// constructor-only), which would silently orphan a live quorum source:
    /// `lastprice` would keep serving until stale, then no admin could ever
    /// republish. Admin changes therefore go exclusively through the 2-step
    /// `transfer_ownership`/`accept_ownership` flow.
    fn renounce_ownership(env: &Env) {
        env.panic_with_error(ContractError::InvalidInput);
    }
}

#[contractimpl]
impl PriceFeedTrait for PriceSource {
    fn base(env: Env) -> Asset {
        extend_instance_ttl(&env);
        Asset::Other(BASE_USD)
    }

    fn assets(env: Env) -> Vec<Asset> {
        extend_instance_ttl(&env);
        load_assets(&env)
    }

    fn decimals(env: Env) -> u32 {
        extend_instance_ttl(&env);
        DECIMALS
    }

    fn resolution(env: Env) -> u32 {
        extend_instance_ttl(&env);
        RESOLUTION
    }

    /// Returns the stored price only when `timestamp` names the bucket of the
    /// latest publication; this feed keeps no historical buckets.
    fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        let latest = latest(&env, &asset)?;
        (latest.timestamp == timestamp).then_some(latest)
    }

    /// Returns at most the single latest record; this feed keeps no history.
    fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        if records == 0 {
            return None;
        }
        let latest = latest(&env, &asset)?;
        let mut out = Vec::new(&env);
        out.push_back(latest);
        Some(out)
    }

    fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        latest(&env, &asset)
    }
}

fn load_assets(env: &Env) -> Vec<Asset> {
    env.storage()
        .instance()
        .get(&DataKey::Assets)
        .unwrap_or_else(|| Vec::new(env))
}

fn latest(env: &Env, asset: &Asset) -> Option<PriceData> {
    let key = DataKey::Price(asset.clone());
    let price: Option<PriceData> = env.storage().persistent().get(&key);
    if price.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, DEFAULT_TTL_THRESHOLD, DEFAULT_TTL_EXTEND_TO);
    }
    price
}

#[cfg(test)]
mod tests;
