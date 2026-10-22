//! Standalone deterministic property runner for ADR0008 generated replay.
//! Depends only on pinned proptest and the relative Blend test-suites crate.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use test_suites::adr8_properties::replay_generated;

/// Default seed and case count are env-overridable via PROPTEST_RNG_SEED /
/// PROPTEST_CASES (seed and case count are parsed explicitly below).
const DEFAULT_SEED: u32 = 9110008;
const DEFAULT_CASES: u32 = 10000;
const MAX_OPS: usize = 64;
const MAX_USERS: u8 = 4;
const MAX_RESERVES: u8 = 3;

fn operations_strategy() -> impl Strategy<Value = Vec<(u8, u8, u8, u64, u32)>> {
    prop::collection::vec(
        (
            0u8..=18,
            0u8..=MAX_USERS - 1,
            0u8..=MAX_RESERVES - 1,
            0u64..=5,
            0u32..=8,
        ),
        1..=MAX_OPS,
    )
}

#[test]
fn property_adr8_invariants() {
    let seed_text = std::env::var("PROPTEST_RNG_SEED")
        .ok()
        .map(|s| s.parse::<u32>().expect("PROPTEST_RNG_SEED must be u32"))
        .unwrap_or(DEFAULT_SEED);
    let cases_text = std::env::var("PROPTEST_CASES")
        .ok()
        .map(|s| s.parse::<u32>().expect("PROPTEST_CASES must be u32"))
        .unwrap_or(DEFAULT_CASES);
    assert!(cases_text > 0, "PROPTEST_CASES must execute at least one case");
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&seed_text.to_le_bytes());
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &bytes);

    let config = Config {
        cases: cases_text,
        verbose: 2,
        rng_algorithm: RngAlgorithm::ChaCha,
        ..Config::default()
    };
    let mut runner = TestRunner::new_with_rng(config, rng);
    // With verbose=2 a failing run is minimized and the complete shrunk
    // operations tuple is printed verbatim; deterministic by the fixed ChaCha
    // seed regardless of case count.
    let result = runner.run(&operations_strategy(), |ops| {
        replay_generated(&ops);
        Ok(())
    });
    if let Err(failure) = &result {
        eprintln!(
            "FAIL adr8 seed={} configured-cases={}\nshrunk-witness:\n{}",
            seed_text, cases_text, failure
        );
    }
    assert!(result.is_ok(), "adr8 property failed; see shrunk witness above");
    eprintln!(
        "PASS adr8 mode=wasm-registered fixtures seed={} cases={} bounds ops<=64 users<=4 reserves<=3 amount-selectors<=5 time-selectors<=8",
        seed_text,
        cases_text
    );
}
