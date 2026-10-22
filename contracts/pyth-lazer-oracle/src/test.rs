//! Regression tests for the Pyth Lazer receiver.
//!
//! The envelope tests run real signature crypto in-process: the *official*
//! verifier contract instance is registered at the fixed testnet verifier
//! address with a controlled trust list (the fixture's host-recovered
//! pubkey). The fixture signature itself was produced by the live Lazer
//! signers; the parent's live probe binds the *deployed* trust list, these
//! tests bind the verifier *code*.

extern crate alloc;

use crate::{
    check_age, check_monotonic, stored_price, validate_update, Asset, FeedUpdate, LazerOracleError,
    PythLazerOracle, PythLazerOracleClient, StoredPrice, USDC_ADDRESS, XLM_ADDRESS,
};
use hex_literal::hex;
use pyth_lazer_stellar::{PythLazerContract, PythLazerContractClient};
use pyth_lazer_stellar_sdk::{Channel, Feed, Update};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, BytesN, Env,
};

/// leEcdsa envelope of the real captured public response for feeds 23/7
/// (`rc-xlmusdc-20261009/lazer-access-public-response.json`, base64-decoded).
const FIXTURE: [u8; 139] = hex!(
    "e4bd474d140b83b8d2a99c63cfbd7b42e5fb458d354ae0395b5751daca6d3ab2"
    "b77f8abf12c9998d4c4323fa0bb9e2ce5c497164c9e429189eee2639dbf3764d"
    "ca29d23801440075d3c793c08feea66b5d06000302170000000300e71e260100"
    "00000004f8ff0c01c08feea66b5d060007000000030088cbf5050000000004f8"
    "ff0c01c08feea66b5d0600"
);
const FIXTURE_TS_S: u64 = 1_791_566_803;
const FIXTURE_TS_US: u64 = 1_791_566_803_800_000;

/// Mantissas scaled to SEP-40 7-decimals: `price / 10`, floor.
const XLM_RAW: i64 = 19_275_495;
const USDC_RAW: i64 = 99_994_504;
const XLM_PRICE: i128 = 1_927_549;
const USDC_PRICE: i128 = 9_999_450;

// The fixture derives the testnet id itself; it is copied from neither the
// production constant nor the mainnet id.
const TESTNET_PASSPHRASE: &[u8] = b"Test SDF Network ; September 2015";
const MAINNET_PASSPHRASE: &[u8] = b"Public Global Stellar Network ; September 2015";

fn sha256(env: &Env, passphrase: &[u8]) -> [u8; 32] {
    env.crypto()
        .sha256(&Bytes::from_slice(env, passphrase))
        .to_array()
}

// Testnet verifier registered at its literal official address, independent
// of the production constant: a wrong hardcoded binding on the receiver then
// breaks the signed-fixture acceptance tests instead of silently passing.
const TESTNET_VERIFIER: &str = "CAYFT5JE3UQTKT4Q6ZOZK4FXVYVT6RE3MFC7STA4UB6WAEGBT65MRU52";

fn ledger(env: &Env, timestamp: u64, network_id: [u8; 32]) {
    env.ledger().with_mut(|li| {
        li.protocol_version = 26;
        li.timestamp = timestamp;
        li.network_id = network_id;
    });
}

struct Fixture {
    env: Env,
    oracle_id: Address,
    lazer_id: Address,
}

fn setup(network_id: [u8; 32]) -> Fixture {
    let env = Env::default();
    env.mock_all_auths();
    ledger(&env, FIXTURE_TS_S, network_id);
    let executor = Address::generate(&env);
    let lazer_id = env.register_at(
        &Address::from_str(&env, TESTNET_VERIFIER),
        PythLazerContract,
        (executor, None::<BytesN<33>>, None::<u64>),
    );
    // The SDK runs __constructor for us; it binds or refuses the network.
    let oracle_id = env.register(PythLazerOracle, ());
    Fixture {
        env,
        oracle_id,
        lazer_id,
    }
}

fn testnet() -> Fixture {
    let env = Env::default();
    setup(sha256(&env, TESTNET_PASSPHRASE))
}

fn oracle(fx: &Fixture) -> PythLazerOracleClient<'_> {
    PythLazerOracleClient::new(&fx.env, &fx.oracle_id)
}

fn fixture_pubkey(fx: &Fixture) -> BytesN<33> {
    let signature = BytesN::from_array(&fx.env, FIXTURE[4..68].try_into().unwrap());
    let payload = fx
        .env
        .crypto()
        .keccak256(&Bytes::from_slice(&fx.env, &FIXTURE[71..]));
    let uncompressed = fx
        .env
        .crypto()
        .secp256k1_recover(&payload, &signature, FIXTURE[68] as u32);
    let raw = uncompressed.to_array();
    let mut compressed = [0u8; 33];
    compressed[0] = 2 + (raw[64] & 1);
    compressed[1..].copy_from_slice(&raw[1..33]);
    BytesN::from_array(&fx.env, &compressed)
}

fn accepted_fixture() -> Fixture {
    let fx = testnet();
    PythLazerContractClient::new(&fx.env, &fx.lazer_id)
        .update_trusted_signer(&fixture_pubkey(&fx), &u64::MAX);
    fx
}

/// Wraps the generated `try_update` into the typed contract error,
/// following the upstream sdk_client_test mapping.
fn try_update(fx: &Fixture, payload: &Bytes) -> Result<(), LazerOracleError> {
    match oracle(fx).try_update(payload) {
        Ok(Ok(())) => Ok(()),
        Err(Ok(e)) => Err(e),
        // The receiver wraps every verifier failure into a typed error, so
        // decode and untyped host errors are never expected here.
        Ok(Err(_)) | Err(Err(_)) => panic!("unexpected untyped invoke failure"),
    }
}

fn stored(fx: &Fixture, feed_id: u32) -> Option<StoredPrice> {
    fx.env
        .as_contract(&fx.oracle_id, || stored_price(&fx.env, feed_id))
}

fn canonical_assets(env: &Env) -> (Asset, Asset) {
    (
        Asset::Stellar(Address::from_str(env, XLM_ADDRESS)),
        Asset::Stellar(Address::from_str(env, USDC_ADDRESS)),
    )
}

fn assert_prices_intact(fx: &Fixture) {
    let oracle = oracle(fx);
    let (xlm, usdc) = canonical_assets(&fx.env);
    assert_eq!(oracle.lastprice(&xlm).unwrap().price, XLM_PRICE);
    assert_eq!(oracle.lastprice(&usdc).unwrap().price, USDC_PRICE);
    assert_eq!(oracle.lastprice(&xlm).unwrap().timestamp, FIXTURE_TS_S);
    assert_eq!(oracle.lastprice(&usdc).unwrap().timestamp, FIXTURE_TS_S);
}

// ---- network binding ------------------------------------------------------

#[test]
fn testnet_network_id_derives_from_canonical_passphrase() {
    let env = Env::default();
    assert_eq!(sha256(&env, TESTNET_PASSPHRASE), crate::TESTNET_NETWORK_ID);
    let mainnet = sha256(&env, MAINNET_PASSPHRASE);
    assert_ne!(crate::TESTNET_NETWORK_ID, mainnet);
    assert_eq!(
        mainnet,
        hex!("7ac33997544e3175d266bd022439b22cdb16508c01163f26e5cb2a3e1045a979")
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn mainnet_network_id_rejected() {
    let env = Env::default();
    let mainnet = sha256(&env, MAINNET_PASSPHRASE);
    setup(mainnet); // register runs __constructor under the mainnet id
}

// ---- on-chain: official verifier code + real captured envelope ------------

#[test]
fn accepted_envelope_stores_scaled_prices_and_timestamps() {
    let fx = accepted_fixture();
    assert_eq!(
        try_update(&fx, &Bytes::from_slice(&fx.env, &FIXTURE[..])),
        Ok(())
    );
    let oracle = oracle(&fx);
    let (xlm, usdc) = canonical_assets(&fx.env);
    // SEP-40 view: floor(seconds) of the signed feed timestamp.
    assert_eq!(oracle.lastprice(&xlm).unwrap().price, XLM_PRICE);
    assert_eq!(oracle.lastprice(&usdc).unwrap().price, USDC_PRICE);
    assert_eq!(oracle.lastprice(&xlm).unwrap().timestamp, FIXTURE_TS_S);
    assert_eq!(oracle.lastprice(&usdc).unwrap().timestamp, FIXTURE_TS_S);
    // Private storage keeps full microseconds.
    assert_eq!(stored(&fx, 23).unwrap().timestamp_us, FIXTURE_TS_US);
    assert_eq!(stored(&fx, 7).unwrap().timestamp_us, FIXTURE_TS_US);
    // Latest-only ABI.
    assert_eq!(oracle.prices(&xlm, &1).unwrap().len(), 1);
    assert_eq!(oracle.price(&xlm, &FIXTURE_TS_S), None);
}

#[test]
fn identical_replay_is_no_op_preserving_both_prices() {
    let fx = accepted_fixture();
    let payload = Bytes::from_slice(&fx.env, &FIXTURE[..]);
    assert_eq!(try_update(&fx, &payload), Ok(()));
    assert_eq!(try_update(&fx, &payload), Ok(()));
    assert_prices_intact(&fx);
}

fn rejected_update_preserves_prices(fx: &Fixture, payload: &[u8]) {
    assert_eq!(
        try_update(fx, &Bytes::from_slice(&fx.env, payload)).err(),
        Some(LazerOracleError::SignatureVerificationFailed)
    );
    assert_prices_intact(fx);
}

#[test]
fn tampered_payload_refused_and_prices_preserved() {
    let fx = accepted_fixture();
    assert_eq!(
        try_update(&fx, &Bytes::from_slice(&fx.env, &FIXTURE[..])),
        Ok(())
    );
    // Parent's live probe bit-flipped payload byte 80; the official verifier
    // refused the envelope.
    let mut tampered = FIXTURE;
    tampered[80] ^= 0xFF;
    rejected_update_preserves_prices(&fx, &tampered);
}

#[test]
fn unsigned_payload_refused_and_prices_preserved() {
    let fx = accepted_fixture();
    assert_eq!(
        try_update(&fx, &Bytes::from_slice(&fx.env, &FIXTURE[..])),
        Ok(())
    );
    // Bare payload: the envelope magic check fails before any parse.
    rejected_update_preserves_prices(&fx, &FIXTURE[71..]);
}

#[test]
fn revoked_signer_refused_and_prices_preserved() {
    let fx = accepted_fixture();
    assert_eq!(
        try_update(&fx, &Bytes::from_slice(&fx.env, &FIXTURE[..])),
        Ok(())
    );
    // expires_at == 0 removes the signer; replaying the same envelope now
    // fails verification (the path another publisher's front-run hits).
    PythLazerContractClient::new(&fx.env, &fx.lazer_id)
        .update_trusted_signer(&fixture_pubkey(&fx), &0);
    rejected_update_preserves_prices(&fx, &FIXTURE[..]);
}

// ---- read-side freshness and fail-closed reads -----------------------------

#[test]
fn read_expires_exactly_after_300s() {
    let fx = accepted_fixture();
    assert_eq!(
        try_update(&fx, &Bytes::from_slice(&fx.env, &FIXTURE[..])),
        Ok(())
    );
    let (xlm, usdc) = canonical_assets(&fx.env);
    ledger(
        &fx.env,
        FIXTURE_TS_S + 300,
        sha256(&fx.env, TESTNET_PASSPHRASE),
    );
    assert_eq!(oracle(&fx).lastprice(&xlm).unwrap().price, XLM_PRICE);
    ledger(
        &fx.env,
        FIXTURE_TS_S + 301,
        sha256(&fx.env, TESTNET_PASSPHRASE),
    );
    assert_eq!(oracle(&fx).lastprice(&xlm), None);
    // Clock-skew side of the read window, at the same exact boundary.
    ledger(
        &fx.env,
        FIXTURE_TS_S - 16,
        sha256(&fx.env, TESTNET_PASSPHRASE),
    );
    assert_eq!(oracle(&fx).lastprice(&usdc), None);
}

#[test]
fn unseeded_reads_fail_closed() {
    let fx = testnet();
    let oracle = oracle(&fx);
    let (xlm, _) = canonical_assets(&fx.env);
    assert_eq!(oracle.lastprice(&xlm), None);
    assert_eq!(oracle.prices(&xlm, &0), None);
    assert_eq!(oracle.prices(&xlm, &1), None);
    assert_eq!(oracle.price(&xlm, &FIXTURE_TS_S), None);
    assert_eq!(
        oracle.lastprice(&Asset::Stellar(Address::generate(&fx.env))),
        None
    );
    assert_eq!(oracle.lastprice(&crate::base_asset()), None);
}

// ---- pure policy: malformed updates ------------------------------------------

fn now_us() -> u64 {
    FIXTURE_TS_S * crate::US_PER_SEC
}

fn feed(id: u32, raw: Option<i64>, exponent: Option<i16>, feed_ts_us: Option<u64>) -> Feed {
    Feed {
        feed_id: id,
        price: raw,
        exponent,
        feed_update_timestamp: feed_ts_us,
        best_bid_price: None,
        best_ask_price: None,
        publisher_count: None,
        confidence: None,
        funding_rate: None,
        funding_timestamp: None,
        funding_rate_interval: None,
        market_session: None,
        ema_price: None,
        ema_confidence: None,
    }
}

fn update(header_ts_us: u64, channel: Channel, feeds: alloc::vec::Vec<Feed>) -> Update {
    Update {
        timestamp: header_ts_us,
        channel,
        feeds,
    }
}

fn feed_23() -> Feed {
    feed(23, Some(XLM_RAW), Some(-8), Some(FIXTURE_TS_US))
}
fn feed_7() -> Feed {
    feed(7, Some(USDC_RAW), Some(-8), Some(FIXTURE_TS_US))
}

fn validate(
    header_ts_us: u64,
    feeds: alloc::vec::Vec<Feed>,
) -> Result<[FeedUpdate; 2], LazerOracleError> {
    validate_update(
        &update(header_ts_us, Channel::FixedRate200ms, feeds),
        now_us(),
    )
}

#[test]
fn real_mantissas_scale_to_sep40_exactly() {
    let accepted = validate(FIXTURE_TS_US, alloc::vec![feed_23(), feed_7()]).expect("valid");
    let by_id = |id: u32| accepted.iter().find(|f| f.feed_id == id);
    assert_eq!(
        by_id(23).map(|f| (f.price, f.feed_ts_us)),
        Some((XLM_PRICE, FIXTURE_TS_US))
    );
    assert_eq!(
        by_id(7).map(|f| (f.price, f.feed_ts_us)),
        Some((USDC_PRICE, FIXTURE_TS_US))
    );
    // The floor is exact: an odd last mantissa digit drops, never rounds up.
    let floored = validate(
        FIXTURE_TS_US,
        alloc::vec![
            feed_23(),
            feed(7, Some(USDC_RAW + 1), Some(-8), Some(FIXTURE_TS_US))
        ],
    )
    .expect("valid");
    assert_eq!(
        floored.iter().find(|f| f.feed_id == 7).map(|f| f.price),
        Some(USDC_PRICE)
    );
    // The smallest positive SEP-40 price is the raw floor boundary.
    let smallest = validate(
        FIXTURE_TS_US,
        alloc::vec![feed(23, Some(10), Some(-8), Some(FIXTURE_TS_US)), feed_7()],
    )
    .expect("valid");
    assert_eq!(
        smallest.iter().find(|f| f.feed_id == 23).map(|f| f.price),
        Some(1)
    );
}

#[test]
fn wrong_channel_rejected() {
    assert_eq!(
        validate_update(
            &update(
                FIXTURE_TS_US,
                Channel::RealTime,
                alloc::vec![feed_23(), feed_7()]
            ),
            now_us(),
        )
        .err(),
        Some(LazerOracleError::WrongChannel)
    );
}

#[test]
fn canonical_composition_enforced() {
    assert_eq!(
        validate(FIXTURE_TS_US, alloc::vec![feed_23()]).err(),
        Some(LazerOracleError::MissingFeed)
    );
    assert_eq!(
        validate(FIXTURE_TS_US, alloc::vec![feed_23(), feed_23()]).err(),
        Some(LazerOracleError::DuplicateFeed)
    );
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![
                feed_23(),
                feed_7(),
                feed(1, Some(10), Some(-8), Some(FIXTURE_TS_US))
            ],
        )
        .err(),
        Some(LazerOracleError::ForeignFeed)
    );
}

#[test]
fn properties_and_prices_enforced() {
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![feed(23, Some(XLM_RAW), None, Some(FIXTURE_TS_US)), feed_7()],
        )
        .err(),
        Some(LazerOracleError::MissingProperty)
    );
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![
                feed(23, Some(XLM_RAW), Some(-7), Some(FIXTURE_TS_US)),
                feed_7()
            ],
        )
        .err(),
        Some(LazerOracleError::WrongExponent)
    );
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![feed(23, None, Some(-8), Some(FIXTURE_TS_US)), feed_7()],
        )
        .err(),
        Some(LazerOracleError::MissingProperty)
    );
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![feed(23, Some(-1), Some(-8), Some(FIXTURE_TS_US)), feed_7()],
        )
        .err(),
        Some(LazerOracleError::NonPositivePrice)
    );
    // raw 1..9 floors to 0 USD8: not a positive price.
    for raw in 1..10 {
        assert_eq!(
            validate(
                FIXTURE_TS_US,
                alloc::vec![feed(23, Some(raw), Some(-8), Some(FIXTURE_TS_US)), feed_7()],
            )
            .err(),
            Some(LazerOracleError::NonPositivePrice),
            "raw {raw} must not floor to a zero price"
        );
    }
}

#[test]
fn micro_freshness_boundaries() {
    let us = crate::US_PER_SEC;
    // Header: exact age/future boundaries, and zero is never valid.
    assert_eq!(check_age(now_us() - 60 * us, now_us()), Ok(()));
    assert_eq!(
        check_age(now_us() - 60 * us - 1, now_us()),
        Err(LazerOracleError::StaleUpdate)
    );
    assert_eq!(check_age(now_us() + 15 * us, now_us()), Ok(()));
    assert_eq!(
        check_age(now_us() + 15 * us + 1, now_us()),
        Err(LazerOracleError::UpdateTimestampInFuture)
    );
    assert_eq!(check_age(0, now_us()), Err(LazerOracleError::StaleUpdate));
    // Zero and stale *feeds* are refused on their own timestamps: a fresh
    // header cannot restamp an old feed.
    assert_eq!(
        validate(
            now_us(),
            alloc::vec![feed(23, Some(XLM_RAW), Some(-8), Some(0)), feed_7()],
        )
        .err(),
        Some(LazerOracleError::StaleUpdate)
    );
    assert_eq!(
        validate(
            now_us(),
            alloc::vec![
                feed(23, Some(XLM_RAW), Some(-8), Some(now_us() - 61 * us)),
                feed_7()
            ],
        )
        .err(),
        Some(LazerOracleError::StaleUpdate)
    );
}

#[test]
fn feed_may_predate_header_but_never_postdate_it() {
    let us = crate::US_PER_SEC;
    // Older feed than header is normal (carried-forward observation).
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![
                feed(23, Some(XLM_RAW), Some(-8), Some(FIXTURE_TS_US - 30 * us)),
                feed_7()
            ],
        )
        .map(|_| ()),
        Ok(())
    );
    // A feed newer than the signed header that carries it is impossible data.
    assert_eq!(
        validate(
            FIXTURE_TS_US,
            alloc::vec![
                feed(23, Some(XLM_RAW), Some(-8), Some(FIXTURE_TS_US + 1)),
                feed_7()
            ],
        )
        .err(),
        Some(LazerOracleError::UpdateTimestampInFuture)
    );
}

// ---- monotonicity (microsecond precision) ------------------------------------

fn feed_update(id: u32, price: i128, feed_ts_us: u64) -> FeedUpdate {
    FeedUpdate {
        feed_id: id,
        price,
        feed_ts_us,
    }
}

fn stored_price_of(price: i128, ts_us: u64) -> StoredPrice {
    StoredPrice {
        price,
        timestamp_us: ts_us,
    }
}

#[test]
fn monotonicity_orders_consumers_by_microseconds() {
    let current = stored_price_of(XLM_PRICE, FIXTURE_TS_US);
    let check = |feed: FeedUpdate| check_monotonic(Some(&current), &feed);
    // Older microseconds: refused even with the identical price.
    assert_eq!(
        check(feed_update(23, XLM_PRICE, FIXTURE_TS_US - 1)).err(),
        Some(LazerOracleError::StaleUpdate)
    );
    // Same microsecond, different price: conflicting observations.
    assert_eq!(
        check(feed_update(23, XLM_PRICE + 1, FIXTURE_TS_US)).err(),
        Some(LazerOracleError::SameTimestampConflict)
    );
    // Identical (timestamp, price): replay no-op.
    assert_eq!(check(feed_update(23, XLM_PRICE, FIXTURE_TS_US)), Ok(()));
    // Same calendar second but newer microseconds with a new price: accepted,
    // so consumers ordering strictly after the stored quote always apply.
    assert_eq!(
        check(feed_update(23, XLM_PRICE + 100, FIXTURE_TS_US + 100_000)),
        Ok(())
    );
}
