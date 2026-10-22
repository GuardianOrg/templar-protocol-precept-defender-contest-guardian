//! One side's activation sequence, mirroring
//! `blend-contracts-v2/test-suites/src/{test_fixture.rs,setup.rs}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context};
use serde_json::json;

use crate::manifest::{
    begin_market_pending, finish_market_pending, Contracts, MarketConfig, MarketCustodyRecord,
    MarketManifest, MarketMutationKind, MarketNetworkRecord, MarketPendingResult, MarketStepRecord,
    MarketTestnet, OracleStackManifest, PairConfig, PairManifest, ReserveOverride, ReserveRef,
    Sha256Hex, SharedWasm, SideManifest, SideSpec, Tok, Who, MARKET_NETWORK_LABEL,
    MARKET_SOURCE_COUNT, MARKET_TESTNET_PASSPHRASE,
};
use crate::manifest::{expand_home, MARKET_ARTIFACT_ROLES};
use crate::manifest::{MARKET_BASE, MARKET_SOURCE_RESOLUTION};
use crate::market::{custody_payload_digest, sha256_hex};
use crate::scenario::{req, Req, E6, E9, I128_MAX, SCALAR_7};
use crate::stellar::{
    AccountId, Capture, CmdOutput, ContractId, Executor, Invocation, Net, Policy, RealExecutor,
    Safe, Stellar, WasmHash, MARKET_CAPTURE_STDERR_CAP, MARKET_CAPTURE_STDOUT_CAP,
    MARKET_MUTATION_WALL,
};
use serde_json::Value;

use serde::{Deserialize, Serialize};
impl Executor for Box<dyn Executor> {
    fn run(&self, args: &[String]) -> anyhow::Result<CmdOutput> {
        (**self).run(args)
    }

    fn run_by(
        &self,
        args: &[String],
        start_by: std::time::SystemTime,
    ) -> anyhow::Result<CmdOutput> {
        (**self).run_by(args, start_by)
    }
}

pub struct SharedHashes {
    pub token: String,
    pub oracle: String,
    pub comet: String,
    pub emitter: String,
}

/// Deterministic 32-byte salt: nonce, role, side. Unique per (deployer, salt).
pub fn salt(nonce: u64, role: u32, side: char) -> String {
    format!(
        "{:016x}{:016x}{:016x}{:016x}",
        nonce, role as u64, side as u64, 0u64
    )
}

pub fn ensure_identities<E: Executor>(st: &Stellar<E>) -> anyhow::Result<BTreeMap<Who, String>> {
    let mut ids = BTreeMap::new();
    for who in Who::ALL {
        let addr = st.keys_ensure_funded(&who.key_name())?;
        ids.insert(who, addr.as_str().to_string());
    }
    Ok(ids)
}

pub fn upload_shared<E: Executor>(
    st: &Stellar<E>,
    w: &SharedWasm,
    admin: &str,
) -> anyhow::Result<SharedHashes> {
    Ok(SharedHashes {
        token: st.upload(&w.token_wasm, admin)?.as_str().to_string(),
        oracle: st.upload(&w.oracle_wasm, admin)?.as_str().to_string(),
        comet: st.upload(&w.comet_wasm, admin)?.as_str().to_string(),
        emitter: st.upload(&w.emitter_wasm, admin)?.as_str().to_string(),
    })
}

/// Fixture reserve configs (`test-suites/src/setup.rs`) with optional overrides.
pub fn reserve_config(tok: Tok, index: u32, ov: Option<&ReserveOverride>) -> serde_json::Value {
    let (decimals, c, l, util, cap) = match tok {
        Tok::Stable => (
            6,
            9_000_000,
            9_500_000,
            8_500_000,
            "1000000000000000000".to_string(),
        ),
        Tok::Xlm => (
            7,
            7_500_000,
            7_500_000,
            5_000_000,
            "1000000000000000000".to_string(),
        ),
        Tok::Weth => (9, 8_000_000, 8_000_000, 7_000_000, I128_MAX.to_string()),
        _ => (
            tok.decimals(),
            7_500_000,
            7_500_000,
            5_000_000,
            "1000000000000000000".to_string(),
        ),
    };
    let ov = ov.cloned().unwrap_or_default();
    json!({
        "index": index,
        "decimals": decimals,
        "c_factor": ov.c_factor.unwrap_or(c),
        "l_factor": ov.l_factor.unwrap_or(l),
        "util": ov.util.unwrap_or(util),
        "max_util": ov.max_util.unwrap_or(9_500_000),
        "r_base": ov.r_base.unwrap_or(100_000),
        "r_one": ov.r_one.unwrap_or(500_000),
        "r_two": ov.r_two.unwrap_or(5_000_000),
        "r_three": ov.r_three.unwrap_or(15_000_000),
        "reactivity": ov.reactivity.unwrap_or(20),
        "supply_cap": ov.supply_cap.unwrap_or(cap),
        "enabled": true
    })
}

pub fn request_json(reqs: &[Req], contracts: &Contracts) -> String {
    let v: Vec<_> = reqs
        .iter()
        .map(|r| json!({"request_type": r.kind, "address": contracts.token(r.asset), "amount": r.amount}))
        .collect();
    serde_json::to_string(&v).expect("json")
}

fn kv(k: &str, v: impl ToString) -> (&str, String) {
    (k, v.to_string())
}

/// Fixture prices in oracle order, matching the pair engine's published
/// set and the scenario fixtures: weth 2000, usdc 1, xlm 0.1, stable 1
/// (base units, 7 decimals).
pub fn fixture_price(tok: Tok) -> i128 {
    match tok {
        Tok::Weth => 2_000 * SCALAR_7,
        Tok::Usdc => SCALAR_7,
        Tok::Xlm => E6,
        Tok::Stable => SCALAR_7,
        _ => SCALAR_7,
    }
}

/// An i128 quantity from a CLI-rendered value: JSON numbers or strings,
/// which is how the RPC surface encodes 128-bit integers.
fn price_as_i128(v: &Value) -> Option<i128> {
    if let Some(n) = v.as_u64() {
        return Some(i128::from(n));
    }
    if let Some(n) = v.as_i64() {
        return Some(n as i128);
    }
    let raw = v.as_str()?;
    if let Some(hex) = raw.strip_prefix("0x") {
        return i128::from_str_radix(hex, 16).ok();
    }
    raw.parse::<i128>().ok()
}

/// Stored hex digest -> checksum-validated hash newtype. Strictly 32 bytes of
/// lowercase hex; malformed or oversized digests fail closed rather than
/// truncate, and no raw-string path reaches `deploy`.
fn hash_of(hex: &str) -> anyhow::Result<WasmHash> {
    hex.parse::<WasmHash>().map_err(|_| {
        anyhow::anyhow!(
            "{}",
            Safe::literal("stored wasm digest is not a valid sha256 hex hash")
        )
    })
}
fn must_ok(
    inv: crate::stellar::Invocation,
    what: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    anyhow::ensure!(inv.ok, "{}", Safe::pair_must_ok(&inv, what));
    Ok(inv.value)
}
// ---------------------------------------------------------------------------
// In-tree keccak256 (original Keccak padding, NOT NIST SHA3) and the
// factory-child contract-id prediction it feeds. The pool factory derives a
// child salt as keccak256(salt || admin-strkey padded to 56 bytes)
// (blend-contracts-v2 pool_factory.rs:92-96) and deploys at
// sha256(XDR(HashIdPreimage::ContractId)) with itself as sender
// (pool_factory.rs:98-109). The stellar CLI exposes no preimage prediction,
// so this host-side law is the only prediction path for factory children.
// Every function below is gated by known-answer tests pinned to the offline
// pool-proof harness (blend-contracts-v2/pool-proof), which proves the law
// against the real factory wasm.

/// Keccak-f[1600] round constants.
const KECCAK_RC: [u64; 24] = [
    0x0000_0000_0000_0001,
    0x0000_0000_0000_8082,
    0x8000_0000_0000_808a,
    0x8000_0000_8000_8000,
    0x0000_0000_0000_808b,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8009,
    0x0000_0000_0000_008a,
    0x0000_0000_0000_0088,
    0x0000_0000_8000_8009,
    0x0000_0000_8000_000a,
    0x0000_0000_8000_808b,
    0x8000_0000_0000_008b,
    0x8000_0000_0000_8089,
    0x8000_0000_0000_8003,
    0x8000_0000_0000_8002,
    0x8000_0000_0000_0080,
    0x0000_0000_0000_800a,
    0x8000_0000_8000_000a,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8080,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8008,
];
/// Rho rotation offsets, paired with the Pi lane map below.
const KECCAK_ROT: [u32; 24] = [
    1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
];
/// Pi lane permutation, index-matched to `KECCAK_ROT`.
const KECCAK_PI: [usize; 24] = [
    10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
];

/// One Keccak-f[1600] permutation over the 5x5 lane state (`a[x + 5*y]`).
fn keccak_f1600(a: &mut [u64; 25]) {
    for &rc in KECCAK_RC.iter() {
        // theta
        let mut c = [0u64; 5];
        for (x, cx) in c.iter_mut().enumerate() {
            *cx = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        let mut d = [0u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] ^= d[x];
            }
        }
        // rho + pi
        let mut last = a[1];
        for i in 0..24 {
            let tmp = a[KECCAK_PI[i]];
            a[KECCAK_PI[i]] = last.rotate_left(KECCAK_ROT[i]);
            last = tmp;
        }
        // chi
        for y in 0..5 {
            let row = [
                a[5 * y],
                a[5 * y + 1],
                a[5 * y + 2],
                a[5 * y + 3],
                a[5 * y + 4],
            ];
            for x in 0..5 {
                a[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
            }
        }
        // iota
        a[0] ^= rc;
    }
}

/// keccak256 with the ORIGINAL Keccak padding (0x01 ... 0x80), which is what
/// the pool factory's `e.crypto().keccak256` applies — not NIST SHA3's
/// 0x06 domain byte. Known-answer tests below pin both properties.
pub(crate) fn keccak256(data: &[u8]) -> [u8; 32] {
    const RATE: usize = 136; // 1088-bit rate for a 256-bit digest
    let mut state = [0u64; 25];
    let absorb = |block: &[u8], state: &mut [u64; 25]| {
        for (i, lane) in block.chunks_exact(8).enumerate() {
            state[i] ^= u64::from_le_bytes(lane.try_into().expect("lane"));
        }
        keccak_f1600(state);
    };
    let mut blocks = data.chunks_exact(RATE);
    for block in blocks.by_ref() {
        absorb(block, &mut state);
    }
    // The remainder comes from the SAME (now exhausted) iterator, so the
    // tail is the true tail — the same trap the sha256 896-bit KAT guards.
    let rem = blocks.remainder();
    let mut block = [0u8; RATE];
    block[..rem.len()].copy_from_slice(rem);
    block[rem.len()] ^= 0x01;
    block[RATE - 1] ^= 0x80;
    absorb(&block, &mut state);
    let mut out = [0u8; 32];
    for (i, lane) in state.iter().take(4).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&lane.to_le_bytes());
    }
    out
}

#[cfg(test)]
const STRKEY_G: u8 = 0x30; // ed25519 account
const STRKEY_C: u8 = 2 << 3; // contract (first character must encode 'C')
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, no padding (the strkey encoding).
fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 8 + 4) / 5);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &b in data {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32_ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
            // Emitted quintets must be cleared from the window, or the next
            // extraction re-reads them.
            buffer &= if bits >= 32 {
                u32::MAX
            } else {
                (1u32 << bits) - 1
            };
        }
    }
    if bits > 0 {
        buffer &= if bits >= 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        out.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// CRC-16/XMODEM (poly 0x1021, init 0), the strkey checksum.
fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Strkey encode: version byte, payload, CRC-16 checksum appended low-byte
/// first (little-endian), base32.
fn strkey_encode(version: u8, payload: &[u8]) -> String {
    let mut data = Vec::with_capacity(payload.len() + 3);
    data.push(version);
    data.extend_from_slice(payload);
    // The strkey checksum is appended LOW BYTE FIRST (little-endian).
    data.extend_from_slice(&crc16_xmodem(&data).to_le_bytes());
    base32_encode(&data)
}

/// Strkey decode with checksum validation: (version, payload) or `None`.
fn strkey_decode(s: &str) -> Option<(u8, Vec<u8>)> {
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut data = Vec::with_capacity(s.len() * 5 / 8);
    for ch in s.bytes() {
        let v = BASE32_ALPHABET.iter().position(|&a| a == ch)? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            data.push(((buffer >> bits) & 0xff) as u8);
            // Same law as the encoder: emitted bytes are cleared from the
            // window before the next extraction.
            buffer &= if bits >= 32 {
                u32::MAX
            } else {
                (1u32 << bits) - 1
            };
        }
    }
    if bits > 0
        && (buffer
            & if bits >= 32 {
                u32::MAX
            } else {
                (1u32 << bits) - 1
            })
            != 0
    {
        return None; // a valid strkey carries no stray trailing bits
    }
    if data.len() < 3 {
        return None;
    }
    let (body, crc) = data.split_at(data.len() - 2);
    let expect = u16::from_le_bytes([*crc.first()?, *crc.get(1)?]);
    (crc16_xmodem(body) == expect).then(|| (body[0], body[1..].to_vec()))
}

/// The market's network id: sha256 of the frozen testnet passphrase. This is
/// the `network_id` the chain mixes into every HashIdPreimage.
fn market_network_id() -> [u8; 32] {
    hex32(&sha256_hex(MARKET_TESTNET_PASSPHRASE.as_bytes())).expect("sha256 hex")
}

/// 32 bytes from exactly 64 lowercase hex characters; fail closed otherwise.
fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Host-side `sha256(XDR(HashIdPreimage::ContractId))` for a factory
/// sender. XDR layout (stellar-xdr 26.0.0, proven by pool-proof): union
/// discriminant 0x00000008 (ContractId), network id, inner-union arm
/// 0x00000000 (ContractIdPreimage::Address), ScAddress discriminant
/// 0x00000001 (Contract), sender, salt. Account-branch deploy prediction
/// lives in the CLI (`contract id wasm`), whose salt input is
/// sha256(wasm-hash || salt); this helper serves only factory children,
/// whose salt the factory itself derives.
fn predict_contract_child(
    network_id: &[u8; 32],
    factory: &[u8; 32],
    child_salt: &[u8; 32],
) -> [u8; 32] {
    let mut preimage = Vec::with_capacity(108);
    preimage.extend_from_slice(&8u32.to_be_bytes());
    preimage.extend_from_slice(network_id);
    preimage.extend_from_slice(&0u32.to_be_bytes());
    preimage.extend_from_slice(&1u32.to_be_bytes());
    preimage.extend_from_slice(factory);
    preimage.extend_from_slice(child_salt);
    hex32(&sha256_hex(&preimage)).expect("sha256 hex")
}

/// The pool factory's child salt: keccak256(deploy salt || admin strkey
/// bytes copied into a zero-padded [u8;56]) (pool_factory.rs:92-96). The
/// raw role-235 salt never deploys anything by itself and must never be
/// bound as a prediction input elsewhere in this preimage.
fn pool_factory_child_salt(salt_hex: &str, admin_strkey: &str) -> Option<[u8; 32]> {
    let salt = hex32(salt_hex)?;
    let admin = admin_strkey.as_bytes();
    if admin.len() > 56 {
        return None; // the factory itself copies into a fixed [u8;56]
    }
    let mut preimage = [0u8; 88];
    preimage[..32].copy_from_slice(&salt);
    preimage[32..32 + admin.len()].copy_from_slice(admin);
    Some(keccak256(&preimage))
}

/// Predict the pool a factory `deploy` will return, offline: derive the
/// child salt, then the XDR child id, then render the contract strkey.
fn predict_pool_child(
    factory_strkey: &str,
    salt_hex: &str,
    admin_strkey: &str,
) -> anyhow::Result<String> {
    let (_version, factory) = strkey_decode(factory_strkey)
        .filter(|(v, p)| *v == STRKEY_C && p.len() == 32)
        .ok_or_else(|| anyhow!("the settled factory id is not a contract strkey"))?;
    let factory: [u8; 32] = factory.try_into().expect("len checked");
    let child = pool_factory_child_salt(salt_hex, admin_strkey)
        .ok_or_else(|| anyhow!("the pool salt or operator strkey is not derivable"))?;
    let id = predict_contract_child(&market_network_id(), &factory, &child);
    Ok(strkey_encode(STRKEY_C, &id))
}

pub fn deploy_side<E: Executor>(
    st: &Stellar<E>,
    spec: &SideSpec,
    side: char,
    nonce: u64,
    ids: &BTreeMap<Who, String>,
    shared: &SharedHashes,
) -> anyhow::Result<SideManifest> {
    let admin_key = Who::Admin.key_name();
    let frodo_key = Who::Frodo.key_name();
    let admin = ids[&Who::Admin].as_str();
    let frodo = ids[&Who::Frodo].as_str();
    let deploy_ledger = st.latest_ledger()?;
    let mut wasm = BTreeMap::new();
    let mut c = Contracts::default();
    let mut minted: BTreeMap<Tok, i64> = BTreeMap::new();

    // 1. side-specific code
    let pool_hash = st.upload(&spec.pool_wasm, &admin_key)?;
    let backstop_hash = st.upload(&spec.backstop_wasm, &admin_key)?;
    let factory_hash = st.upload(&spec.factory_wasm, &admin_key)?;
    wasm.insert("pool".into(), pool_hash.as_str().to_string());
    wasm.insert("backstop".into(), backstop_hash.as_str().to_string());
    wasm.insert("pool_factory".into(), factory_hash.as_str().to_string());
    wasm.insert("token".into(), shared.token.clone());
    wasm.insert("oracle".into(), shared.oracle.clone());
    wasm.insert("comet".into(), shared.comet.clone());
    wasm.insert("emitter".into(), shared.emitter.clone());

    // 2. mock tokens
    for (i, tok) in Tok::MOCK.iter().enumerate() {
        let id = st.deploy(
            &hash_of(&shared.token)?,
            &admin_key,
            Some(&salt(nonce, 10 + i as u32, side)),
            &[],
        )?;
        let sym = tok.as_str().to_uppercase();
        must_ok(
            st.invoke(
                id.as_str(),
                &admin_key,
                "initialize",
                &[
                    kv("admin", admin),
                    kv("decimal", tok.decimals()),
                    kv("name", &sym),
                    kv("symbol", &sym),
                ],
            )?,
            "token initialize",
        )?;
        c.set_token(*tok, id.as_str().to_string());
    }

    // 3. Comet LP seeded by admin
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Blnd,
        admin,
        1_000 * SCALAR_7,
    )?;
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Usdc,
        admin,
        25 * SCALAR_7,
    )?;
    let lp = st.deploy(
        &hash_of(&shared.comet)?,
        &admin_key,
        Some(&salt(nonce, 20, side)),
        &[],
    )?;
    must_ok(
        st.invoke(
            lp.as_str(),
            &admin_key,
            "init",
            &[
                kv("controller", admin),
                kv("tokens", json!([c.blnd, c.usdc])),
                kv("weights", json!(["8000000", "2000000"])),
                kv(
                    "balances",
                    json!([(1_000 * SCALAR_7).to_string(), (25 * SCALAR_7).to_string()]),
                ),
                kv("swap_fee", 30_000),
            ],
        )?,
        "comet init",
    )?;
    c.lp = lp.as_str().to_string();

    // 4. emitter (constructor metadata only; never called)
    c.emitter = st
        .deploy(
            &hash_of(&shared.emitter)?,
            &admin_key,
            Some(&salt(nonce, 30, side)),
            &[],
        )?
        .as_str()
        .to_string();

    // 5. backstop <-> factory cycle via predicted id
    let factory_salt = salt(nonce, 40, side);
    let predicted_factory = st.predict_id(&admin_key, &factory_salt)?;
    let drop_list = json!([
        [admin, (10_000_000 * SCALAR_7).to_string()],
        [frodo, (30_000_000 * SCALAR_7).to_string()]
    ]);
    c.backstop = st
        .deploy(
            &backstop_hash,
            &admin_key,
            Some(&salt(nonce, 41, side)),
            &[
                kv("backstop_token", &c.lp),
                kv("emitter", &c.emitter),
                kv("blnd_token", &c.blnd),
                kv("usdc_token", &c.usdc),
                kv("pool_factory", predicted_factory.as_str()),
                kv("drop_list", drop_list),
            ],
        )?
        .as_str()
        .to_string();
    let meta = json!({
        "backstop": c.backstop,
        "pool_hash": pool_hash.as_str(),
        "blnd_id": c.blnd
    });
    c.pool_factory = st
        .deploy(
            &factory_hash,
            &admin_key,
            Some(&factory_salt),
            &[kv("pool_init_meta", meta)],
        )?
        .as_str()
        .to_string();
    anyhow::ensure!(
        c.pool_factory == predicted_factory.as_str(),
        "factory id {} != predicted {}",
        c.pool_factory,
        predicted_factory.as_str()
    );

    // 6. oracle
    c.oracle = st
        .deploy(
            &hash_of(&shared.oracle)?,
            &admin_key,
            Some(&salt(nonce, 50, side)),
            &[],
        )?
        .as_str()
        .to_string();
    let assets: Vec<_> = Tok::ORACLE_ORDER
        .iter()
        .map(|t| json!({"Stellar": c.token(*t)}))
        .collect();
    must_ok(
        st.invoke(
            &c.oracle,
            &admin_key,
            "set_data",
            &[
                kv("admin", admin),
                kv("base", json!({"Other": "USD"})),
                kv("assets", json!(assets)),
                kv("decimals", 7),
                kv("resolution", 300),
            ],
        )?,
        "oracle set_data",
    )?;
    must_ok(
        set_prices(st, &c, &[2_000 * SCALAR_7, SCALAR_7, 1_000_000, SCALAR_7])?,
        "deploy set_price_stable",
    )?;

    // 7. whale funding
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Blnd,
        frodo,
        100_000_000 * SCALAR_7,
    )?;
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Usdc,
        frodo,
        2_600_000 * SCALAR_7,
    )?;
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Stable,
        frodo,
        100_000 * E6,
    )?;
    mint_tok(
        st,
        &admin_key,
        &c,
        &mut minted,
        Tok::Xlm,
        frodo,
        1_000_000 * SCALAR_7,
    )?;
    mint_tok(st, &admin_key, &c, &mut minted, Tok::Weth, frodo, 100 * E9)?;

    // 8. Comet seeding
    must_ok(
        st.invoke(
            &c.lp,
            &frodo_key,
            "join_pool",
            &[
                kv("pool_amount_out", 10_000_000 * SCALAR_7),
                kv(
                    "max_amounts_in",
                    json!([
                        (110_000_000 * SCALAR_7).to_string(),
                        (2_600_000 * SCALAR_7).to_string()
                    ]),
                ),
                kv("user", frodo),
            ],
        )?,
        "comet join_pool",
    )?;

    // 9. pool
    let pool_val = must_ok(
        st.invoke(
            &c.pool_factory,
            &admin_key,
            "deploy",
            &[
                kv("admin", admin),
                kv("name", "Teapot"),
                kv("salt", salt(nonce, 60, side)),
                kv("oracle", &c.oracle),
                // Fixture divergence: upstream setup.rs passes 1_000_000. A zero take
                // rate keeps interest with suppliers so A/B deltas attribute to the
                // fork's logic, not backstop skimming; identical on both sides.
                kv("backstop_take_rate", 0),
                kv("max_positions", 6),
                kv("min_collateral", SCALAR_7),
            ],
        )?,
        "factory deploy",
    )?;
    c.pool = pool_val
        .and_then(|v| v.as_str().map(str::to_string))
        .context("factory deploy returned no address")?;

    // 10. reserves
    let mut reserves = BTreeMap::new();
    for (i, tok) in Tok::RESERVES.iter().enumerate() {
        let cfg = reserve_config(*tok, i as u32, spec.reserves.get(tok));
        must_ok(
            st.invoke(
                &c.pool,
                &admin_key,
                "queue_set_reserve",
                &[kv("asset", c.token(*tok)), kv("metadata", cfg)],
            )?,
            "queue_set_reserve",
        )?;
        let idx = must_ok(
            st.invoke(
                &c.pool,
                &admin_key,
                "set_reserve",
                &[kv("asset", c.token(*tok))],
            )?,
            "set_reserve",
        )?;
        let index = idx
            .and_then(|v| v.as_u64())
            .context("set_reserve returned no index")? as u32;
        anyhow::ensure!(
            index == i as u32,
            "reserve {tok:?} got index {index}, expected {i}"
        );
        reserves.insert(
            *tok,
            ReserveRef {
                index,
                decimals: tok.decimals(),
            },
        );
    }

    // 11. backstop deposit and activation
    must_ok(
        st.invoke(
            &c.backstop,
            &frodo_key,
            "deposit",
            &[
                kv("from", frodo),
                kv("pool_address", &c.pool),
                kv("amount", 50_000 * SCALAR_7),
            ],
        )?,
        "backstop deposit",
    )?;
    must_ok(
        st.invoke(&c.pool, &admin_key, "set_status", &[kv("pool_status", 3)])?,
        "set_status(3)",
    )?;
    let status = must_ok(
        st.invoke(&c.pool, &admin_key, "update_status", &[])?,
        "update_status",
    )?;
    anyhow::ensure!(
        status.as_ref().and_then(|v| v.as_u64()) == Some(1),
        "update_status returned {status:?}, expected 1 (backstop threshold not met?)"
    );

    // 12. seed positions (setup.rs:85-130)
    let seed = |asset: Tok, supply: i128, borrow: i128| -> Vec<Req> {
        vec![
            Req::new(req::SUPPLY_COLLATERAL, asset, supply),
            Req::new(req::BORROW, asset, borrow),
        ]
    };
    for reqs in [
        seed(Tok::Stable, 10_000 * E6, 8_000 * E6),
        seed(Tok::Weth, 10 * E9, 5 * E9),
        seed(Tok::Xlm, 100_000 * SCALAR_7, 65_000 * SCALAR_7),
    ] {
        must_ok(
            st.invoke(
                &c.pool,
                &frodo_key,
                "submit",
                &[
                    kv("from", frodo),
                    kv("spender", frodo),
                    kv("to", frodo),
                    kv("requests", request_json(&reqs, &c)),
                ],
            )?,
            "seed submit",
        )?;
    }

    Ok(SideManifest {
        label: spec.label.clone(),
        deploy_ledger,
        wasm,
        contracts: c,
        reserves,
        minted,
    })
}

fn mint_tok<E: Executor>(
    st: &Stellar<E>,
    admin_key: &str,
    c: &Contracts,
    minted: &mut BTreeMap<Tok, i64>,
    tok: Tok,
    to: &str,
    amount: i128,
) -> anyhow::Result<()> {
    must_ok(
        st.invoke(
            c.token(tok),
            admin_key,
            "mint",
            &[kv("to", to), kv("amount", amount)],
        )?,
        "mint",
    )?;
    *minted.entry(tok).or_insert(0) += i64::try_from(amount).context("mint amount exceeds i64")?;
    Ok(())
}

pub fn set_prices<E: Executor>(
    st: &Stellar<E>,
    c: &Contracts,
    prices: &[i128],
) -> anyhow::Result<Invocation> {
    let p: Vec<String> = prices.iter().map(|x| x.to_string()).collect();
    let inv = st.invoke(
        &c.oracle,
        &Who::Admin.key_name(),
        "set_price_stable",
        &[kv("prices", json!(p))],
    )?;
    Ok(inv)
}

pub fn deploy_pair<E: Executor>(
    st: &Stellar<E>,
    cfg: &PairConfig,
    nonce: u64,
) -> anyhow::Result<PairManifest> {
    let ids = ensure_identities(st)?;
    let shared = upload_shared(st, &cfg.shared, &Who::Admin.key_name())?;
    let a = deploy_side(st, &cfg.a, 'a', nonce, &ids, &shared)?;
    let b = deploy_side(st, &cfg.b, 'b', nonce, &ids, &shared)?;
    Ok(PairManifest {
        profile: cfg.profile,
        description: cfg.description.clone(),
        net: st.net.clone(),
        identities: ids,
        a,
        b,
    })
}

// ===================== Market deployment engine (plan §4) =====================

/// Non-serializable runtime context for one market run: the exact-testnet
/// RPC endpoint with its request headers, the external admin identity and
/// its explicit config directory, the run-local actor identities and their
/// config directory, the pinned CLI binary, and the isolated HOME/xdg paths.
/// It is deliberately not `Serialize`: no serializable market record can
/// carry a signer alias, config path, secret key, or seed phrase.
pub struct MarketExecutionContext {
    rpc_url: String,
    rpc_headers: Vec<(String, String)>,
    admin_alias: String,
    admin_config_dir: PathBuf,
    actor_config_dir: PathBuf,
    stellar_bin: PathBuf,
    xdg: PathBuf,
    cwd: PathBuf,
}

impl MarketExecutionContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc_url: String,
        rpc_headers: Vec<(String, String)>,
        admin_alias: String,
        admin_config_dir: PathBuf,
        actor_config_dir: PathBuf,
        stellar_bin: PathBuf,
        xdg: PathBuf,
        cwd: PathBuf,
    ) -> anyhow::Result<Self> {
        let alias = admin_alias.trim();
        anyhow::ensure!(!alias.is_empty(), "admin identity alias is empty");
        anyhow::ensure!(
            !alias
                .chars()
                .any(|c| c.is_whitespace() || c == '/' || c == '\\'),
            "admin identity alias must be a plain CLI name, not a path or a secret"
        );
        anyhow::ensure!(
            AccountId::from_str(alias).is_err(),
            "admin identity alias must not be a StrKey; give the CLI alias, not a public key"
        );
        anyhow::ensure!(
            !(admin_alias.len() == 56 && matches!(admin_alias.chars().next(), Some('S' | 'M'))),
            "admin identity alias looks like a private key or muxed account"
        );
        anyhow::ensure!(!rpc_url.trim().is_empty(), "market RPC URL is empty");
        // SEC001 residual: a manually constructed market context can never
        // smuggle a run-directory cwd or relative executable into the CLI
        // executor. The market path runs from the controlled root with a
        // previously resolved absolute CLI path; PathBuf execution needs no
        // shell quoting even when a legitimate path contains spaces.
        anyhow::ensure!(
            cwd == std::path::Path::new("/"),
            "market executor cwd must be the controlled root, not a run or launch directory"
        );
        anyhow::ensure!(
            stellar_bin.is_absolute() && !stellar_bin.as_os_str().is_empty(),
            "market CLI binary path must be absolute"
        );
        let ctx = Self {
            rpc_url: rpc_url.trim().to_string(),
            rpc_headers,
            admin_alias: admin_alias.trim().to_string(),
            admin_config_dir,
            actor_config_dir,
            stellar_bin,
            xdg,
            cwd,
        };
        ctx.assert_testnet()?;
        Ok(ctx)
    }

    pub fn assert_testnet(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            MARKET_TESTNET_PASSPHRASE == "Test SDF Network ; September 2015"
                && MARKET_NETWORK_LABEL == "testnet",
            "market mode requires the exact Stellar testnet invariant"
        );
        Ok(())
    }

    pub fn stellar_bin(&self) -> &Path {
        &self.stellar_bin
    }
    pub fn xdg(&self) -> &Path {
        &self.xdg
    }
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }
    pub fn rpc_headers(&self) -> &[(String, String)] {
        &self.rpc_headers
    }
    pub fn actor_config_dir(&self) -> &Path {
        &self.actor_config_dir
    }
    pub fn admin_config_dir(&self) -> &Path {
        &self.admin_config_dir
    }
    pub fn admin_alias(&self) -> &str {
        &self.admin_alias
    }

    fn admin_net(&self) -> Net {
        Net {
            rpc_url: self.rpc_url.clone(),
            passphrase: MARKET_TESTNET_PASSPHRASE.to_string(),
            config_dir: self.admin_config_dir.clone(),
            headers: self.rpc_headers.clone(),
        }
    }

    fn actor_net(&self) -> Net {
        Net {
            rpc_url: self.rpc_url.clone(),
            passphrase: MARKET_TESTNET_PASSPHRASE.to_string(),
            config_dir: self.actor_config_dir.clone(),
            headers: self.rpc_headers.clone(),
        }
    }

    fn rpc_host_label(&self) -> String {
        let rest = self
            .rpc_url
            .split_once("://")
            .map(|(_, r)| r)
            .unwrap_or(&self.rpc_url);
        let authority = rest.split('/').next().unwrap_or("");
        authority.rsplit('@').next().unwrap_or("").to_string()
    }
}

/// One run's market journal: the numbered phase/slot custody journal under
/// the run directory. Addressed exclusively by numeric coordinates — the
/// name is never derived from record text, so no stored or supplied string
/// can steer a journal write outside `market-pending/`.
#[derive(Clone)]
pub struct MarketJournal {
    out: PathBuf,
}

impl MarketJournal {
    pub fn open(out: &Path) -> Self {
        Self {
            out: out.to_path_buf(),
        }
    }

    pub fn out(&self) -> PathBuf {
        self.out.clone()
    }

    pub fn begin(&self, phase: u32, slot: u32, record: &MarketCustodyRecord) -> anyhow::Result<()> {
        begin_market_pending(&self.out, phase, slot, record)
    }

    pub fn finish(
        &self,
        phase: u32,
        slot: u32,
        result: &MarketPendingResult,
    ) -> anyhow::Result<()> {
        finish_market_pending(&self.out, phase, slot, result)
    }

    pub fn read_intent(
        &self,
        phase: u32,
        slot: u32,
    ) -> anyhow::Result<Option<MarketCustodyRecord>> {
        // The pinned status reader is the sole path to journal records: an
        // absent coordinate is unreserved, while a malformed, relaxed-
        // network, or settlement-without-intent coordinate stays fail-closed.
        Ok(
            crate::market_status::read_journal_coordinate(&self.out, phase, slot)?
                .map(|(intent, _)| intent),
        )
    }

    /// Strictly verified settlement for one numbered coordinate, if any.
    /// `Ok(None)` means the coordinate is unreserved or holds an open
    /// intent only; a standalone, malformed, mismatched, or symlinked
    /// settlement is an error and can never read as a completed slot.
    pub fn result(&self, phase: u32, slot: u32) -> anyhow::Result<Option<MarketPendingResult>> {
        Ok(
            crate::market_status::read_journal_coordinate(&self.out, phase, slot)?
                .and_then(|(_, result)| result),
        )
    }
}

/// Operator commands to the deployment state machine. Headless and TUI
/// callers share this grammar; only the presentation differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeployAct {
    Init,
    Continue,
    Confirm { token: String },
    Halt,
}

/// The deployment event grammar. Every value-moving mutation is gated: the
/// engine emits `Confirm` and blocks until `DeployAct::Confirm` carries the
/// matching token; `Continue` cannot satisfy it. `Custody { unresolved:
/// true, .. }` is terminal: the engine replays nothing and stops.
#[derive(Clone, Debug)]
pub enum DeployEvent {
    Phase { id: usize, label: Safe },
    Confirm { token: String, summary: Safe },
    Custody { unresolved: bool, note: Safe },
    Done(MarketManifest),
    Failed(Safe),
}

/// Market deployment state machine. Owns the frozen configuration, the
/// non-serializable execution context, and the numbered custody journal.
/// Every value-moving mutation is gated: the engine blocks until the
/// operator's confirmation token matches, never retries an ambiguous
/// submission, and stops on unresolved custody.
pub struct MarketDeployer {
    cfg: MarketConfig,
    ctx: MarketExecutionContext,
    journal: MarketJournal,
    exec: RealExecutor,
    halted: bool,
    unresolved: Option<Safe>,
    gate: Option<Gate>,
    progress: Progress,
    manifest: Option<MarketManifest>,
    terminal: Option<DeployEvent>,
    plan_written: bool,
    /// The deployment nonce the salt table is keyed to. `0` means no plan
    /// has been bound, and with it no authorized mutation.
    nonce: u64,
    /// Re-derived artifact digests, role to SHA-256, proven at preflight.
    artifacts: BTreeMap<String, Sha256Hex>,
    /// Contracts this run has deployed, by role. A deployment is recorded
    /// only when its returned id matched its predicted id exactly.
    deployed: BTreeMap<String, ContractId>,
}

struct Gate {
    phase: u32,
    slot: u32,
    token: String,
    record: MarketCustodyRecord,
    summary: Safe,
}

#[derive(Default)]
struct Progress {
    admin: Option<AccountId>,
    identities: BTreeMap<Who, AccountId>,
    phase_done: std::collections::BTreeSet<u32>,
}

/// Outcome of one resume-law reconciliation pass. `Settled` is returned only
/// when the kind's declared public proof uniquely attributes the effect;
/// anything else is `Unresolved` and the run is abandoned until reconciled.
enum Reconciled {
    Settled,
    Unresolved(Safe),
}

/// What read-only preflight establishes. Nothing here is authorized to
/// mutate: it is the fact set a run must prove before a plan can bind.
struct Preflight {
    admin: AccountId,
    artifacts: BTreeMap<String, Sha256Hex>,
    nonce: u64,
}

/// The canonical SHA-256 of the parsed configuration. Two configurations
/// that parse to different values — including one whose artifact digests or
/// quorum shape were edited — produce different digests, so a plan frozen
/// against one cannot be honored by the other.
fn market_config_digest(cfg: &MarketConfig) -> anyhow::Result<String> {
    let text = toml::to_string_pretty(cfg).context("canonicalizing market config")?;
    Ok(sha256_hex(text.as_bytes()))
}

/// The deployment nonce, derived from the configuration digest and this run
/// directory. It is deterministic so a resumed run re-derives the same salt
/// table and the same predicted addresses rather than inventing new ones, and
/// it is unique to this configuration and directory so two different markets
/// never share a salt.
fn market_run_nonce(cfg: &MarketConfig, out: &Path) -> anyhow::Result<u64> {
    let digest = market_config_digest(cfg)?;
    let mut preimage = b"abtest-market-nonce-v1".to_vec();
    preimage.extend_from_slice(digest.as_bytes());
    preimage.extend_from_slice(out.to_string_lossy().as_bytes());
    let hash = sha256_hex(&preimage);
    let mut word = [0u8; 8];
    for (i, b) in word.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("the derived deployment nonce is not hexadecimal"))?;
    }
    Ok(u64::from_be_bytes(word))
}

// ============ Market deployment graph: schedule, salts, frozen plan ============

/// The market graph's salt side. The pair graph uses `'a'`/`'b'`, so no
/// market salt can ever collide with a pair salt.
pub(crate) const MARKET_SIDE: char = 'm';

/// The journal coordinate range the pre-command scan inspects. A coordinate
/// outside it would be invisible to the scan, so an ambiguous submission
/// there could never surface as unresolved custody and the no-replay law
/// would silently break. Every scheduled slot is checked against this range
/// at construction, so a slot added past the scanned range fails at startup.
pub(crate) const MARKET_SCANNED_PHASES: u32 = 12;
pub(crate) const MARKET_SCANNED_SLOTS: u32 = 40;

/// The frozen deployment plan's filename, inside the run directory.
pub(crate) const MARKET_PLAN_FILE: &str = "deployment-plan.toml";
/// The completed run's manifest filename, inside the run directory.
pub(crate) const MARKET_MANIFEST_FILE: &str = "market-manifest.toml";

/// One mutation of the protected-market graph, with its journal coordinate.
#[derive(Clone, Debug)]
pub(crate) struct MSlot {
    pub phase: u32,
    pub slot: u32,
    pub action: &'static str,
    pub kind: MarketMutationKind,
    pub op: MOp,
}

/// One operation of the 12-phase protected-market graph (plan §4). The graph
/// is data: the schedule, its salt table, and its journal coordinates can be
/// proven consistent without executing anything, and the vector's order is
/// the order the engine runs operations in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MOp {
    FundKey {
        who: Who,
    },
    Upload {
        role: &'static str,
    },
    DeployToken {
        tok: Tok,
    },
    InitToken {
        tok: Tok,
    },
    DeploySource {
        index: usize,
    },
    InitSource {
        index: usize,
    },
    SourcePrice {
        index: usize,
    },
    DeployGovernance,
    DeployRuntime,
    /// One governance proposal. The proposal content is built at execution
    /// time from the settled phase-3/4 deployments; the operation the
    /// proposal asks governance to perform is never static data, because it
    /// must bind the concrete deployed source and asset addresses.
    Propose {
        tok: Tok,
    },
    ExecuteProposal {
        id: u64,
    },
    Refresh {
        tok: Tok,
    },
    DeployAdapter,
    DeployComet,
    InitComet,
    DeployEmitter,
    DeployBackstop,
    DeployFactory,
    DeployPool,
    QueueReserve {
        tok: Tok,
    },
    SetReserve {
        tok: Tok,
    },
    SeedBackstop,
    SetStatus {
        to: u32,
    },
    UpdateStatus,
    /// Mint one configured playable balance, by index into `cfg.balances`.
    Mint {
        index: usize,
    },
    SeedPositions,
    Publish,
}

/// The create2 salt role each deterministically deployed market contract is
/// created at, or `None` for operations that consume no salt: uploads are
/// content-addressed by their Wasm hash, and configuration or value-moving
/// invokes mutate state without creating an address. The table below is
/// therefore exactly the set of deterministic deployments.
///
/// Every role is distinct from every other role, and the market range
/// (201..236) is disjoint from the pair graph's roles (`10 + i`, `20`, `30`,
/// `40`, `41`, `50`, `60`); the market graph signs with side `'m'` while the
/// pair graph uses `'a'`/`'b'`. `market_salt_table_is_collision_free` proves
/// the whole table is collision-free.
pub(crate) fn market_salt_role(op: &MOp) -> Option<u32> {
    Some(match op {
        MOp::DeployToken { tok } => 201 + Tok::MOCK.iter().position(|t| *t == *tok)? as u32,
        MOp::DeploySource { index } if *index < 3 => 211 + *index as u32,
        MOp::DeployGovernance => 222,
        MOp::DeployRuntime => 221,
        MOp::DeployAdapter => 223,
        MOp::DeployComet => 231,
        MOp::DeployEmitter => 232,
        MOp::DeployBackstop => 233,
        MOp::DeployFactory => 234,
        MOp::DeployPool => 235,
        _ => return None,
    })
}

/// The `(nonce, role, side)` triples the graph consumes for one nonce.
pub(crate) fn market_salt_triples(nonce: u64) -> Vec<(u64, u32, char)> {
    let mut triples: Vec<(u64, u32, char)> = market_schedule()
        .iter()
        .filter_map(|s| market_salt_role(&s.op))
        .map(|role| (nonce, role, MARKET_SIDE))
        .collect();
    triples.sort();
    triples
}

/// Whether a journal coordinate falls inside the range the scan inspects.
pub(crate) fn market_coordinate_is_scanned(phase: u32, slot: u32) -> bool {
    (1..=MARKET_SCANNED_PHASES).contains(&phase) && (1..=MARKET_SCANNED_SLOTS).contains(&slot)
}

/// Whether this run directory holds an interrupted local publication: the
/// publication coordinate's frozen intent is open with no settlement. The
/// strict journal reader answers, so a malformed or foreign record is a
/// refusal rather than an admission, and a settled publication is never
/// reported pending. Both resume entrypoints need this, because a manifest
/// file on disk is not proof of a completed run: a run interrupted between the
/// manifest's atomic rename and its settlement holds a manifest with no
/// settlement, and only the journal can tell that apart from a finished run.
pub(crate) fn publication_is_unsettled(out: &Path) -> anyhow::Result<bool> {
    let coordinate = market_schedule()
        .into_iter()
        .find(|s| s.op == MOp::Publish)
        .ok_or_else(|| anyhow!("the frozen graph schedules no publication coordinate"))?;
    Ok(matches!(
        crate::market_status::read_journal_coordinate(out, coordinate.phase, coordinate.slot)?,
        Some((_, None))
    ))
}

/// The frozen 12-phase protected-market graph, in executable order.
///
/// Order law: every artifact is uploaded before anything is deployed; the
/// ownership cutover completes before the adapter is deployed; the adapter is
/// deployed before the pool is deployed; the manifest is published last,
/// after every battery has passed.
///
/// Why the cutover is not a transfer: the frozen proxy runtime's constructor
/// binds ownership to the governance contract *at deploy time*
/// (`contract/src/lib.rs:129`), and the frozen governance constructor takes
/// the proxy address in return (`governance-contract/src/lib.rs:43`).
/// Neither address can exist before the other is chosen, so both are
/// established by deterministic prediction, the runtime is deployed already
/// governance-owned, and ownership is never operator-held at any point — it
/// is verified as a read-only precondition rather than moved. Proxy
/// configuration is owner-gated, so it runs as governance proposals, and
/// every proposal is two journaled intents (create, then execute).
pub(crate) fn market_schedule() -> Vec<MSlot> {
    let mut slots: Vec<MSlot> = Vec::new();
    let mut add =
        |phase: u32, slot: u32, action: &'static str, kind: MarketMutationKind, op: MOp| {
            slots.push(MSlot {
                phase,
                slot,
                action,
                kind,
                op,
            })
        };
    // Phase 1: run-local actor identities. Provisioning submits a funding
    // operation, so it is gated and journaled like any other mutation.
    for (i, who) in [Who::Frodo, Who::Sam, Who::User].iter().enumerate() {
        add(
            1,
            1 + i as u32,
            "provision actor identity",
            MarketMutationKind::FundKey,
            MOp::FundKey { who: *who },
        );
    }
    // Phase 2: every pinned artifact, uploaded before anything is deployed.
    for (i, role) in MARKET_ARTIFACT_ROLES.iter().enumerate() {
        add(
            2,
            1 + i as u32,
            "upload artifact",
            MarketMutationKind::Upload,
            MOp::Upload { role },
        );
    }
    // Phase 3: the five fake tokens, each deployed and then initialized with
    // the operator G-address as its admin and the token's own decimals.
    for (i, tok) in Tok::MOCK.iter().enumerate() {
        add(
            3,
            1 + i as u32,
            "deploy token",
            MarketMutationKind::Deploy,
            MOp::DeployToken { tok: *tok },
        );
    }
    for (i, tok) in Tok::MOCK.iter().enumerate() {
        add(
            3,
            6 + i as u32,
            "initialize token",
            MarketMutationKind::Config,
            MOp::InitToken { tok: *tok },
        );
    }
    // Phase 4: three independent sources with the frozen SEP-40 shape.
    for i in 0..3usize {
        add(
            4,
            1 + i as u32,
            "deploy source",
            MarketMutationKind::Deploy,
            MOp::DeploySource { index: i },
        );
    }
    for i in 0..3usize {
        add(
            4,
            4 + i as u32,
            "configure source",
            MarketMutationKind::Config,
            MOp::InitSource { index: i },
        );
    }
    for i in 0..3usize {
        add(
            4,
            7 + i as u32,
            "publish source prices",
            MarketMutationKind::Invoke,
            MOp::SourcePrice { index: i },
        );
    }
    // Phase 5: the proxy runtime, governance-owned from the moment it exists.
    add(
        5,
        1,
        "deploy proxy runtime",
        MarketMutationKind::Deploy,
        MOp::DeployRuntime,
    );
    // Phase 7: governance materializes and then authorizes the proxy
    // configuration only it can perform — all three sources per asset,
    // 3-of-3, 3600s maximum age, 60s clock drift — as one proposal pair per
    // asset, each proposal two separate journaled intents.
    //
    // Proposal-id law (verified against the pinned sources): the kernel's
    // `next_id` starts at 0 and `create` rejects any id != next_id, so the
    // four SetProxy proposals occupy ids 0..=3 in oracle order. No enabling
    // proposals exist or are needed: `require_authorized` admits the caller
    // holding `Role::Admin` (governance-contract/src/engine.rs:37) and the
    // constructor grants Admin to the frozen operator. A zero uniform TTL at
    // construction is what the frozen adjacent execute slots require; the
    // execute arm still proves ttl_ns == 0 read-only before submitting.
    add(
        7,
        1,
        "deploy governance",
        MarketMutationKind::Deploy,
        MOp::DeployGovernance,
    );
    for (i, tok) in Tok::ORACLE_ORDER.iter().enumerate() {
        add(
            7,
            2 + i as u32,
            "propose proxy configuration",
            MarketMutationKind::Governance,
            MOp::Propose { tok: *tok },
        );
    }
    // The proposal id a slot executes is the oracle-order position of the
    // asset whose SetProxy proposal it is: governance assigns ids in create
    // order and the create slots above run in oracle order, so position,
    // scheduled slot, and proposal id stay one and the same law.
    for (id, _tok) in Tok::ORACLE_ORDER.iter().enumerate() {
        add(
            7,
            6 + id as u32,
            "execute proxy configuration",
            MarketMutationKind::Governance,
            MOp::ExecuteProposal { id: id as u64 },
        );
    }
    // Phase 6: one refresh per asset, proving the configured quorum serves.
    for (i, tok) in Tok::ORACLE_ORDER.iter().enumerate() {
        add(
            6,
            1 + i as u32,
            "refresh asset",
            MarketMutationKind::Invoke,
            MOp::Refresh { tok: *tok },
        );
    }
    // Phase 8: the pool's price adapter — governance-owned, reading only the
    // proxy runtime, resolution 1.
    add(
        8,
        1,
        "deploy adapter",
        MarketMutationKind::Deploy,
        MOp::DeployAdapter,
    );
    // Phase 9: Comet and emitter, the predicted-ID backstop/factory cycle,
    // and the pool with the operator as admin and the adapter as oracle.
    add(
        9,
        1,
        "deploy comet",
        MarketMutationKind::Deploy,
        MOp::DeployComet,
    );
    add(
        9,
        2,
        "initialize comet",
        MarketMutationKind::Config,
        MOp::InitComet,
    );
    add(
        9,
        3,
        "deploy emitter",
        MarketMutationKind::Deploy,
        MOp::DeployEmitter,
    );
    add(
        9,
        4,
        "deploy backstop",
        MarketMutationKind::Deploy,
        MOp::DeployBackstop,
    );
    add(
        9,
        5,
        "deploy factory",
        MarketMutationKind::Deploy,
        MOp::DeployFactory,
    );
    add(
        9,
        6,
        "deploy pool",
        MarketMutationKind::Deploy,
        MOp::DeployPool,
    );
    // Phase 10: reserves, backstop seeding, and the status transition.
    for (i, tok) in Tok::RESERVES.iter().enumerate() {
        add(
            10,
            1 + i as u32,
            "queue reserve",
            MarketMutationKind::Config,
            MOp::QueueReserve { tok: *tok },
        );
    }
    for (i, tok) in Tok::RESERVES.iter().enumerate() {
        add(
            10,
            4 + i as u32,
            "enable reserve",
            MarketMutationKind::Config,
            MOp::SetReserve { tok: *tok },
        );
    }
    add(
        10,
        7,
        "seed backstop",
        MarketMutationKind::Invoke,
        MOp::SeedBackstop,
    );
    add(
        10,
        8,
        "lower pool status",
        MarketMutationKind::Config,
        MOp::SetStatus { to: 3 },
    );
    add(
        10,
        9,
        "advance pool status",
        MarketMutationKind::Invoke,
        MOp::UpdateStatus,
    );
    // Phase 11: configured playable balances and the seeded healthy positions.
    for i in 0..MARKET_BALANCE_COUNT {
        add(
            11,
            1 + i as u32,
            "mint playable balance",
            MarketMutationKind::Invoke,
            MOp::Mint { index: i },
        );
    }
    add(
        11,
        1 + MARKET_BALANCE_COUNT as u32,
        "seed positions",
        MarketMutationKind::Invoke,
        MOp::SeedPositions,
    );
    // Phase 12: publication, after every battery has passed.
    add(
        12,
        1,
        "publish market manifest",
        MarketMutationKind::Config,
        MOp::Publish,
    );
    slots
}

/// The number of configured playable balances the graph mints. The count is
/// part of the frozen plan; a configuration with a different number of
/// balances is a different deployment, bound by its own config digest.
pub(crate) const MARKET_BALANCE_COUNT: usize = 3;

/// The frozen deployment plan (plan §5): written atomically, create-new,
/// before the first journaled mutation, and the fact a resume reproduces.
/// A run whose plan cannot be reproduced is refused, not guessed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentPlan {
    pub schema: u32,
    pub config_digest: String,
    pub nonce: String,
    pub network_label: String,
    pub passphrase: String,
    pub admin: String,
    pub artifacts: BTreeMap<String, Sha256Hex>,
    pub salt_roles: Vec<u32>,
    pub slots: Vec<(u32, u32)>,
}

/// Read the frozen plan. `Ok(None)` when this run has none yet.
pub(crate) fn read_deployment_plan(out: &Path) -> anyhow::Result<Option<DeploymentPlan>> {
    let path = out.join(MARKET_PLAN_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes).context("deployment plan is not UTF-8")?;
            let plan: DeploymentPlan =
                toml::from_str(&text).context("deployment plan is malformed")?;
            anyhow::ensure!(plan.schema == 1, "deployment plan has an unknown schema");
            anyhow::ensure!(
                plan.passphrase == MARKET_TESTNET_PASSPHRASE
                    && plan.network_label == MARKET_NETWORK_LABEL,
                "deployment plan is not bound to the exact testnet"
            );
            Ok(Some(plan))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::new(e).context("deployment plan is unreadable")),
    }
}

/// Write the frozen plan exactly once, atomically: a temporary file, its
/// bytes and file synchronization, a no-replace rename, then the directory's
/// synchronization. A plan that already exists is never rewritten, so a
/// halted run cannot be quietly re-planned.
pub(crate) fn write_deployment_plan(out: &Path, plan: &DeploymentPlan) -> anyhow::Result<()> {
    std::fs::create_dir_all(out)
        .context("creating the run directory that holds its deployment plan")?;
    let final_path = out.join(MARKET_PLAN_FILE);
    anyhow::ensure!(
        !final_path.exists(),
        "a frozen deployment plan already exists; it can only be abandoned, never rewritten"
    );
    let text =
        toml::to_string_pretty(plan).map_err(|e| anyhow!("serializing deployment plan: {e}"))?;
    let tmp = out.join(format!("{MARKET_PLAN_FILE}.{}.tmp", std::process::id()));
    use std::io::Write as _;
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .context("creating the deployment plan staging file")?;
        file.write_all(text.as_bytes())
            .context("writing the deployment plan")?;
        file.sync_all().context("syncing the deployment plan")?;
    }
    std::fs::rename(&tmp, &final_path).context("freezing the deployment plan")?;
    sync_dir(out);
    anyhow::ensure!(
        read_deployment_plan(out)?.is_some(),
        "the frozen deployment plan is unreadable after publication"
    );
    Ok(())
}

/// Synchronize a directory so a rename or newly created entry is durable.
fn sync_dir(path: &Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.sync_all();
    }
}

impl MarketDeployer {
    pub fn new(
        cfg: MarketConfig,
        ctx: MarketExecutionContext,
        journal: MarketJournal,
    ) -> anyhow::Result<Self> {
        ctx.assert_testnet()?;
        // Deploy-time law: no reserve may inherit a fixture cap, factor or
        // rate curve. Checked before any executor call.
        cfg.require_explicit_reserves()?;
        let exec = RealExecutor {
            bin: ctx.stellar_bin().to_path_buf(),
            xdg: ctx.xdg().to_path_buf(),
            cwd: ctx.cwd().to_path_buf(),
            capture: Capture::Market {
                stdout_cap: MARKET_CAPTURE_STDOUT_CAP,
                stderr_cap: MARKET_CAPTURE_STDERR_CAP,
                wall: Some(MARKET_MUTATION_WALL),
            },
        };
        let mut d = Self {
            cfg,
            ctx,
            journal,
            exec,
            halted: false,
            unresolved: None,
            gate: None,
            progress: Progress::default(),
            manifest: None,
            terminal: None,
            plan_written: false,
            nonce: 0,
            artifacts: BTreeMap::new(),
            deployed: BTreeMap::new(),
        };
        // Settled journal results rebuild deployment and identity state
        // first, so reconciliation and every later lookup bind from the
        // same facts the journal already attests.
        d.rebuild_from_journal()?;
        // Preflight first (read-only): reconciliation proofs bind the
        // resolved operator, so scanning before the admin is known would
        // silently fail every deterministic-deploy proof.
        match d.preflight() {
            Ok(p) => {
                d.progress.admin = Some(p.admin);
                d.artifacts = p.artifacts;
                d.nonce = p.nonce;
                // A run that already has a frozen plan is a resume, not a new
                // deployment. The plan is never rewritten; it is verified to
                // reproduce from this configuration and, if it does, the run
                // continues under the custody law. A plan that does not
                // reproduce means the configuration changed under the run, and
                // the run is refused rather than quietly re-planned.
                match read_deployment_plan(&d.journal.out()) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        if let Err(e) = write_deployment_plan(&d.journal.out(), &d.frozen_plan()) {
                            d.terminal =
                                Some(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)));
                            return Ok(d);
                        }
                    }
                    Err(e) => {
                        d.terminal =
                            Some(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)));
                        return Ok(d);
                    }
                }
                d.plan_written = true;
                if let Some(e) = d.verify_frozen_plan() {
                    d.terminal = Some(DeployEvent::Failed(e));
                }
            }
            Err(e) => {
                d.terminal = Some(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)));
            }
        }
        // Reconciliation is the one thing a refused resume must never do: it
        // can settle a coordinate and write the run's manifest. So the scan
        // runs only for a run its own validation accepted. A run refused by
        // preflight, by the plan write, or by the frozen-plan check leaves
        // every existing record exactly as it found it — including a resume
        // attempted under a different operator or configuration, whose records
        // belong to the run the plan froze. `step` already refuses to honor a
        // command once a terminal event is bound, so this is the same law the
        // rest of the machine already applies.
        if d.terminal.is_none() {
            d.scan_journal()?;
            if d.unresolved.is_some() {
                d.terminal = Some(DeployEvent::Custody {
                    unresolved: true,
                    note: d.unresolved.clone().unwrap(),
                });
            }
        }
        Ok(d)
    }

    /// Read-only preflight (plan §4 phase 1). Proves the tool version, every
    /// pinned artifact's re-derived SHA-256, and the operator's resolved
    /// G-address, using no mutation, no simulation, and no ledger read. A run
    /// that cannot prove all three binds no plan and authorizes no mutation.
    ///
    /// The digest check re-reads each artifact and hashes its bytes rather
    /// than trusting the pin or the path: a swapped file or a build that
    /// drifted from what was reviewed fails here, while a malformed or
    /// placeholder-shaped pin never parsed as a `MarketConfig` at all.
    fn preflight(&self) -> anyhow::Result<Preflight> {
        self.cfg.validate()?;
        self.cfg.check_artifacts()?;
        let net = self.ctx.actor_net();
        let st = Stellar::new(&net, &self.exec, Policy::Market);
        st.assert_rpc_testnet()?;
        st.assert_version()?;
        let mut artifacts: BTreeMap<String, Sha256Hex> = BTreeMap::new();
        for role in MARKET_ARTIFACT_ROLES {
            let pin = self
                .cfg
                .artifacts
                .get(role)
                .ok_or_else(|| anyhow!("a required artifact role is unpinned"))?;
            let path = expand_home(&pin.path);
            anyhow::ensure!(
                path.is_absolute() && path.is_file(),
                "a pinned artifact is missing from disk"
            );
            let bytes = std::fs::read(&path).context("reading a pinned artifact")?;
            let digest = Sha256Hex::try_from(sha256_hex(&bytes))?;
            anyhow::ensure!(
                digest == pin.sha256,
                "a pinned artifact's bytes do not match its digest"
            );
            artifacts.insert((*role).to_string(), digest);
        }
        let admin = Stellar::new(&self.ctx.admin_net(), &self.exec, Policy::Market)
            .keys_public_key(self.ctx.admin_alias())?
            .ok_or_else(|| anyhow!("the administrator identity did not resolve to a G-address"))?;
        anyhow::ensure!(
            self.cfg.balances.len() == MARKET_BALANCE_COUNT,
            "the configured playable balances do not match the frozen mint schedule"
        );
        let nonce = market_run_nonce(&self.cfg, &self.journal.out())?;
        Ok(Preflight {
            admin,
            artifacts,
            nonce,
        })
    }

    /// The plan this run must freeze: the configuration digest, the nonce,
    /// the exact testnet, the resolved operator, the proven artifact
    /// digests, and every journal coordinate and salt role the graph
    /// consumes.
    fn frozen_plan(&self) -> DeploymentPlan {
        DeploymentPlan {
            schema: 1,
            config_digest: market_config_digest(&self.cfg).unwrap_or_else(|_| "0".repeat(64)),
            nonce: format!("{:016x}", self.nonce),
            network_label: MARKET_NETWORK_LABEL.to_string(),
            passphrase: MARKET_TESTNET_PASSPHRASE.to_string(),
            admin: self
                .progress
                .admin
                .as_ref()
                .map(|a| a.as_str().to_string())
                .unwrap_or_default(),
            artifacts: self.artifacts.clone(),
            salt_roles: market_salt_triples(self.nonce)
                .into_iter()
                .map(|(_, role, _)| role)
                .collect(),
            slots: market_schedule()
                .iter()
                .map(|s| (s.phase, s.slot))
                .collect(),
        }
    }

    /// Prove the frozen plan is reproducible and internally lawful. A
    /// coordinate outside the scanned range would be invisible to the
    /// no-replay scan, and a reused salt would deploy two contracts at one
    /// address, so the run is refused rather than silently endangered.
    /// Returns the rejection, if any.
    fn verify_frozen_plan(&self) -> Option<Safe> {
        let plan = match read_deployment_plan(&self.journal.out()) {
            Ok(Some(plan)) => plan,
            Ok(None) => self.frozen_plan(),
            Err(_) => {
                return Some(Safe::literal(
                    "the frozen deployment plan on disk is unreadable; the run is refused, not re-planned",
                ))
            }
        };
        if plan != self.frozen_plan() {
            return Some(Safe::literal(
                "the frozen deployment plan does not reproduce from this configuration; the run is refused, not re-planned",
            ));
        }
        let schedule = market_schedule();
        if schedule
            .iter()
            .any(|s| !market_coordinate_is_scanned(s.phase, s.slot))
        {
            return Some(Safe::literal(
                "the deployment graph schedules a journal coordinate outside the range the no-replay scan inspects",
            ));
        }
        let coordinates: std::collections::BTreeSet<(u32, u32)> =
            schedule.iter().map(|s| (s.phase, s.slot)).collect();
        if coordinates.len() != schedule.len() {
            return Some(Safe::literal(
                "the deployment graph reuses a journal coordinate; the run is refused before any mutation",
            ));
        }
        let triples = market_salt_triples(self.nonce);
        let roles: std::collections::BTreeSet<u32> =
            triples.iter().map(|(_, role, _)| *role).collect();
        if roles.len() != triples.len() {
            return Some(Safe::literal(
                "two deterministic deployments would share a create2 salt; the run is refused before any mutation",
            ));
        }
        None
    }

    /// Scan every numbered phase/slot coordinate for custody state before
    /// any command is honored: an intent without a settled result is
    /// ambiguous and must be reconciled under the resume law (plan §4)
    /// before the run may proceed. No replay happens before this completes.
    fn scan_journal(&mut self) -> anyhow::Result<()> {
        for phase in 1..=12u32 {
            let mut settled = 0usize;
            let mut touched = 0usize;
            for slot in 1..=40u32 {
                let intent = self.journal.read_intent(phase, slot)?;
                let Some(record) = intent else { continue };
                touched += 1;
                if self.journal.result(phase, slot)?.is_some() {
                    settled += 1;
                    continue;
                }
                match self.reconcile_pending(phase, slot, &record)? {
                    Reconciled::Settled => settled += 1,
                    Reconciled::Unresolved(note) => {
                        self.unresolved = Some(note);
                        return Ok(());
                    }
                }
            }
            if touched > 0 && settled == touched {
                self.progress.phase_done.insert(phase);
            }
        }
        Ok(())
    }

    /// Resume law (plan §4), per mutation kind. Ambiguity is never retried
    /// blindly; only the kind's declared public proof may settle it.
    fn reconcile_pending(
        &mut self,
        phase: u32,
        slot: u32,
        record: &MarketCustodyRecord,
    ) -> anyhow::Result<Reconciled> {
        let unresolved_note = || {
            DeployEvent::Custody {
                unresolved: true,
                note: Safe::literal(
                    "unresolved market custody: the declared public proof did not uniquely attribute the submission; the run is abandoned until reconciled",
                ),
            }
        };
        let _ = unresolved_note;
        // Upload: observed, never settled on inspection. See `observe_upload`
        // for why no local evidence can answer whether the chain holds the
        // code, and why a wrong answer here is unrecoverable.
        if record.kind == MarketMutationKind::Upload {
            return Ok(self.observe_upload(phase, slot, record));
        }
        // The adjudicated no-op coordinate settles on resume the same way it
        // settles when executed: nothing was or will be submitted for it.
        if market_schedule()
            .into_iter()
            .any(|s| s.phase == phase && s.slot == slot && s.op == MOp::SeedPositions)
        {
            let step = MarketStepRecord {
                phase,
                action: record.action.clone(),
                command: Safe::render_parsed(
                    Policy::Market,
                    &json!({"reconciled": true, "phase": phase, "slot": slot}),
                    200,
                ),
                outcome: Safe::literal(
                    "position seeding deliberately not performed; adjudicated no-op, nothing was submitted",
                ),
                target: record.target.clone(),
                violations: Vec::new(),
            };
            self.journal.finish(
                phase,
                slot,
                &MarketPendingResult {
                    custody: record.clone(),
                    step,
                },
            )?;
            return Ok(Reconciled::Settled);
        }
        // The publication coordinate submits nothing to the chain, so its
        // open intent is never an ambiguous submission: it is completed under
        // its own local law rather than asked of a contract read that could
        // not answer whether a local file was written.
        if let Some(coordinate) = market_schedule()
            .into_iter()
            .find(|s| s.phase == phase && s.slot == slot && s.op == MOp::Publish)
        {
            return match self.reconcile_publish(&coordinate, record) {
                Ok(()) => Ok(Reconciled::Settled),
                // A refusal here is the publication's own strict law (a
                // manifest that does not reproduce this run's state), so the
                // coordinate stays unresolved and nothing is replayed.
                Err(e) => Ok(Reconciled::Unresolved(Safe::error_chain(
                    Policy::Market,
                    &e,
                ))),
            };
        }
        let proof = match record.kind {
            // Upload is handled above and can never settle here.
            MarketMutationKind::Upload => Ok(false),
            // Funded key: the frozen actor G-address and account existence.
            MarketMutationKind::FundKey => self.reconcile_funded_key(record),
            // Deterministic deploy: predicted id plus constructor readback.
            MarketMutationKind::Deploy => self.reconcile_deterministic_deploy(record),
            // Idempotent config: exact readback of configured state.
            MarketMutationKind::Config => self.reconcile_config(record),
            // Non-idempotent value movements and governance composites can
            // only be attributed from recorded pre-state plus attributable
            // evidence and post-state deltas; that attribution requires the
            // settled record, which is exactly what is missing.
            MarketMutationKind::Invoke | MarketMutationKind::Governance => Ok(false),
        }?;
        if proof {
            let step = MarketStepRecord {
                phase,
                action: record.action.clone(),
                command: Safe::render_parsed(
                    Policy::Market,
                    &json!({"reconciled": true, "phase": phase, "slot": slot}),
                    200,
                ),
                outcome: Safe::literal("reconciled by declared public proof; not replayed"),
                target: record.target.clone().or_else(|| record.predicted.clone()),
                violations: Vec::new(),
            };
            self.journal.finish(
                phase,
                slot,
                &MarketPendingResult {
                    custody: record.clone(),
                    step,
                },
            )?;
            return Ok(Reconciled::Settled);
        }
        Ok(Reconciled::Unresolved(Safe::literal(
            "unresolved market custody: the declared public proof did not uniquely attribute the submission; the run is abandoned until reconciled",
        )))
    }

    /// Funded-key reconciliation: frozen G-address from the intent plus
    /// account existence. A keystore hit proves nothing about the account;
    /// the actor identity must resolve to the exact frozen G-address.
    fn reconcile_funded_key(&self, record: &MarketCustodyRecord) -> anyhow::Result<bool> {
        let Some(name) = self.actor_name_for(record) else {
            return Ok(false);
        };
        let net = self.ctx.actor_net();
        let st = Stellar::new(&net, &self.exec, Policy::Market);
        let found = st
            .keys_public_key(&name)?
            .map(|a| a == record.signer)
            .unwrap_or(false);
        Ok(found)
    }

    /// The actor whose recorded identity matches a signer address. An address
    /// shared by more than one actor cannot be attributed, so the lookup
    /// resolves to nothing rather than to the first match: an ambiguous signer
    /// is never given a confident name in the journal or in reconciliation.
    fn actor_name_for(&self, record: &MarketCustodyRecord) -> Option<String> {
        let matched: Vec<String> = Who::ALL
            .iter()
            .filter(|who| self.progress.identities.get(who) == Some(&record.signer))
            .map(|who| who.key_name())
            .collect();
        if matched.len() == 1 {
            matched.into_iter().next()
        } else {
            None
        }
    }

    /// Upload custody observation. An open upload intent is **never settled
    /// on inspection**, and this is the law rather than a placeholder.
    ///
    /// Work the crash case this arm exists for: an intent is written, the
    /// process dies before anything is submitted, and no transaction ever
    /// reaches the chain. On resume the pinned artifact is still sitting on
    /// the operator's disk with exactly the digest the intent declared, so any
    /// local comparison — against the pin, against the intent, against
    /// anything — returns the same answer it would return had the upload
    /// landed. Settling on it would mark the coordinate complete, move the
    /// graph past the upload phase, and deploy contracts whose code the chain
    /// never received, carrying a journal record that asserts an upload that
    /// never happened. That is unrecoverable: a coordinate recorded complete
    /// is never uploaded.
    ///
    /// No public read in this surface answers whether the chain holds a Wasm
    /// hash, so the coordinate stays unresolved and reaches the operator. The
    /// artifact is still re-derived, as an observation only, because two
    /// different emergencies need two different answers: bytes that changed
    /// underneath a booked mutation, and an intact artifact whose submission
    /// simply remains an unanswered question about the chain.
    fn observe_upload(&self, phase: u32, slot: u32, record: &MarketCustodyRecord) -> Reconciled {
        let unresolved = |drift: bool| {
            Reconciled::Unresolved(Safe::literal(if drift {
                "unresolved market custody: the artifact behind an open upload intent no longer matches the digest that intent declared; the pinned bytes changed underneath this run and nothing was replayed"
            } else {
                "unresolved market custody: an upload intent is open, the pinned artifact is intact on disk, and no public read can show the chain holds the code; nothing was assumed complete and nothing was replayed"
            }))
        };
        let Some(declared) = record.artifact_digest.as_ref() else {
            return unresolved(false);
        };
        let Some(coordinate) = market_schedule()
            .into_iter()
            .find(|s| s.phase == phase && s.slot == slot)
        else {
            return unresolved(false);
        };
        let MOp::Upload { role } = &coordinate.op else {
            return unresolved(false);
        };
        let Some(pin) = self.cfg.artifacts.get(*role) else {
            return unresolved(false);
        };
        let Ok(bytes) = std::fs::read(expand_home(&pin.path)) else {
            return unresolved(false);
        };
        if Self::artifact_drifts_from_intent(&bytes, declared) {
            return unresolved(true);
        }
        // Public reconciliation read: `contract fetch --wasm-hash` retrieves
        // installed code from the chain, so a successful fetch proves the
        // chain holds exactly the declared hash's code (the fetch output
        // re-hashes to the requested hash by construction of the ledger
        // entry key). Intact-local + fetched-public settles the coordinate;
        // a fetch that fails leaves the intent unresolved for the operator.
        let net = self.ctx.admin_net();
        let st = Stellar::new(&net, &self.exec, Policy::Market);
        let scratch = self.journal.out().join(".fetch-probe.wasm");
        let fetched = st.probe_ok(&[
            "contract",
            "fetch",
            "--wasm-hash",
            declared.as_str(),
            "--out-file",
            &scratch.display().to_string(),
        ]);
        let _ = std::fs::remove_file(&scratch);
        if fetched {
            let step = MarketStepRecord {
                phase,
                action: record.action.clone(),
                command: Safe::render_parsed(
                    Policy::Market,
                    &json!({"reconciled": true, "phase": phase, "slot": slot}),
                    200,
                ),
                outcome: Safe::literal(
                    "chain holds the declared wasm hash; reconciled by public fetch, not replayed",
                ),
                target: record.target.clone(),
                violations: Vec::new(),
            };
            if self
                .journal
                .finish(
                    phase,
                    slot,
                    &MarketPendingResult {
                        custody: record.clone(),
                        step,
                    },
                )
                .is_ok()
            {
                return Reconciled::Settled;
            }
        }
        unresolved(false)
    }

    /// Whether an artifact's bytes differ from the digest an open custody
    /// intent declared for them.
    fn artifact_drifts_from_intent(bytes: &[u8], declared: &Sha256Hex) -> bool {
        sha256_hex(bytes) != declared.as_str()
    }

    /// Deterministic-deploy reconciliation: predicted id plus exact
    /// constructor readback.
    fn reconcile_deterministic_deploy(&self, record: &MarketCustodyRecord) -> anyhow::Result<bool> {
        let Some(predicted) = &record.predicted else {
            return Ok(false);
        };
        Ok(self.deploy_readback(predicted).unwrap_or(false))
    }

    /// Idempotent-config reconciliation: exact readback of the configured
    /// state. The blend pool is the only market role with a public
    /// `get_admin`; every other configured target is proven by its phase
    /// battery (proxy `get_proxy`/`get_cached`, governance
    /// `next_proposal_id`/`get_proposal`, adapter/pool effects), so an
    /// open config intent on any other target stays unresolved.
    fn reconcile_config(&self, record: &MarketCustodyRecord) -> anyhow::Result<bool> {
        let Some(target) = &record.target else {
            return Ok(false);
        };
        Ok(self.pool_admin_readback(target).unwrap_or(false))
    }

    /// Resume law for the publication coordinate. It freezes no target and
    /// submits nothing to the chain, so an open intent there is an interrupted
    /// local write rather than an ambiguous submission, and the proof it
    /// declared is local: a manifest already on disk that reproduces this
    /// run's settled facts settles the coordinate it froze. A manifest not yet
    /// written is completed through the publisher's own idempotent path, which
    /// assembles only from settled facts. The intent must also carry the
    /// operator this run resolved as its signer, because the manifest names
    /// that operator: an intent frozen by another operator is not this run's
    /// publication. The frozen intent is never rewritten, and no mutation is
    /// replayed.
    fn reconcile_publish(
        &mut self,
        coordinate: &MSlot,
        record: &MarketCustodyRecord,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            record.phase == coordinate.phase
                && record.kind == coordinate.kind
                && record.action.to_string() == coordinate.action
                && record.target.is_none()
                && record.predicted.is_none()
                && self.progress.admin.as_ref() == Some(&record.signer),
            "the open intent at the publication coordinate is not the local publication this graph froze"
        );
        let step = self.run_slot(coordinate, record)?;
        self.journal.finish(
            coordinate.phase,
            coordinate.slot,
            &MarketPendingResult {
                custody: record.clone(),
                step,
            },
        )
    }

    /// Admin-binding readback for the blend pool. The proxy runtime,
    /// governance, and adapter expose no `get_admin`; their proofs are the
    /// real getters named above.
    fn pool_admin_readback(&self, id: &ContractId) -> anyhow::Result<bool> {
        let net = self.ctx.actor_net();
        let st = Stellar::new(&net, &self.exec, Policy::Market);
        let Some(admin) = &self.progress.admin else {
            return Ok(false);
        };
        // The readback itself is the proof: predicted id + frozen admin.
        // Gating on the deployed map would deadlock a resume (that map is
        // only rebuilt from a settled result, which is what this proves).
        let source = Who::User.key_name();
        let inv = st.try_view(id.as_str(), &source, "get_admin", &[]);
        let Ok(inv) = inv else { return Ok(false) };
        Ok(inv.ok && inv.value.as_ref().and_then(Value::as_str) == Some(admin.as_str()))
    }

    /// Deployed-presence proof for deterministic deploys: only the blend
    /// pool has a public presence reader, and it must carry the frozen
    /// operator as admin. All other roles are proven by their phase
    /// batteries and downstream effects that require them.
    fn deploy_readback(&self, id: &ContractId) -> anyhow::Result<bool> {
        self.pool_admin_readback(id)
    }

    /// The bookkeeping key a deterministic deployment is recorded under, or
    /// `None` for operations that create no address.
    fn deployed_role(op: &MOp) -> Option<String> {
        Some(match op {
            MOp::DeployToken { tok } => format!("token:{}", tok.as_str()),
            MOp::DeploySource { index } => format!("source:{index}"),
            MOp::DeployRuntime => "proxy-runtime".to_string(),
            MOp::DeployGovernance => "proxy-governance".to_string(),
            MOp::DeployAdapter => "adapter".to_string(),
            MOp::DeployComet => "comet".to_string(),
            MOp::DeployEmitter => "emitter".to_string(),
            MOp::DeployBackstop => "backstop".to_string(),
            MOp::DeployFactory => "factory".to_string(),
            MOp::DeployPool => "pool".to_string(),
            _ => return None,
        })
    }

    /// A contract this run has already deployed and settled. Deployment
    /// state is rebuilt from settled journal results on every start, so a
    /// role absent here has no settled deployment in this run directory.
    fn deployed_id(&self, role: &str) -> anyhow::Result<ContractId> {
        self.deployed.get(role).cloned().ok_or_else(|| {
            anyhow!("a prior graph coordinate did not settle the deployed contract this mutation requires")
        })
    }

    fn token_id(&self, tok: Tok) -> anyhow::Result<ContractId> {
        self.deployed_id(&format!("token:{}", tok.as_str()))
    }

    /// Reconstruct deployment and identity state from settled journal
    /// results only: an actor's provisioned address from its settled
    /// funding coordinate, and each deterministic deployment from the
    /// predicted id its own settled custody record carries. Nothing here
    /// opens an intent or observes the chain.
    fn rebuild_from_journal(&mut self) -> anyhow::Result<()> {
        for slot in market_schedule() {
            let Some(result) = self.journal.result(slot.phase, slot.slot)? else {
                continue;
            };
            if let MOp::FundKey { who } = &slot.op {
                self.progress
                    .identities
                    .insert(*who, result.custody.signer.clone());
                continue;
            }
            if let (Some(role), Some(predicted)) = (
                Self::deployed_role(&slot.op),
                result.custody.predicted.clone(),
            ) {
                self.deployed.insert(role, predicted);
            }
        }
        Ok(())
    }

    /// The create2 salt the schedule consumes for this slot's salt role, or
    /// `None` for operations that deploy nothing.
    fn slot_salt(&self, slot: &MSlot) -> Option<String> {
        market_salt_role(&slot.op).map(|role| salt(self.nonce, role, MARKET_SIDE))
    }

    /// The exact governance operation and proxy configuration the schedule
    /// intends for one market asset: all three deployed source contracts —
    /// distinct oracles, which `validate_proxy_config` requires — serving
    /// the asset the proxy is asked to protect, three-of-three, 3600 seconds
    /// maximum age, 60 seconds clock drift. Every address is read from this
    /// run's settled deployments; a missing one refuses rather than
    /// substituting a placeholder.
    fn set_proxy_operation(&self, tok: Tok) -> anyhow::Result<(Value, Value)> {
        let asset = json!({ "Stellar": self.token_id(tok)?.as_str() });
        let mut sources = Vec::with_capacity(MARKET_SOURCE_COUNT);
        for index in 0..MARKET_SOURCE_COUNT {
            sources.push(json!({
                "oracle": self.deployed_id(&format!("source:{index}"))?.as_str(),
                "asset": asset,
            }));
        }
        let config = json!({
            "sources": sources,
            "min_sources": self.cfg.oracle.min_sources,
            "max_age_secs": self.cfg.oracle.max_age_secs,
            "max_clock_drift_secs": self.cfg.oracle.max_clock_drift_secs,
        });
        Ok((
            json!({ "SetProxy": [asset.clone(), config.clone()] }),
            config,
        ))
    }

    /// The pool's own config read-back, including its live status. Used to
    /// prove a status or reserve coordinate is lawful to submit at all, and
    /// to settle only on state the pool itself reports.
    fn pool_config(&self, pool: &ContractId) -> anyhow::Result<Value> {
        let admin_net = self.ctx.admin_net();
        let st = Stellar::new(&admin_net, &self.exec, Policy::Market);
        let config = st.view(pool.as_str(), self.ctx.admin_alias(), "get_config", &[])?;
        anyhow::ensure!(
            config.get("status").and_then(Value::as_u64).is_some(),
            "the pool did not report its status"
        );
        Ok(config)
    }

    /// Read `get_reserve` for `tok` and require its live `config` to equal,
    /// field for field, the configuration this market derives from its
    /// manifest. Any difference refuses: the pool seals reserve economics on
    /// leaving setup, so a drifted cap, factor or rate curve must be caught
    /// while it can still be fixed.
    fn require_reserve_matches_manifest(
        &self,
        pool: &ContractId,
        tok: Tok,
        index: u32,
    ) -> anyhow::Result<()> {
        let admin_net = self.ctx.admin_net();
        let st = Stellar::new(&admin_net, &self.exec, Policy::Market);
        let live = st.view(
            pool.as_str(),
            self.ctx.admin_alias(),
            "get_reserve",
            &[kv("asset", self.token_id(tok)?.as_str())],
        )?;
        let live_config = live
            .get("config")
            .ok_or_else(|| anyhow!("the pool did not report a reserve configuration"))?;
        let expected = reserve_config(tok, index, self.cfg.reserves.get(&tok));
        for (field, want) in expected.as_object().expect("reserve_config is an object") {
            let got = live_config.get(field);
            let equal = match want {
                Value::Bool(b) => got.and_then(Value::as_bool) == Some(*b),
                _ => got.and_then(price_as_i128) == price_as_i128(want),
            };
            anyhow::ensure!(
                equal,
                "live reserve '{}' field '{field}' does not match the market config",
                tok.as_str()
            );
        }
        Ok(())
    }

    /// The completed market's manifest, assembled only from this run's
    /// settled facts: provisioned identities, deployed addresses, and the
    /// configured network and oracle stack. Any gap refuses; publication
    /// never carries an unproven field. `deploy_ledger` is the only fact not
    /// held by the journal, so the caller supplies it: an assembly built from
    /// a published manifest's own stamp needs no observation at all.
    fn assemble_manifest(&self, deploy_ledger: u32) -> anyhow::Result<MarketManifest> {
        let admin = self
            .progress
            .admin
            .as_ref()
            .ok_or_else(|| anyhow!("the operator is unresolved"))?;
        let mut tokens = std::collections::BTreeMap::new();
        for tok in Tok::MOCK {
            tokens.insert(tok, self.token_id(tok)?);
        }
        let mut sources = Vec::with_capacity(MARKET_SOURCE_COUNT);
        for index in 0..MARKET_SOURCE_COUNT {
            sources.push(self.deployed_id(&format!("source:{index}"))?);
        }
        let mut identities = std::collections::BTreeMap::new();
        for who in [Who::Frodo, Who::Sam, Who::User] {
            identities.insert(
                who,
                self.progress
                    .identities
                    .get(&who)
                    .cloned()
                    .ok_or_else(|| anyhow!("a run-local actor is not provisioned"))?,
            );
        }
        let rpc_host = self.ctx.rpc_host_label();
        let mut reserves = std::collections::BTreeMap::new();
        for (index, tok) in Tok::RESERVES.iter().enumerate() {
            reserves.insert(
                *tok,
                ReserveRef {
                    index: index as u32,
                    decimals: tok.decimals(),
                },
            );
        }
        Ok(MarketManifest {
            network: MarketNetworkRecord {
                label: Safe::literal(MARKET_NETWORK_LABEL),
                passphrase: Safe::literal(MARKET_TESTNET_PASSPHRASE),
            },
            admin: admin.clone(),
            rpc_host: Safe::render_parsed(Policy::Market, &json!({"host": rpc_host}), 256),
            oracle: OracleStackManifest {
                runtime: self.deployed_id("proxy-runtime")?,
                governance: Some(self.deployed_id("proxy-governance")?),
                adapter: self.deployed_id("adapter")?,
                sources,
                min_sources: self.cfg.oracle.min_sources,
                max_age_secs: self.cfg.oracle.max_age_secs,
                max_clock_drift_secs: self.cfg.oracle.max_clock_drift_secs,
                decimals: self.cfg.oracle.decimals,
                resolution: self.cfg.oracle.resolution,
                base: Safe::literal(MARKET_BASE),
            },
            tokens,
            pool: self.deployed_id("pool")?,
            backstop: self.deployed_id("backstop")?,
            factory: self.deployed_id("factory")?,
            comet: self.deployed_id("comet")?,
            emitter: self.deployed_id("emitter")?,
            reserves,
            identities,
            deploy_ledger,
        })
    }

    /// The published manifest for this run directory, if one exists and
    /// reproduces the state assembled from this run's settled facts. This is
    /// the local proof a publication completed: `deploy_ledger` is a stamp of
    /// when the file was proven, not a fact a resume must re-observe, so the
    /// published stamp is fed back into assembly and the rendered bytes are
    /// required to be identical. Anything less is a refusal, never a guess.
    /// Performs no observation: it reads this run directory and nothing else.
    fn published_manifest(&self) -> anyhow::Result<Option<MarketManifest>> {
        if !self.journal.out().join(MARKET_MANIFEST_FILE).exists() {
            return Ok(None);
        }
        let published = MarketManifest::load(self.journal.out().as_path())
            .context("the published market manifest is unreadable")?;
        let assembled = self.assemble_manifest(published.deploy_ledger)?;
        anyhow::ensure!(
            toml::to_string_pretty(&published).context("rendering the market manifest")?
                == toml::to_string_pretty(&assembled).context("rendering the market manifest")?,
            "a different market manifest was already published here"
        );
        Ok(Some(published))
    }

    /// Assembly plus the one fact no journal can hold: the ledger the run was
    /// proven against. Read-only, and needed only when a manifest is
    /// published for the first time.
    fn build_manifest(&self) -> anyhow::Result<MarketManifest> {
        let admin_net = self.ctx.admin_net();
        let st = Stellar::new(&admin_net, &self.exec, Policy::Market);
        self.assemble_manifest(st.latest_ledger()?)
    }
}

/// Whether this run has a frozen deployment plan. Its mere existence binds
/// the run to custody reconciliation (plan §4): it forbids auto-continuation
/// after a halt, and it is the only fact the halt path needs. The plan lives
/// beside the journal, so this is the path that law reads.
fn read_plan(out: &Path) -> anyhow::Result<Option<()>> {
    Ok(out.join(MARKET_PLAN_FILE).exists().then_some(()))
}

impl MarketDeployer {
    /// The shared deployment state machine driven by every operator surface.
    /// Terminal events are replayed forever; nothing auto-continues past a
    /// gate, a halt, or unresolved custody.
    pub fn step(&mut self, act: DeployAct) -> anyhow::Result<DeployEvent> {
        if let Some(ev) = &self.terminal {
            return Ok(ev.clone());
        }
        self.scan_journal()?;
        if let Some(note) = &self.unresolved {
            return Ok(DeployEvent::Custody {
                unresolved: true,
                note: note.clone(),
            });
        }
        match act {
            DeployAct::Confirm { token }
                if self.gate.as_ref().is_some_and(|g| g.token == token) =>
            {
                let gate = self.gate.take().unwrap();
                if let Err(e) = self.execute_gate(&gate) {
                    return Ok(self.unresolved(
                        gate.phase,
                        gate.slot,
                        Some(Safe::error_chain(Policy::Market, &e)),
                    ));
                }
                Ok(DeployEvent::Phase {
                    id: gate.phase as usize,
                    label: Safe::literal("mutation submitted after confirmation"),
                })
            }
            DeployAct::Confirm { .. } if self.gate.is_some() => {
                let gate = self.gate.as_ref().unwrap();
                Ok(DeployEvent::Confirm {
                    token: gate.token.clone(),
                    summary: gate.summary.clone(),
                })
            }
            DeployAct::Confirm { .. } => Ok(DeployEvent::Failed(Safe::literal(
                "unexpected confirmation: nothing is awaiting a token",
            ))),
            DeployAct::Halt => {
                if self.gate.is_some() {
                    self.gate = None;
                }
                if self.plan_written {
                    let e =
                        anyhow!("deployment halted with a frozen plan; resume reconciles custody");
                    return Ok(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)));
                }
                self.halted = true;
                let e = anyhow!("deployment halted before any journaled mutation");
                Ok(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)))
            }
            DeployAct::Init | DeployAct::Continue => {
                self.plan_written = read_plan(&self.journal.out())?.is_some();
                self.advance()
            }
        }
    }

    /// Advance the graph to its next required step: open the confirmation
    /// gate for the first scheduled coordinate that has no settled result, or
    /// finish the run. Journal completion drives progress; nothing is skipped
    /// that the journal has not settled, and nothing auto-continues past a
    /// gate. Without a frozen plan and a bound nonce the graph authorizes no
    /// mutation and ends in a terminal event, never at a gate.
    fn advance(&mut self) -> anyhow::Result<DeployEvent> {
        if let Some(m) = &self.manifest {
            return Ok(DeployEvent::Done(m.clone()));
        }
        if self.nonce == 0 || !self.plan_written {
            let e = anyhow!("no frozen deployment plan is bound, so no mutation is authorized");
            self.terminal = Some(DeployEvent::Failed(Safe::error_chain(Policy::Market, &e)));
            return Ok(self.terminal.clone().unwrap());
        }
        if let Some(slot) = self.next_unsettled_slot()? {
            return self.prepare_gate(&slot);
        }
        self.publish()
    }

    /// The first scheduled coordinate with no verified settlement, in graph
    /// order, or an error when a settlement cannot be verified: an unknown
    /// custody stops the run before it can mutate or publish anything.
    fn next_unsettled_slot(&self) -> anyhow::Result<Option<MSlot>> {
        for slot in market_schedule() {
            if self.journal.result(slot.phase, slot.slot)?.is_none() {
                return Ok(Some(slot));
            }
        }
        Ok(None)
    }

    /// Open the confirmation gate for one scheduled mutation. The custody
    /// record and its payload digest are computed *before* the gate is
    /// offered, so the token the operator is asked to confirm is bound to the
    /// exact mutation content: phase, coordinate, kind, action, target,
    /// artifact digest, payload digest, and the exact testnet. Confirming one
    /// gate cannot authorize a different mutation, and a re-frozen plan cannot
    /// quietly change what a retained token releases.
    fn prepare_gate(&mut self, slot: &MSlot) -> anyhow::Result<DeployEvent> {
        if let Some(gate) = &self.gate {
            if gate.phase == slot.phase && gate.slot == slot.slot {
                return Ok(DeployEvent::Confirm {
                    token: gate.token.clone(),
                    summary: gate.summary.clone(),
                });
            }
        }
        let mut record = MarketCustodyRecord {
            phase: slot.phase,
            kind: slot.kind,
            action: Safe::literal(slot.action),
            network: MarketTestnet::Testnet,
            signer: self.slot_signer(slot)?,
            payload_digest: Sha256Hex::default(),
            target: self.slot_target(slot),
            artifact_digest: self.slot_artifact_digest(slot),
            predicted: None,
            pre_state: vec![Safe::literal(
                "no settled result exists at this coordinate; nothing has been replayed",
            )],
            tool_error: None,
        };
        // Deterministic deployments bind their predicted id into the custody
        // record — and through it into the payload digest the confirmation
        // token signs — before any mutation is offered. Most deployments
        // derive from the offline account-branch law (`contract id wasm
        // --salt --source`), not a chain observation. The pool is no
        // exception, only no longer excluded: a factory child is predictable
        // under a second proven law — keccak256(salt || admin-strkey padded
        // to 56) then sha256(XDR HashIdPreimage::ContractId) with the
        // settled factory as sender (pool_factory.rs:92-109, proven offline
        // against the real factory wasm by blend-contracts-v2/pool-proof and
        // gated by known-answer tests in this file). The raw role-235 salt
        // deploys nothing on its own and must never be the bound value; the
        // binding below is always the derived child id.
        if let Some(salt) = self.slot_salt(slot) {
            record.predicted = Some(match slot.op {
                MOp::DeployPool => {
                    // The settled factory is the only lawful sender for a
                    // pool child; it is deployed one coordinate earlier and
                    // rebuilt from its settled custody record on every
                    // start, so the gate cannot bind against an absent or
                    // substituted factory.
                    let factory = self.deployed_id("factory")?;
                    let admin = self
                        .progress
                        .admin
                        .clone()
                        .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                    let predicted = predict_pool_child(factory.as_str(), &salt, admin.as_str())?;
                    ContractId::from_str(&predicted)
                        .map_err(|_| anyhow!("the predicted pool id is malformed"))?
                }
                _ => {
                    let admin_net = self.ctx.admin_net();
                    let st = Stellar::new(&admin_net, &self.exec, Policy::Market);
                    st.predict_id(self.ctx.admin_alias(), &salt)?
                }
            });
        }
        anyhow::ensure!(
            record.target.is_some()
                || record.predicted.is_some()
                || matches!(
                    slot.op,
                    MOp::FundKey { .. } | MOp::Upload { .. } | MOp::Publish
                ),
            "market target is not bound before the confirmation gate"
        );
        record.payload_digest = custody_payload_digest(&record)?;
        let summary = Safe::render_parsed(
            Policy::Market,
            &json!({
                "phase": slot.phase,
                "slot": slot.slot,
                "action": slot.action,
                "network": MARKET_NETWORK_LABEL,
                "signer": record.signer.as_str(),
                "target": record.target.as_ref().map(|t| t.as_str()),
                "artifact_digest": record.artifact_digest.as_deref(),
                "payload_digest": record.payload_digest.as_str(),
            }),
            600,
        );
        let token = self.confirm_token(slot, &record);
        self.gate = Some(Gate {
            phase: slot.phase,
            slot: slot.slot,
            token: token.clone(),
            record,
            summary: summary.clone(),
        });
        Ok(DeployEvent::Confirm { token, summary })
    }

    /// The one-time confirmation token for one mutation: derived from the
    /// frozen configuration digest and the custody record, so it is
    /// reproducible for that mutation and useless for any other.
    fn confirm_token(&self, slot: &MSlot, record: &MarketCustodyRecord) -> String {
        let mut preimage = b"abtest-market-confirm-v1".to_vec();
        preimage.extend_from_slice(
            market_config_digest(&self.cfg)
                .unwrap_or_default()
                .as_bytes(),
        );
        preimage.extend_from_slice(&slot.phase.to_le_bytes());
        preimage.extend_from_slice(&slot.slot.to_le_bytes());
        preimage.extend_from_slice(record.payload_digest.as_bytes());
        sha256_hex(&preimage)[..40].to_uppercase()
    }

    /// The signer frozen for one scheduled mutation. Only run-local actor
    /// provisioning is signed by an actor; every other mutation is signed by
    /// the resolved operator, and an unresolved operator authorizes nothing.
    fn slot_signer(&self, slot: &MSlot) -> anyhow::Result<AccountId> {
        match &slot.op {
            MOp::FundKey { who } => {
                if let Some(addr) = self.progress.identities.get(who) {
                    return Ok(addr.clone());
                }
                let net = self.ctx.actor_net();
                let st = Stellar::new(&net, &self.exec, Policy::Market);
                // Provisioning creates the identity, so an actor that does
                // not exist yet is expected: the operator authorizes the
                // generation. Once provisioned, resume binds the actor's
                // own address.
                Ok(st.keys_public_key(&who.key_name())?.unwrap_or_else(|| {
                    self.progress
                        .admin
                        .clone()
                        .expect("preflight resolved the operator before any gate")
                }))
            }
            _ => {
                self.progress.admin.clone().ok_or_else(|| {
                    anyhow!("the operator is unresolved, so no mutation is authorized")
                })
            }
        }
    }

    /// The contract a scheduled mutation acts on, when it is already known.
    /// Deterministic deployments name their target only once they are
    /// deployed; their custody record is completed with the predicted id
    /// inside the executor, before the deploying call is made.
    ///
    /// Gate preparation binds only from facts already proven, so this is a
    /// lookup against recorded deployments and never observes the chain.
    ///
    /// A refresh runs against the proxy runtime, not the asset's token: the
    /// runtime is the contract that aggregates and caches prices, and the
    /// token names only the asset argument of the call. Binding a refresh to
    /// the token would journal a target that cannot serve the call and submit
    /// the transaction to a contract that cannot apply it.
    fn slot_target(&self, slot: &MSlot) -> Option<ContractId> {
        let role = match &slot.op {
            MOp::InitToken { tok } => format!("token:{}", tok.as_str()),
            MOp::InitSource { index } | MOp::SourcePrice { index } => format!("source:{index}"),
            MOp::InitComet => "comet".to_string(),
            MOp::Refresh { .. } => "proxy-runtime".to_string(),
            MOp::Propose { .. } | MOp::ExecuteProposal { .. } => "proxy-governance".to_string(),
            MOp::QueueReserve { .. }
            | MOp::SetReserve { .. }
            | MOp::SetStatus { .. }
            | MOp::UpdateStatus
            | MOp::SeedBackstop
            | MOp::SeedPositions => "pool".to_string(),
            MOp::Mint { index } => match self.cfg.balances.get(*index) {
                Some(b) => format!("token:{}", b.token.as_str()),
                None => return None,
            },
            _ => return None,
        };
        self.deployed.get(&role).cloned()
    }

    /// Execute one confirmed and journaled mutation, returning the settled
    /// step only on a verified outcome.
    ///
    /// A mutation is submitted only where this engine knows the pinned
    /// contract's real interface from its source. Where it does not, the
    /// mutation is refused before the call rather than guessed: a wrong
    /// function name against a live market spends a transaction and leaves a
    /// journal entry claiming an effect the chain never applied. Refusal is a
    /// definite outcome, so no ambiguity can accumulate.
    ///
    /// Settlement reads the chain back. A submission that merely reported
    /// success is not proof: an upload must return the digest it was pinned
    /// to, and a provisioned actor must resolve to an address distinct from
    /// every other actor and from the operator.
    fn run_slot(
        &mut self,
        slot: &MSlot,
        record: &MarketCustodyRecord,
    ) -> anyhow::Result<MarketStepRecord> {
        // The executing net follows the frozen signer: run-local actor
        // provisioning signs from the run-local config; every other
        // mutation is operator-signed and must resolve + sign from the
        // admin config dir.
        let net = match &slot.op {
            MOp::FundKey { .. } => self.ctx.actor_net(),
            _ => self.ctx.admin_net(),
        };
        // SEC003: bind the pinned signer once at the drive chokepoint — the
        // frozen record signer G is the transaction source and the CLI
        // signs with the alias that must own it (never a seed). The alias
        // and pinned G are bound as long-lived values before the market
        // context exists; the CLI seam re-resolves the alias immediately
        // before every mutation and refuses on any mismatch, so a rebound
        // alias can never sign an unpinned key.
        let alias = match &slot.op {
            MOp::FundKey { who } => who.key_name(),
            _ => self.ctx.admin_alias().to_string(),
        };
        let st = Stellar::new(&net, &self.exec, Policy::Market)
            .with_market_signer(&alias, &record.signer);
        let command = Safe::render_parsed(
            Policy::Market,
            &json!({"phase": slot.phase, "slot": slot.slot}),
            200,
        );
        let settled = |outcome: &'static str| MarketStepRecord {
            phase: slot.phase,
            action: Safe::literal(slot.action),
            command: command.clone(),
            outcome: Safe::literal(outcome),
            target: record.target.clone(),
            violations: Vec::new(),
        };
        match &slot.op {
            MOp::FundKey { who } => {
                let addr = st.keys_ensure_funded(&who.key_name())?;
                let mut identities = self.progress.identities.clone();
                identities.insert(*who, addr.clone());
                let mut seen = std::collections::BTreeSet::new();
                for address in identities.values() {
                    anyhow::ensure!(
                        seen.insert(address.as_str().to_string()),
                        "two run-local actor identities resolve to one address; \
                         the market cannot attribute a signer"
                    );
                }
                if let Some(admin) = &self.progress.admin {
                    anyhow::ensure!(
                        !identities.values().any(|a| a == admin),
                        "a run-local actor identity is the operator's own address; \
                         the market cannot attribute a signer"
                    );
                }
                self.progress.identities = identities;
                Ok(settled(
                    "actor identity provisioned and read back distinct from every other signer",
                ))
            }
            MOp::Upload { role } => {
                let pin =
                    self.cfg.artifacts.get(*role).ok_or_else(|| {
                        anyhow!("the artifact role this upload is pinned to is gone")
                    })?;
                let path = expand_home(&pin.path);
                let bytes = std::fs::read(&path).context("re-reading the artifact to upload")?;
                anyhow::ensure!(
                    sha256_hex(&bytes) == pin.sha256.as_str(),
                    "the artifact's bytes no longer match the digest this gate confirmed"
                );
                let hash = st.upload(&path, self.ctx.admin_alias())?;
                anyhow::ensure!(
                    hash.as_str() == pin.sha256.as_str(),
                    "the uploaded Wasm hash is not the pinned artifact digest"
                );
                Ok(settled(
                    "artifact uploaded and content-addressed to its pin",
                ))
            }
            MOp::DeployToken { tok } => {
                let artifact = self
                    .artifacts
                    .get("token")
                    .ok_or_else(|| anyhow!("the token artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled token deploy lost its salt role"))?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a token deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed token is not the contract this gate bound"
                );
                self.deployed
                    .insert(format!("token:{}", tok.as_str()), id.clone());
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "token deployed at its predicted id; the pinned sep-41 mock takes no constructor arguments",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::InitToken { tok } => {
                let id = self.token_id(*tok)?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let sym = tok.as_str().to_uppercase();
                must_ok(
                    st.invoke(
                        id.as_str(),
                        self.ctx.admin_alias(),
                        "initialize",
                        &[
                            kv("admin", admin.as_str()),
                            kv("decimal", tok.decimals()),
                            kv("name", &sym),
                            kv("symbol", &sym),
                        ],
                    )?,
                    "token initialize",
                )?;
                let decimals = st.view(id.as_str(), self.ctx.admin_alias(), "decimals", &[])?;
                let name = st.view(id.as_str(), self.ctx.admin_alias(), "name", &[])?;
                let symbol = st.view(id.as_str(), self.ctx.admin_alias(), "symbol", &[])?;
                anyhow::ensure!(
                    decimals.as_u64() == Some(u64::from(tok.decimals()))
                        && name.as_str() == Some(sym.as_str())
                        && symbol.as_str() == Some(sym.as_str()),
                    "the initialized token does not read back its scheduled metadata"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "token initialized with the operator as admin; decimals/name/symbol read back exactly",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::DeploySource { index } => {
                let artifact = self
                    .artifacts
                    .get("source")
                    .ok_or_else(|| anyhow!("the source artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled source deploy lost its salt role"))?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[kv("owner", admin.as_str())],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a source deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed source is not the contract this gate bound"
                );
                self.deployed.insert(format!("source:{index}"), id.clone());
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "price source deployed at its predicted id with the operator as its owner",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::InitSource { index } => {
                let id = self.deployed_id(&format!("source:{index}"))?;
                let base = st.view(id.as_str(), self.ctx.admin_alias(), "base", &[])?;
                let decimals = st.view(id.as_str(), self.ctx.admin_alias(), "decimals", &[])?;
                let resolution = st.view(id.as_str(), self.ctx.admin_alias(), "resolution", &[])?;
                anyhow::ensure!(
                    base == json!({ "Other": MARKET_BASE })
                        && decimals.as_u64() == Some(u64::from(self.cfg.oracle.decimals)),
                    "the deployed source does not read back the base and decimals this market requires"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::render_parsed(
                        Policy::Market,
                        &json!({
                            "read_only": true,
                            "detail": "the pinned source exposes no post-constructor configuration; this coordinate settles on the read-only proof alone. resolution read from the chain:",
                            "resolution": resolution,
                        }),
                        400,
                    ),
                    target: Some(id),
                    violations: if resolution.as_u64() == Some(u64::from(MARKET_SOURCE_RESOLUTION))
                    {
                        Vec::new()
                    } else {
                        vec![Safe::literal(
                            "the market config declares source resolution 300; the pinned source reports resolution 1 (latest-only feed). metadata mismatch, reported not gated",
                        )]
                    },
                })
            }
            MOp::SourcePrice { index } => {
                let source = self.deployed_id(&format!("source:{index}"))?;
                // Fresh, strictly-increasing, never future-dated: publication
                // time trails the wall clock by two minutes — inside the
                // proxy's 60-second clock-drift bound and the 3600-second
                // freshness bound, and monotonic per asset within this slot.
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| anyhow!("the wall clock is behind the epoch"))?
                    .as_secs()
                    .saturating_sub(120);
                for (i, tok) in Tok::ORACLE_ORDER.iter().enumerate() {
                    let asset = json!({ "Stellar": self.token_id(*tok)?.as_str() }).to_string();
                    let timestamp = now + i as u64;
                    let price = fixture_price(*tok);
                    must_ok(
                        st.invoke(
                            source.as_str(),
                            self.ctx.admin_alias(),
                            "set_data",
                            &[
                                kv("asset", asset.clone()),
                                kv("price", price.to_string()),
                                kv("timestamp", timestamp.to_string()),
                            ],
                        )?,
                        "source set_data",
                    )?;
                    let lastprice = st.view(
                        source.as_str(),
                        self.ctx.admin_alias(),
                        "lastprice",
                        &[kv("asset", asset)],
                    )?;
                    anyhow::ensure!(
                        lastprice.get("price").and_then(price_as_i128) == Some(price)
                            && lastprice.get("timestamp").and_then(Value::as_u64)
                                == Some(timestamp),
                        "a published source price does not read back"
                    );
                }
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the source published a fresh, strictly increasing price for every market asset; each published price read back from lastprice",
                    ),
                    target: Some(source),
                    violations: Vec::new(),
                })
            }
            MOp::DeployRuntime => {
                let artifact = self
                    .artifacts
                    .get("proxy-runtime")
                    .ok_or_else(|| anyhow!("the proxy runtime artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled runtime deploy lost its salt role"))?;
                let governance_salt = salt(
                    self.nonce,
                    market_salt_role(&MOp::DeployGovernance)
                        .ok_or_else(|| anyhow!("the governance salt role is gone"))?,
                    MARKET_SIDE,
                );
                let admin_net = self.ctx.admin_net();
                let st_pred = Stellar::new(&admin_net, &self.exec, Policy::Market);
                let predicted_governance =
                    st_pred.predict_id(self.ctx.admin_alias(), &governance_salt)?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[
                        kv("governance", predicted_governance.as_str()),
                        kv("base", json!({ "Other": MARKET_BASE }).to_string()),
                    ],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a runtime deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed runtime is not the contract this gate bound"
                );
                self.deployed
                    .insert("proxy-runtime".to_string(), id.clone());
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "proxy runtime deployed at its predicted id, governance-owned from its first block",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::DeployGovernance => {
                let artifact = self
                    .artifacts
                    .get("proxy-governance")
                    .ok_or_else(|| anyhow!("the governance artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled governance deploy lost its salt role"))?;
                let runtime = self.deployed_id("proxy-runtime")?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[
                        kv("admin", admin.as_str()),
                        kv("proxy_oracle", runtime.as_str()),
                        kv("initial_uniform_ttl_ns", "0"),
                    ],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a governance deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed governance contract is not the contract this gate bound"
                );
                self.deployed
                    .insert("proxy-governance".to_string(), id.clone());
                let bound = st.view(id.as_str(), self.ctx.admin_alias(), "proxy_oracle", &[])?;
                let admin_role = st.view(
                    id.as_str(),
                    self.ctx.admin_alias(),
                    "has_role",
                    &[kv("account", admin.as_str()), kv("role", "Admin")],
                )?;
                let next_id =
                    st.view(id.as_str(), self.ctx.admin_alias(), "next_proposal_id", &[])?;
                anyhow::ensure!(
                    bound.as_str() == Some(runtime.as_str())
                        && admin_role.as_bool() == Some(true)
                        && next_id.as_u64() == Some(0),
                    "the deployed governance contract does not read back the bound runtime, the operator's admin role, and a fresh proposal counter"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "governance deployed at its predicted id; uniform proposal timelock constructed at zero, as the frozen adjacent execute schedule requires",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::Propose { tok } => {
                let governance = self.deployed_id("proxy-governance")?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let expected = Tok::ORACLE_ORDER
                    .iter()
                    .position(|t| *t == *tok)
                    .ok_or_else(|| {
                        anyhow!("a scheduled proposal names an asset outside oracle order")
                    })? as u64;
                let next_id = st.view(
                    governance.as_str(),
                    self.ctx.admin_alias(),
                    "next_proposal_id",
                    &[],
                )?;
                anyhow::ensure!(
                    next_id.as_u64() == Some(expected),
                    "governance's proposal counter is not the id this slot is scheduled to create; the slot refuses rather than renumber the graph"
                );
                let (operation, _config) = self.set_proxy_operation(*tok)?;
                must_ok(
                    st.invoke(
                        governance.as_str(),
                        self.ctx.admin_alias(),
                        "create_proposal",
                        &[
                            kv("caller", admin.as_str()),
                            kv("id", expected.to_string()),
                            kv("operation", operation.to_string()),
                            kv("requested_ttl", "0"),
                        ],
                    )?,
                    "create_proposal",
                )?;
                let stored = st.view(
                    governance.as_str(),
                    self.ctx.admin_alias(),
                    "get_proposal",
                    &[kv("id", expected.to_string())],
                )?;
                anyhow::ensure!(
                    stored
                        .get("created_by")
                        .and_then(Value::as_str)
                        == Some(admin.as_str())
                        && stored.get("ttl_ns").and_then(Value::as_u64) == Some(0)
                        && stored.get("operation").and_then(|o| o.get("SetProxy")).and_then(|a| a.get(0))
                            == operation.get("SetProxy").and_then(|a| a.get(0)),
                    "the created proposal does not read back as the scheduled SetProxy for this asset with a zero timelock"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the SetProxy proposal for this asset was created, created by the operator, and reads back with a zero timelock",
                    ),
                    target: Some(governance),
                    violations: Vec::new(),
                })
            }
            MOp::ExecuteProposal { id } => {
                let governance = self.deployed_id("proxy-governance")?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let tok = Tok::ORACLE_ORDER.get(*id as usize).ok_or_else(|| {
                    anyhow!("a scheduled execution id has no asset in oracle order")
                })?;
                let (operation, config) = self.set_proxy_operation(*tok)?;
                // Read-only proof before any submission: the proposal must
                // exist, carry the zero timelock construction established,
                // and target exactly the asset this slot schedules. A slot
                // that cannot prove maturity refuses instead of submitting
                // into a timelock trap that would burn its custody forever.
                let stored = st.view(
                    governance.as_str(),
                    self.ctx.admin_alias(),
                    "get_proposal",
                    &[kv("id", id.to_string())],
                )?;
                anyhow::ensure!(
                    stored.get("ttl_ns").and_then(Value::as_u64) == Some(0)
                        && stored
                            .get("operation")
                            .and_then(|o| o.get("SetProxy"))
                            .and_then(|a| a.get(0))
                            == operation.get("SetProxy").and_then(|a| a.get(0)),
                    "proposal is absent or timelocked; the execution refuses before submitting"
                );
                must_ok(
                    st.invoke(
                        governance.as_str(),
                        self.ctx.admin_alias(),
                        "execute_proposal",
                        &[kv("caller", admin.as_str()), kv("id", id.to_string())],
                    )?,
                    "execute_proposal",
                )?;
                let runtime = self.deployed_id("proxy-runtime")?;
                let asset = operation
                    .get("SetProxy")
                    .and_then(|a| a.get(0))
                    .ok_or_else(|| anyhow!("the scheduled operation lost its asset"))?;
                let registered = st.view(
                    runtime.as_str(),
                    self.ctx.admin_alias(),
                    "get_proxy",
                    &[kv("asset", asset.to_string())],
                )?;
                anyhow::ensure!(
                    registered == config,
                    "the runtime does not serve the exact configuration this proposal authorized"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the proposal executed and the runtime now serves the scheduled quorum configuration for this asset",
                    ),
                    target: Some(governance),
                    violations: Vec::new(),
                })
            }
            MOp::Refresh { tok } => {
                let runtime = self.deployed_id("proxy-runtime")?;
                let asset = json!({ "Stellar": self.token_id(*tok)?.as_str() }).to_string();
                let inv = st.invoke(
                    runtime.as_str(),
                    self.ctx.admin_alias(),
                    "refresh",
                    &[kv("asset", asset.clone())],
                )?;
                let accepted = inv.value.as_ref().and_then(|v| v.get("Accepted")).is_some();
                anyhow::ensure!(
                    inv.ok && accepted,
                    "a refresh did not return an Accepted price"
                );
                let cached = st.view(
                    runtime.as_str(),
                    self.ctx.admin_alias(),
                    "get_cached",
                    &[kv("asset", asset)],
                )?;
                anyhow::ensure!(
                    cached
                        .get("status")
                        .and_then(|s| s.get("Accepted"))
                        .is_some(),
                    "the cached price for this asset is not Accepted"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the asset's configured quorum resolved Accepted and the runtime caches an Accepted price",
                    ),
                    target: Some(runtime),
                    violations: Vec::new(),
                })
            }
            MOp::DeployAdapter => {
                let artifact = self
                    .artifacts
                    .get("adapter")
                    .ok_or_else(|| anyhow!("the adapter artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled adapter deploy lost its salt role"))?;
                let runtime = self.deployed_id("proxy-runtime")?;
                let governance = self.deployed_id("proxy-governance")?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[
                        kv("deployment_owner", governance.as_str()),
                        kv("parent_oracle", runtime.as_str()),
                        kv("decimals", self.cfg.adapter.decimals),
                        kv("resolution", self.cfg.adapter.resolution),
                        kv("base", json!({ "Other": MARKET_BASE }).to_string()),
                    ],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("an adapter deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed adapter is not the contract this gate bound"
                );
                self.deployed.insert("adapter".to_string(), id.clone());
                let config = st.view(id.as_str(), self.ctx.admin_alias(), "config", &[])?;
                anyhow::ensure!(
                    config.get("parent_oracle").and_then(Value::as_str) == Some(runtime.as_str())
                        && config.get("decimals").and_then(Value::as_u64)
                            == Some(u64::from(self.cfg.adapter.decimals))
                        && config.get("resolution").and_then(Value::as_u64)
                            == Some(u64::from(self.cfg.adapter.resolution))
                        && config.get("base") == Some(&json!({ "Other": MARKET_BASE })),
                    "the deployed adapter does not read back its immutable configuration"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the pool's oracle adapter deployed at its predicted id and reads back its immutable configuration",
                    ),
                    target: Some(id),
                    violations: Vec::new(),
                })
            }
            MOp::DeployComet => {
                let artifact = self
                    .artifacts
                    .get("comet")
                    .ok_or_else(|| anyhow!("the comet artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled comet deploy lost its salt role"))?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a comet deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed comet is not the contract this gate bound"
                );
                self.deployed.insert("comet".to_string(), id.clone());
                Ok(settled(
                    "comet deployed at its predicted id; initialization follows at its own coordinate",
                ))
            }
            MOp::InitComet => {
                let comet = self.deployed_id("comet")?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                for (tok, amount) in [(Tok::Blnd, 1_000 * SCALAR_7), (Tok::Usdc, 25 * SCALAR_7)] {
                    must_ok(
                        st.invoke(
                            self.token_id(tok)?.as_str(),
                            self.ctx.admin_alias(),
                            "mint",
                            &[kv("to", admin.as_str()), kv("amount", amount)],
                        )?,
                        "controller seed mint",
                    )?;
                }
                must_ok(
                    st.invoke(
                        comet.as_str(),
                        self.ctx.admin_alias(),
                        "init",
                        &[
                            kv("controller", admin.as_str()),
                            kv(
                                "tokens",
                                json!([
                                    self.token_id(Tok::Blnd)?.as_str(),
                                    self.token_id(Tok::Usdc)?.as_str()
                                ]),
                            ),
                            kv("weights", json!(["8000000", "2000000"])),
                            kv(
                                "balances",
                                json!([
                                    (1_000 * SCALAR_7).to_string(),
                                    (25 * SCALAR_7).to_string()
                                ]),
                            ),
                            kv("swap_fee", 30_000),
                        ],
                    )?,
                    "comet init",
                )?;
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the controller's seed balances were minted and the LP pool initialized with the fixture weights and fee",
                    ),
                    target: Some(comet),
                    violations: Vec::new(),
                })
            }
            MOp::DeployEmitter => {
                let artifact = self
                    .artifacts
                    .get("emitter")
                    .ok_or_else(|| anyhow!("the emitter artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled emitter deploy lost its salt role"))?;
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("an emitter deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed emitter is not the contract this gate bound"
                );
                self.deployed.insert("emitter".to_string(), id.clone());
                Ok(settled(
                    "emitter deployed at its predicted id; constructor metadata only, never called",
                ))
            }
            MOp::DeployBackstop => {
                let artifact = self
                    .artifacts
                    .get("backstop")
                    .ok_or_else(|| anyhow!("the backstop artifact pin is gone"))?;
                let deploy_salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled backstop deploy lost its salt role"))?;
                let predicted_factory = self.deployed_id("factory").or_else(|_| {
                    let factory_salt = salt(
                        self.nonce,
                        market_salt_role(&MOp::DeployFactory)
                            .ok_or_else(|| anyhow!("the factory salt role is gone"))?,
                        MARKET_SIDE,
                    );
                    let admin_net = self.ctx.admin_net();
                    let st_pred = Stellar::new(&admin_net, &self.exec, Policy::Market);
                    st_pred.predict_id(self.ctx.admin_alias(), &factory_salt)
                })?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                let frodo =
                    self.progress.identities.get(&Who::Frodo).ok_or_else(|| {
                        anyhow!("the backstop drop-list actor is not provisioned")
                    })?;
                let drop_list = json!([
                    [admin.as_str(), (10_000_000 * SCALAR_7).to_string()],
                    [frodo.as_str(), (30_000_000 * SCALAR_7).to_string()]
                ]);
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&deploy_salt),
                    &[
                        kv("backstop_token", self.deployed_id("comet")?.as_str()),
                        kv("emitter", self.deployed_id("emitter")?.as_str()),
                        kv("blnd_token", self.token_id(Tok::Blnd)?.as_str()),
                        kv("usdc_token", self.token_id(Tok::Usdc)?.as_str()),
                        kv("pool_factory", predicted_factory.as_str()),
                        kv("drop_list", drop_list),
                    ],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a backstop deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed backstop is not the contract this gate bound"
                );
                self.deployed.insert("backstop".to_string(), id.clone());
                Ok(settled(
                    "backstop deployed at its predicted id against the predicted factory address",
                ))
            }
            MOp::DeployFactory => {
                let artifact = self
                    .artifacts
                    .get("factory")
                    .ok_or_else(|| anyhow!("the factory artifact pin is gone"))?;
                let salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled factory deploy lost its salt role"))?;
                let meta = json!({
                    "backstop": self.deployed_id("backstop")?.as_str(),
                    "pool_hash": self
                        .artifacts
                        .get("pool")
                        .ok_or_else(|| anyhow!("the pool artifact pin is gone"))?,
                    "blnd_id": self.token_id(Tok::Blnd)?.as_str()
                });
                let id = st.deploy(
                    &hash_of(artifact)?,
                    self.ctx.admin_alias(),
                    Some(&salt),
                    &[kv("pool_init_meta", meta)],
                )?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a factory deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed factory is not the contract this gate bound"
                );
                self.deployed.insert("factory".to_string(), id.clone());
                Ok(settled(
                    "factory deployed at its predicted id with the backstop/factory cycle closed",
                ))
            }
            MOp::DeployPool => {
                let factory = self
                    .deployed_id("factory")
                    .map_err(|_| anyhow!("the factory deploy has not settled"))?;
                let salt = self
                    .slot_salt(slot)
                    .ok_or_else(|| anyhow!("a scheduled pool deploy lost its salt role"))?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                // Init-args adjudication (ADR-0005, doc/adr): the protected
                // market is a non-pair protected pool, so per the ADR-0002/
                // 0005/0007 freeze its oracle is this market's settled
                // adapter (`pool_oracle()` == the adapter), its backstop
                // take rate is 0 (interest stays with suppliers, matching
                // the pair-side frozen divergence), and max_positions and
                // min_collateral keep the harness-proven pair-side tuple
                // (6 and SCALAR_7) within the factory's bounds
                // (pool_factory.rs:76-90). The name is an honest non-pair
                // label; none of these values are live-discovered.
                let oracle = self.deployed_id("adapter")?;
                let pool_val = must_ok(
                    st.invoke(
                        factory.as_str(),
                        self.ctx.admin_alias(),
                        "deploy",
                        &[
                            kv("admin", admin.as_str()),
                            kv("name", "ProtectedMarket"),
                            kv("salt", salt),
                            kv("oracle", oracle.as_str()),
                            kv("backstop_take_rate", 0),
                            kv("max_positions", 6),
                            kv("min_collateral", SCALAR_7),
                        ],
                    )?,
                    "factory deploy pool",
                )?;
                let id = ContractId::from_str(
                    pool_val
                        .as_ref()
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("the factory deploy returned no address"))?,
                )
                .map_err(|_| anyhow!("the factory returned a malformed pool address"))?;
                let predicted = record
                    .predicted
                    .as_ref()
                    .ok_or_else(|| anyhow!("a pool deploy reached execution unpredicated"))?;
                anyhow::ensure!(
                    &id == predicted,
                    "the deployed pool is not the contract this gate bound"
                );
                let is_pool = st.view(
                    factory.as_str(),
                    self.ctx.admin_alias(),
                    "is_pool",
                    &[kv("pool_address", id.as_str())],
                )?;
                anyhow::ensure!(
                    is_pool == Value::Bool(true),
                    "the factory does not recognize the deployed pool"
                );
                let pool_admin = st.view(id.as_str(), self.ctx.admin_alias(), "get_admin", &[])?;
                anyhow::ensure!(
                    pool_admin.as_str() == Some(admin.as_str()),
                    "the pool admin did not read back as the operator"
                );
                let config = self.pool_config(&id)?;
                anyhow::ensure!(
                    config.get("oracle").and_then(Value::as_str) == Some(oracle.as_str())
                        && config.get("status").and_then(Value::as_u64) == Some(6)
                        && config.get("bstop_rate").and_then(Value::as_u64) == Some(0)
                        && config.get("max_positions").and_then(Value::as_u64) == Some(6)
                        && config.get("min_collateral").and_then(price_as_i128) == Some(SCALAR_7),
                    "the deployed pool's configuration is not the frozen protected-market shape"
                );
                self.deployed.insert("pool".to_string(), id.clone());
                Ok(settled(
                    "pool deployed through the factory at its predicted child id, with admin, oracle and configuration read back",
                ))
            }
            MOp::QueueReserve { tok } => {
                let pool = self.deployed_id("pool")?;
                let index = Tok::RESERVES
                    .iter()
                    .position(|t| *t == *tok)
                    .ok_or_else(|| anyhow!("a scheduled reserve is outside the reserve list"))?
                    as u32;
                let config = self.pool_config(&pool)?;
                anyhow::ensure!(
                    config.get("status").and_then(Value::as_u64) == Some(6),
                    "reserve initialization is setup-only law; the pool is not in setup status"
                );
                must_ok(
                    st.invoke(
                        pool.as_str(),
                        self.ctx.admin_alias(),
                        "queue_set_reserve",
                        &[
                            kv("asset", self.token_id(*tok)?.as_str()),
                            kv(
                                "metadata",
                                reserve_config(*tok, index, self.cfg.reserves.get(tok)),
                            ),
                        ],
                    )?,
                    "queue_set_reserve",
                )?;
                Ok(settled(
                    "the reserve initialization was queued during setup status, with the fixture's parameters and any operator override",
                ))
            }
            MOp::SetReserve { tok } => {
                let pool = self.deployed_id("pool")?;
                let index = Tok::RESERVES
                    .iter()
                    .position(|t| *t == *tok)
                    .ok_or_else(|| anyhow!("a scheduled reserve is outside the reserve list"))?;
                let config = self.pool_config(&pool)?;
                anyhow::ensure!(
                    config.get("status").and_then(Value::as_u64) == Some(6),
                    "reserve enablement is setup-only law; the pool is not in setup status"
                );
                let index_value = must_ok(
                    st.invoke(
                        pool.as_str(),
                        self.ctx.admin_alias(),
                        "set_reserve",
                        &[kv("asset", self.token_id(*tok)?.as_str())],
                    )?,
                    "set_reserve",
                )?;
                anyhow::ensure!(
                    index_value.as_ref().and_then(Value::as_u64) == Some(index as u64),
                    "the reserve did not receive the index this graph schedules"
                );
                self.require_reserve_matches_manifest(&pool, *tok, index as u32)?;
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the reserve was enabled during setup status, returned its scheduled index, and read back equal to the market config",
                    ),
                    target: Some(pool),
                    violations: Vec::new(),
                })
            }
            MOp::SeedBackstop => {
                let pool = self.deployed_id("pool")?;
                let comet = self
                    .deployed_id("comet")
                    .map_err(|_| anyhow!("the comet deploy has not settled"))?;
                let backstop = self
                    .deployed_id("backstop")
                    .map_err(|_| anyhow!("the backstop deploy has not settled"))?;
                let admin = self
                    .progress
                    .admin
                    .as_ref()
                    .ok_or_else(|| anyhow!("the operator is unresolved"))?;
                // Fixture sequence (setup.rs:19-27 and :79, mirrored live by
                // the pair side): mint both LP legs, join the LP pool, then
                // deposit LP into the pool's backstop. Under this engine's
                // signer law every leg is operator-signed: the legs are
                // minted to the operator under the mock-token admin law the
                // adapter-seed coordinate already uses, and the join and
                // deposit are the operator's own value movements. No token
                // approve is needed for the deposit: execute_deposit pulls
                // via a plain SEP-41 transfer under the deposit's own
                // `from.require_auth` (backstop/src/backstop/deposit.rs:19).
                for (tok, amount) in [
                    (Tok::Blnd, 110_000_000 * SCALAR_7),
                    (Tok::Usdc, 2_600_000 * SCALAR_7),
                ] {
                    must_ok(
                        st.invoke(
                            self.token_id(tok)?.as_str(),
                            self.ctx.admin_alias(),
                            "mint",
                            &[kv("to", admin.as_str()), kv("amount", amount)],
                        )?,
                        "lp leg mint",
                    )?;
                }
                must_ok(
                    st.invoke(
                        comet.as_str(),
                        self.ctx.admin_alias(),
                        "join_pool",
                        &[
                            kv("pool_amount_out", 10_000_000 * SCALAR_7),
                            kv(
                                "max_amounts_in",
                                json!([
                                    (110_000_000 * SCALAR_7).to_string(),
                                    (2_600_000 * SCALAR_7).to_string()
                                ]),
                            ),
                            kv("user", admin.as_str()),
                        ],
                    )?,
                    "comet join_pool",
                )?;
                must_ok(
                    st.invoke(
                        backstop.as_str(),
                        self.ctx.admin_alias(),
                        "deposit",
                        &[
                            kv("from", admin.as_str()),
                            kv("pool_address", pool.as_str()),
                            kv("amount", 50_000 * SCALAR_7),
                        ],
                    )?,
                    "backstop deposit",
                )?;
                let data = st.view(
                    backstop.as_str(),
                    self.ctx.admin_alias(),
                    "pool_data",
                    &[kv("pool", pool.as_str())],
                )?;
                anyhow::ensure!(
                    data.get("tokens").and_then(price_as_i128) == Some(50_000 * SCALAR_7),
                    "the pool's backstop did not read back the seeded balance"
                );
                Ok(settled(
                    "the backstop was seeded 50k LP from the operator's own join, and the pool's backstop balance read back",
                ))
            }
            MOp::SetStatus { to } => {
                let pool = self.deployed_id("pool")?;
                let config = self.pool_config(&pool)?;
                let observed = config.get("status").and_then(Value::as_u64);
                anyhow::ensure!(
                    *to == 3 && observed == Some(6),
                    "the graph knows exactly one lawful status transition, setup to on-ice; \
                     the pool's live status is not setup, so this coordinate refuses"
                );
                // Leaving setup seals reserve economics for the pool's life
                // (ADR 0008 restriction 3). Every scheduled reserve must read
                // back equal to the market config before that door closes.
                for (index, tok) in Tok::RESERVES.iter().enumerate() {
                    self.require_reserve_matches_manifest(&pool, *tok, index as u32)?;
                }
                must_ok(
                    st.invoke(
                        pool.as_str(),
                        self.ctx.admin_alias(),
                        "set_status",
                        &[kv("pool_status", *to)],
                    )?,
                    "set_status",
                )?;
                let after = self.pool_config(&pool)?;
                anyhow::ensure!(
                    after.get("status").and_then(Value::as_u64) == Some(u64::from(*to)),
                    "the pool status did not read back as set"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the pool status read back on-ice after the operator set it",
                    ),
                    target: Some(pool),
                    violations: Vec::new(),
                })
            }
            MOp::UpdateStatus => {
                let pool = self.deployed_id("pool")?;
                let config = self.pool_config(&pool)?;
                anyhow::ensure!(
                    config.get("status").and_then(Value::as_u64) == Some(3),
                    "status advance is lawful only from on-ice; the pool's live status is not \
                     on-ice, so this coordinate refuses"
                );
                let status = must_ok(
                    st.invoke(pool.as_str(), self.ctx.admin_alias(), "update_status", &[])?,
                    "update_status",
                )?;
                anyhow::ensure!(
                    status.as_ref().and_then(Value::as_u64) == Some(1),
                    "the status advance did not return active; the backstop threshold is not met"
                );
                let after = self.pool_config(&pool)?;
                anyhow::ensure!(
                    after.get("status").and_then(Value::as_u64) == Some(1),
                    "the pool status did not read back active"
                );
                Ok(MarketStepRecord {
                    phase: slot.phase,
                    action: Safe::literal(slot.action),
                    command: command.clone(),
                    outcome: Safe::literal(
                        "the pool status read back active; the market is live on testnet only",
                    ),
                    target: Some(pool),
                    violations: Vec::new(),
                })
            }
            MOp::Mint { index } => {
                let balance = self
                    .cfg
                    .balances
                    .get(*index)
                    .ok_or_else(|| anyhow!("a scheduled mint names no configured balance"))?;
                let to =
                    self.progress.identities.get(&balance.who).ok_or_else(|| {
                        anyhow!("the mint recipient's identity is not provisioned")
                    })?;
                must_ok(
                    st.invoke(
                        self.token_id(balance.token)?.as_str(),
                        self.ctx.admin_alias(),
                        "mint",
                        &[kv("to", to.as_str()), kv("amount", balance.amount)],
                    )?,
                    "mint",
                )?;
                Ok(settled(
                    "the configured playable balance was minted to a run-local actor",
                ))
            }
            MOp::SeedPositions => Ok(settled(
                "position seeding deliberately not performed: the frozen plan mints only stable \
                 to run-local actors, so no lawful submitter holds the fixture's whale \
                 collateral shape; positions are opened through market play (supply stable, \
                 borrow against it) instead of pre-seeded — adjudicated, not silently skipped",
            )),
            MOp::Publish => {
                // A publication already on disk that reproduces this run's
                // settled facts is complete, and honoring that proof needs no
                // observation at all. It is checked before the manifest is
                // assembled so a resume finishing an interrupted publication
                // never needs the chain. The run directory is single-writer
                // under the market run lock.
                if let Some(published) = self.published_manifest()? {
                    self.manifest = Some(published);
                    return Ok(settled(
                        "the market manifest was already published; the published bytes reproduce this run's state",
                    ));
                }
                let manifest = self.build_manifest()?;
                let text =
                    toml::to_string_pretty(&manifest).context("rendering the market manifest")?;
                let final_path = self.journal.out().join(MARKET_MANIFEST_FILE);
                let tmp = self
                    .journal
                    .out()
                    .join(format!("{MARKET_MANIFEST_FILE}.{}.tmp", std::process::id()));
                use std::io::Write as _;
                {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&tmp)
                        .context("creating the market manifest staging file")?;
                    file.write_all(text.as_bytes())
                        .context("writing the market manifest")?;
                    file.sync_all().context("syncing the market manifest")?;
                }
                std::fs::rename(&tmp, &final_path).context("publishing the market manifest")?;
                sync_dir(&self.journal.out());
                let published_text = std::fs::read_to_string(&final_path)
                    .context("reading back the published market manifest")?;
                anyhow::ensure!(
                    published_text == text,
                    "the published manifest does not reproduce this run's state"
                );
                let published = MarketManifest::load(self.journal.out().as_path())
                    .context("the published market manifest is unreadable")?;
                self.manifest = Some(published);
                Ok(settled(
                    "the market manifest was published atomically and reads back complete",
                ))
            }
        }
    }

    /// The artifact digest a scheduled mutation is bound to, when it uses one.
    fn slot_artifact_digest(&self, slot: &MSlot) -> Option<Sha256Hex> {
        let role = match &slot.op {
            MOp::Upload { role } => *role,
            MOp::DeployToken { .. } => "token",
            MOp::DeploySource { .. } => "source",
            MOp::DeployGovernance => "proxy-governance",
            MOp::DeployRuntime => "proxy-runtime",
            MOp::DeployAdapter => "adapter",
            MOp::DeployComet | MOp::InitComet => "comet",
            MOp::DeployEmitter => "emitter",
            MOp::DeployBackstop => "backstop",
            MOp::DeployFactory => "factory",
            MOp::DeployPool => "pool",
            _ => return None,
        };
        self.artifacts.get(role).cloned()
    }

    /// Completion. Publication itself is the journal coordinate at phase 12
    /// slot 1; this path only ever reports `Done` from a manifest that is
    /// already published and readable in this run directory — otherwise it
    /// refuses terminally rather than emitting an unproven `Done`.
    fn publish(&mut self) -> anyhow::Result<DeployEvent> {
        if let Some(manifest) = self.manifest.clone() {
            self.terminal = Some(DeployEvent::Done(manifest));
            return Ok(self.terminal.clone().unwrap());
        }
        let settled = self.publish_proof_settled()?;
        match MarketManifest::load(self.journal.out().as_path()) {
            Ok(manifest) if settled => {
                self.manifest = Some(manifest.clone());
                self.terminal = Some(DeployEvent::Done(manifest));
            }
            _ => {
                self.terminal = Some(DeployEvent::Failed(Safe::literal(
                    "market completion is unproven: no published manifest is reconcilable with this run's journal",
                )));
            }
        }
        Ok(self.terminal.clone().unwrap())
    }

    /// Whether the journal's phase 12 slot 1 result attests to the
    /// publication, i.e. the manifest coordinate completed and settled.
    fn publish_proof_settled(&self) -> anyhow::Result<bool> {
        Ok(self
            .journal
            .result(12, 1)?
            .is_some_and(|result| result.custody.tool_error.is_none()))
    }

    /// Execute one confirmed mutation. The custody intent is written before
    /// the submission is attempted and settled only on a definite, verified
    /// outcome, so a submission whose fate is unknown is reported as
    /// unresolved custody and never replayed. An already-settled coordinate is
    /// honored as done, and an already-open intent is reported, not re-run.
    fn execute_gate(&mut self, gate: &Gate) -> anyhow::Result<()> {
        if self.journal.result(gate.phase, gate.slot)?.is_some() {
            return Ok(());
        }
        if let Some(open) = self.journal.read_intent(gate.phase, gate.slot)? {
            return if self.intent_matches_gate(&open, gate) {
                Err(anyhow!(
                    "a market custody intent for this mutation is already open; \
                     it must be reconciled, not replayed"
                ))
            } else {
                Err(anyhow!(
                    "the open custody intent at this coordinate is not the mutation \
                     this token bound; the journal cannot say which submission is in flight"
                ))
            };
        }
        let slot = market_schedule()
            .into_iter()
            .find(|s| s.phase == gate.phase && s.slot == gate.slot)
            .ok_or_else(|| anyhow!("the confirmed coordinate is not part of the frozen graph"))?;
        if custody_payload_digest(&gate.record)? != gate.record.payload_digest {
            return Err(anyhow!(
                "the confirmed mutation no longer matches the payload its token bound"
            ));
        }
        self.ctx.assert_testnet()?;
        Stellar::new(&self.ctx.actor_net(), &self.exec, Policy::Market).assert_rpc_testnet()?;
        self.journal.begin(gate.phase, gate.slot, &gate.record)?;
        match self.run_slot(&slot, &gate.record) {
            Ok(step) => {
                self.journal.finish(
                    gate.phase,
                    gate.slot,
                    &MarketPendingResult {
                        custody: gate.record.clone(),
                        step,
                    },
                )?;
                Ok(())
            }
            Err(e) => {
                if self.journal.result(gate.phase, gate.slot)?.is_some() {
                    return Ok(());
                }
                Err(e)
            }
        }
    }

    /// Whether an open intent is the mutation a token bound. A mismatch means
    /// the journal cannot say which submission is in flight.
    fn intent_matches_gate(&self, open: &MarketCustodyRecord, gate: &Gate) -> bool {
        open.phase == gate.record.phase
            && open.kind == gate.record.kind
            && open.payload_digest == gate.record.payload_digest
            && open.signer == gate.record.signer
            && open.target == gate.record.target
    }

    fn unresolved(&mut self, phase: u32, slot: u32, note: Option<Safe>) -> DeployEvent {
        let gate = self.gate.take();
        if gate
            .as_ref()
            .is_some_and(|g| g.phase == phase && g.slot == slot)
        {
            self.gate = None;
        }
        let note = note.unwrap_or_else(|| {
            Safe::literal(
                "unresolved market custody: a potentially submitted mutation ended without a definite outcome; nothing was replayed",
            )
        });
        self.unresolved = Some(note.clone());
        DeployEvent::Custody {
            unresolved: true,
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stellar::fake::FakeExecutor;
    use crate::stellar::Net;

    #[test]
    fn salts_are_32_bytes_and_distinct() {
        let a = salt(1, 40, 'a');
        let b = salt(1, 40, 'b');
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_ne!(salt(1, 40, 'a'), salt(2, 40, 'a'));
    }

    #[test]
    fn reserve_config_matches_fixture_and_applies_overrides() {
        let stable = reserve_config(Tok::Stable, 0, None);
        assert_eq!(stable["decimals"], 6);
        assert_eq!(stable["c_factor"], 9_000_000);
        let weth = reserve_config(
            Tok::Weth,
            2,
            Some(&ReserveOverride {
                c_factor: Some(1),
                r_two: Some(7_000_000),
                reactivity: Some(5),
                ..Default::default()
            }),
        );
        assert_eq!(weth["c_factor"], 1);
        assert_eq!(weth["supply_cap"], I128_MAX);
        assert_eq!(weth["r_two"], 7_000_000);
        assert_eq!(weth["reactivity"], 5);
        assert_eq!(
            weth["r_base"], 100_000,
            "unset rate fields keep the fixture value"
        );
    }

    #[test]
    fn market_deploy_refuses_reserves_with_fixture_defaults() {
        let out =
            std::env::temp_dir().join(format!("abtest-explicit-reserves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("scratch directory");
        let (_, full) = fixture_market_config(&out);
        full.require_explicit_reserves()
            .expect("a fully explicit config passes the gate");
        let mut partial = full.clone();
        partial.reserves.get_mut(&Tok::Xlm).unwrap().supply_cap = None;
        partial.reserves.get_mut(&Tok::Xlm).unwrap().r_three = None;
        let err = partial
            .require_explicit_reserves()
            .expect_err("a reserve inheriting a cap or rate must refuse")
            .to_string();
        assert!(
            err.contains("'xlm'") && err.contains("supply_cap, r_three"),
            "{err}"
        );
        let mut absent = full;
        absent.reserves.remove(&Tok::Weth);
        assert!(absent.require_explicit_reserves().is_err());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn request_json_uses_contract_addresses() {
        let c = Contracts {
            stable: "CSTABLE".into(),
            ..Default::default()
        };
        let s = request_json(&[Req::new(req::BORROW, Tok::Stable, 5)], &c);
        assert_eq!(
            s,
            r#"[{"address":"CSTABLE","amount":"5","request_type":4}]"#
        );
    }

    #[test]
    fn identities_reuse_existing_keys() {
        let n = Net {
            rpc_url: "u".into(),
            passphrase: "p".into(),
            config_dir: "/c".into(),
            headers: Vec::new(),
        };
        // Checksum-valid StrKey fixtures: `keys_ensure_funded` now hard-rejects
        // a successful lookup whose stdout is not a valid G-address, so the
        // reuse path must present real ones (checksum-valid zero-key vectors).
        let fx = FakeExecutor::new(vec![
            FakeExecutor::ok(
                "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
                "",
            ),
            FakeExecutor::fail("not found"),
            FakeExecutor::ok("", "funded"),
            FakeExecutor::ok(
                "GAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHV4",
                "",
            ),
            FakeExecutor::ok(
                "GABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABVCX",
                "",
            ),
            FakeExecutor::ok(
                "GABQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABEQO",
                "",
            ),
        ]);
        let st = Stellar::new(&n, &fx, Policy::Pair);
        let ids = ensure_identities(&st).unwrap();
        assert_eq!(
            ids[&Who::Frodo],
            "GAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHV4"
        );
        let calls = fx.calls.borrow();
        assert!(
            calls[2].contains(&"generate".to_string()) && calls[2].contains(&"--fund".to_string())
        );
        assert!(
            !calls[1].contains(&"--rpc-url".to_string()),
            "public-key takes no network flags"
        );
        assert!(calls[1].contains(&"--config-dir".to_string()));
    }

    /// Every deterministic market deployment must consume a distinct create2
    /// salt, and no market salt may collide with a pair-graph salt: two
    /// contracts created from one salt land at one address, silently.
    #[test]
    fn market_salt_table_is_collision_free() {
        let roles: Vec<u32> = market_schedule()
            .iter()
            .filter_map(|s| market_salt_role(&s.op))
            .collect();
        assert!(!roles.is_empty(), "the graph must deploy something");
        let unique: std::collections::BTreeSet<u32> = roles.iter().copied().collect();
        assert_eq!(
            unique.len(),
            roles.len(),
            "two deterministic deployments share a salt role"
        );
        let triples = market_salt_triples(0x0123_4567_89ab_cdef);
        assert_eq!(
            triples.len(),
            roles.len(),
            "the salt table must cover exactly the deploying operations"
        );
        let unique_triples: std::collections::BTreeSet<(u64, u32, char)> =
            triples.iter().copied().collect();
        assert_eq!(unique_triples.len(), triples.len());
        for (nonce, role, side) in &triples {
            assert_eq!(*side, MARKET_SIDE);
            for pair_role in [10u32, 20, 30, 40, 41, 50, 60] {
                assert_ne!(
                    salt(*nonce, *role, MARKET_SIDE),
                    salt(*nonce, pair_role, 'a'),
                    "market salt collides with a pair salt"
                );
                assert_ne!(
                    salt(*nonce, *role, MARKET_SIDE),
                    salt(*nonce, pair_role, 'b'),
                    "market salt collides with a pair salt"
                );
            }
        }
    }

    /// The no-replay law depends on every scheduled mutation living at a
    /// journal coordinate the resume scan actually inspects. A coordinate
    /// outside that range would let an ambiguous submission be replayed
    /// without ever surfacing as unresolved custody, so it is refused here
    /// rather than discovered on a live market.
    #[test]
    fn every_scheduled_coordinate_is_scanned_and_unique() {
        let schedule = market_schedule();
        assert!(!schedule.is_empty());
        for slot in &schedule {
            assert!(
                market_coordinate_is_scanned(slot.phase, slot.slot),
                "coordinate ({}, {}) would be invisible to the no-replay scan",
                slot.phase,
                slot.slot
            );
        }
        let coordinates: std::collections::BTreeSet<(u32, u32)> =
            schedule.iter().map(|s| (s.phase, s.slot)).collect();
        assert_eq!(
            coordinates.len(),
            schedule.len(),
            "two mutations share one journal coordinate"
        );
    }

    /// The graph's ordering laws. Uploading must precede deploying, the
    /// adapter must exist before the pool that reads price through it, and
    /// the manifest must be published last — after every battery — because a
    /// published manifest is a claim that the market is safe to use.
    #[test]
    fn the_graph_uploads_before_deploying_and_publishes_last() {
        let schedule = market_schedule();
        let position = |wanted: &dyn Fn(&MOp) -> bool| {
            schedule
                .iter()
                .position(|s| wanted(&s.op))
                .unwrap_or(usize::MAX)
        };
        let uploads = position(&|op| matches!(op, MOp::Upload { .. }));
        let deploys = position(&|op| matches!(op, MOp::DeployToken { .. } | MOp::DeployPool));
        assert!(
            uploads < deploys,
            "the graph deploys before any artifact was uploaded"
        );
        let adapter = position(&|op| matches!(op, MOp::DeployAdapter));
        let pool = position(&|op| matches!(op, MOp::DeployPool));
        assert!(
            adapter < pool,
            "the pool is deployed before the adapter that supplies its price"
        );
        let publish = position(&|op| matches!(op, MOp::Publish));
        assert_eq!(
            publish,
            schedule.len() - 1,
            "the manifest must be the last mutation of the graph"
        );
        let cutover = position(&|op| matches!(op, MOp::DeployGovernance));
        assert!(
            cutover < adapter,
            "the adapter was deployed before ownership was established"
        );
    }

    /// A frozen deployment plan is written once, atomically, and never
    /// rewritten. If a halted run could re-plan itself, the record of what
    /// was authorized would no longer describe what was done.
    #[test]
    fn a_frozen_plan_is_written_once_and_never_rewritten() {
        let dir = std::env::temp_dir().join(format!("abtest-plan-law-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let plan = DeploymentPlan {
            schema: 1,
            config_digest: "a".repeat(64),
            nonce: "7".to_string(),
            network_label: MARKET_NETWORK_LABEL.to_string(),
            passphrase: MARKET_TESTNET_PASSPHRASE.to_string(),
            admin: String::new(),
            artifacts: BTreeMap::new(),
            salt_roles: vec![221, 222, 223],
            slots: vec![(1, 1), (2, 1)],
        };
        write_deployment_plan(&dir, &plan).expect("the plan is written atomically");
        assert_eq!(
            read_deployment_plan(&dir).expect("readable").as_ref(),
            Some(&plan),
            "the plan a resume reads must be the plan that was frozen"
        );
        let mut drifted = plan.clone();
        drifted.nonce = "8".to_string();
        assert!(
            write_deployment_plan(&dir, &drifted).is_err(),
            "a frozen plan must be abandoned, never quietly re-planned"
        );
        assert_eq!(
            read_deployment_plan(&dir).expect("readable").as_ref(),
            Some(&plan),
            "the rejected re-plan must not have touched the frozen plan"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The crash case the upload arm exists for: an intent was written, the
    /// process died before anything was submitted, and the pinned artifact is
    /// still exactly what it was. Local evidence is identical whether or not
    /// the upload ever landed, which is why an open upload intent is never
    /// settled on inspection — a coordinate marked complete is never uploaded,
    /// and the graph would deploy contracts whose code the chain never got.
    #[test]
    fn an_intact_artifact_is_not_evidence_that_an_upload_happened() {
        let bytes = b"pinned artifact bytes";
        let declared: Sha256Hex = sha256_hex(bytes).try_into().unwrap();
        // Nothing was submitted and the bytes never changed. A predicate that
        // treated this match as settlement would close the coordinate falsely.
        assert!(!MarketDeployer::artifact_drifts_from_intent(
            bytes, &declared
        ));
        // Drift is a different emergency, and it is detected.
        assert!(MarketDeployer::artifact_drifts_from_intent(
            b"swapped bytes",
            &declared
        ));
    }

    fn hex_str(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(text, "{b:02x}");
        }
        text
    }

    /// Keccak256 known answers. The "abc" vector is the ORIGINAL Keccak
    /// digest (4e03657a...12d6c45), not NIST SHA3-256 (...0724ddd): the
    /// pool factory's on-chain keccak256 uses the pre-standardization pad,
    /// and a SHA3 implementation would silently mispredict every child id.
    #[test]
    fn keccak256_known_answers_gate_the_helper() {
        // Primitive isolation probes first: the CRC-16/XMODEM check value
        // and the all-zero strkey vectors pin crc16_xmodem, base32 and the
        // little-endian checksum order before any derivation is trusted.
        assert_eq!(crc16_xmodem(b"123456789"), 0x31C3);
        assert_eq!(
            strkey_encode(STRKEY_G, &[0u8; 32]),
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"
        );
        assert_eq!(
            strkey_encode(STRKEY_C, &[0u8; 32]),
            "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4"
        );
        assert_eq!(
            hex_str(&keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        assert_eq!(
            hex_str(&keccak256(b"abc")),
            "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
        );
    }

    /// The frozen testnet network id, bound from the passphrase constant so
    /// a wrong network cannot be quietly substituted into any preimage.
    #[test]
    fn the_market_network_id_is_the_frozen_testnet_id() {
        assert_eq!(
            hex_str(&market_network_id()),
            "cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472"
        );
    }

    /// Strkey round-trips and checksum discrimination, including the
    /// harness factory contract id used by the derivation vectors below.
    #[test]
    fn strkeys_round_trip_and_reject_corruption() {
        let operator = "GCNQ4ZLK45KOYNIQLIYI7GT6TYWGK2OZEVZEY7A3W5L7QSPBNCBWSJAQ";
        let (version, payload) = strkey_decode(operator).expect("operator strkey");
        assert_eq!(version, STRKEY_G);
        assert_eq!(payload.len(), 32);
        assert_eq!(strkey_encode(STRKEY_G, &payload), operator);
        let harness_factory = "face000000000000000000000000000000000000000000000000000000000001";
        let factory = hex32(harness_factory).expect("harness factory bytes");
        assert_eq!(
            strkey_encode(STRKEY_C, &factory),
            "CD5M4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACW53"
        );
        let (v, back) = strkey_decode("CD5M4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACW53")
            .expect("harness factory strkey");
        assert_eq!((v, back.as_slice()), (STRKEY_C, factory.as_slice()));
        // A flipped payload byte must fail the checksum, not decode.
        let mut broken = operator.as_bytes().to_vec();
        broken[7] = if broken[7] == b'2' { b'3' } else { b'2' };
        let broken = String::from_utf8(broken).expect("ascii");
        assert_ne!(broken, operator);
        if let Some((_, p)) = strkey_decode(&broken) {
            assert_ne!(p, payload, "a corrupted strkey must not decode valid");
        }
    }

    /// End-to-end pool-child proof, both pool-proof vectors: the child salt
    /// is keccak256(salt || admin56), and the predicted id is
    /// sha256(0x00000008 || XDR(ContractId, network, Contract(factory),
    /// child-salt)) — never the raw deploy salt.
    #[test]
    fn factory_child_prediction_reproduces_the_pinned_pool_vectors() {
        let admin = "GCNQ4ZLK45KOYNIQLIYI7GT6TYWGK2OZEVZEY7A3W5L7QSPBNCBWSJAQ";
        // Sample vector (pool-proof [sample]).
        let sample_salt = "112233445566778899aabbccddeeff000102030405060708090a0b0c0d0e0f10";
        assert_eq!(
            hex_str(&pool_factory_child_salt(sample_salt, admin).expect("sample child salt")),
            "317b77f579a309a7e1e9ae862848be7de63812de9c710e92fd585595ddb116c7"
        );
        assert_eq!(
            predict_pool_child(
                "CD5M4AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACW53",
                sample_salt,
                admin
            )
            .expect("sample prediction"),
            "CBRVKZLFWFURNSJOU5MI6EQJNK72STF5CV4IMJXB7Y7V63BVHMEAJKCY"
        );
        // Runtime-fixture vector (pool-proof [runtime]).
        let runtime_salt = "d8".repeat(32);
        assert_eq!(
            hex_str(&pool_factory_child_salt(&runtime_salt, admin).expect("runtime child salt")),
            "e0043250bd4781585e624ee079b1e8d748b4158705d5b0fbeb4414d90d5237a5"
        );
        let runtime_factory = hex32(&"f8".repeat(32)).expect("runtime factory");
        assert_eq!(
            predict_contract_child(
                &market_network_id(),
                &runtime_factory,
                &pool_factory_child_salt(&runtime_salt, admin).expect("runtime child salt")
            ),
            hex32("433c3ae177e4eda53d233ae0d7969d7b362f86d2b7afcf12bf35ab2f9f0fa356")
                .expect("runtime child id")
        );
        assert_eq!(
            strkey_encode(
                STRKEY_C,
                &predict_contract_child(
                    &market_network_id(),
                    &runtime_factory,
                    &pool_factory_child_salt(&runtime_salt, admin).expect("runtime child salt")
                )
            ),
            "CBBTYOXBO7SO3JJ5EM5OBV4WTV5TML4G2K327TYSX422WL47B6RVMJRI"
        );
    }

    /// The settlement trust law for every deployment decision: a
    /// fabricated, corrupt, mismatched, or symlinked result is refused as a
    /// question, never honored as a completed slot, and the action gate
    /// stays closed over it. A legitimate settlement remains readable so a
    /// real resume still skips its completed coordinate.
    #[test]
    fn fabricated_settlements_cannot_complete_a_deploy_coordinate() {
        let out =
            std::env::temp_dir().join(format!("abtest-deploy-fabricated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("scratch directory");
        let journal = MarketJournal::open(&out);
        let signer: AccountId = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"
            .parse()
            .expect("fixture signer");
        let target: ContractId = "CBEF4DWEOEK75LWNGJNROJXVMNBC2DISKHGG63T4NYHAE53OTZRGF3AT"
            .parse()
            .expect("fixture target");
        let mut custody = MarketCustodyRecord {
            phase: 1,
            kind: MarketMutationKind::Deploy,
            action: Safe::literal("deploy-pool"),
            network: MarketTestnet::Testnet,
            signer,
            payload_digest: Sha256Hex::default(),
            target: Some(target),
            artifact_digest: None,
            predicted: None,
            pre_state: Vec::new(),
            tool_error: None,
        };
        custody.payload_digest = custody_payload_digest(&custody).expect("fixture digest");
        assert!(journal
            .read_intent(1, 0)
            .expect("absent intent is readable")
            .is_none());
        assert!(journal
            .result(1, 0)
            .expect("absent result is readable")
            .is_none());
        begin_market_pending(&out, 1, 0, &custody).expect("reserve fixture intent");
        let step = MarketStepRecord {
            phase: 1,
            action: custody.action.clone(),
            command: Safe::literal("test settlement"),
            outcome: Safe::literal("settled"),
            target: custody.target.clone(),
            violations: Vec::new(),
        };
        journal
            .finish(
                1,
                0,
                &MarketPendingResult {
                    custody: custody.clone(),
                    step: step.clone(),
                },
            )
            .expect("settle fixture");
        assert!(
            journal
                .result(1, 0)
                .expect("a legitimate settlement must read as settled")
                .is_some(),
            "a valid settlement still lets the resume skip its completed slot"
        );
        assert!(
            crate::market_status::journal_clear_for_action(&out),
            "a legitimate settlement clears the action gate so a real run can proceed"
        );
        let coordinate = out
            .join("market-pending")
            .join("001-00")
            .join("result.json");
        std::fs::write(&coordinate, b"{").expect("corrupt settlement");
        assert!(
            journal.result(1, 0).is_err(),
            "a malformed settlement is a custody question, never a completed slot"
        );
        assert!(!crate::market_status::journal_clear_for_action(&out));
        std::fs::write(
            &coordinate,
            serde_json::to_vec(&MarketPendingResult {
                custody: MarketCustodyRecord {
                    action: Safe::literal("forged-pool"),
                    ..custody.clone()
                },
                step: step.clone(),
            })
            .expect("forged bytes"),
        )
        .expect("plant forged settlement");
        assert!(
            journal.result(1, 0).is_err(),
            "a settlement carrying another custody record cannot pass as this coordinate's"
        );
        assert!(!crate::market_status::journal_clear_for_action(&out));
        std::fs::remove_file(&coordinate).expect("remove planted settlement");
        std::os::unix::fs::symlink(&out, &coordinate).expect("plant symlink settlement");
        assert!(
            journal.result(1, 0).is_err(),
            "a symlinked settlement must never be followed into completion"
        );
        assert!(!crate::market_status::journal_clear_for_action(&out));
        let first = &market_schedule()[0];
        let (phase, slot) = (first.phase, first.slot);
        let planted = out
            .join("market-pending")
            .join(format!("{phase:03}-{slot:02}"));
        std::fs::create_dir_all(&planted).expect("plant coordinate directory");
        let orphan = MarketCustodyRecord {
            phase,
            ..custody.clone()
        };
        let orphan = MarketCustodyRecord {
            payload_digest: custody_payload_digest(&orphan).expect("rebind orphan digest"),
            ..orphan
        };
        std::fs::write(
            planted.join("result.json"),
            serde_json::to_vec(&MarketPendingResult {
                custody: orphan.clone(),
                step: MarketStepRecord {
                    phase,
                    action: orphan.action.clone(),
                    command: Safe::literal("planted"),
                    outcome: Safe::literal("settled"),
                    target: orphan.target.clone(),
                    violations: Vec::new(),
                },
            })
            .expect("planted bytes"),
        )
        .expect("plant settlement without an intent");
        assert!(
            journal.result(phase, slot).is_err(),
            "a settlement without its pre-send intent can never complete a scheduled slot"
        );
        assert!(
            !crate::market_status::journal_clear_for_action(&out),
            "a journal holding an orphaned settlement must never clear for a new send"
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Target law and producer-to-reader law for the seed phase: a seed
    /// slot has no frozen target while nothing is deployed, both seed slots
    /// bind exactly the deployed pool once one is recorded, a settlement
    /// that abandons the pool for another target is refused before it is
    /// written, and a real settlement the producer writes at the scheduled
    /// seed coordinates is accepted by the strict journal reader and clears
    /// the action gate for a legitimate resume.
    #[test]
    fn seed_slots_bind_the_pool_and_real_settlements_pass_the_strict_reader() {
        let out =
            std::env::temp_dir().join(format!("abtest-deploy-seed-custody-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("scratch directory");
        let schedule = market_schedule();
        let seed_slots: Vec<&MSlot> = schedule
            .iter()
            .filter(|s| matches!(s.op, MOp::SeedBackstop | MOp::SeedPositions))
            .collect();
        assert_eq!(
            seed_slots.len(),
            2,
            "the frozen graph schedules both seed operations"
        );
        // Target law: the engine may not freeze a target for a seed slot
        // before the pool deployment exists, and binds both seed slots to
        // the deployed pool once the deployment is recorded.
        let cfg_dir = out.join("cfg");
        std::fs::create_dir_all(&cfg_dir).expect("fixture directory");
        let mut cfg_text = String::from(
            "[network]\npassphrase = \"Test SDF Network ; September 2015\"\nlabel = \"testnet\"\n\n",
        );
        for role in crate::manifest::MARKET_ARTIFACT_ROLES {
            let artifact = cfg_dir.join(format!("{role}.wasm"));
            std::fs::write(&artifact, b"wasm").expect("fixture artifact");
            cfg_text.push_str(&format!(
                "[artifacts.{role}]\npath = \"{}\"\nsha256 = \"{}\"\n\n",
                artifact.display(),
                "0".repeat(64)
            ));
        }
        cfg_text.push_str(
            "[oracle]\nsources = 3\nmin_sources = 3\nmax_age_secs = 3600\nmax_clock_drift_secs = 60\nbase = \"USD\"\ndecimals = 7\nresolution = 300\n\n[adapter]\ndecimals = 7\nresolution = 1\nbase = \"USD\"\n\n",
        );
        cfg_text.push_str(FIXTURE_RESERVES_TOML);
        cfg_text
            .push_str("[[balances]]\nwho = \"frodo\"\ntoken = \"stable\"\namount = 100000000000\n");
        let cfg_file = cfg_dir.join("config.toml");
        std::fs::write(&cfg_file, &cfg_text).expect("fixture config");
        let cfg = crate::manifest::MarketConfig::load(&cfg_file)
            .expect("fixture config must load through the strict parser");
        for dir in ["admin", "actors", "xdg"] {
            std::fs::create_dir_all(out.join(dir)).expect("fixture directories");
        }
        let ctx = MarketExecutionContext::new(
            "https://soroban-testnet.stellar.org".to_string(),
            Vec::new(),
            "market-admin".to_string(),
            out.join("admin"),
            out.join("actors"),
            out.join("absent-stellar"),
            out.join("xdg"),
            PathBuf::from("/"),
        )
        .expect("fixture market context");
        let mut d = MarketDeployer::new(cfg, ctx, MarketJournal::open(&out))
            .expect("a closed market constructs with its gate failed, never with authority");
        for slot in &seed_slots {
            assert!(
                d.slot_target(slot).is_none(),
                "a seed slot cannot freeze a target while nothing is deployed"
            );
        }
        let pool: ContractId = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4"
            .parse()
            .expect("fixture pool");
        d.deployed.insert("pool".to_string(), pool.clone());
        for slot in &seed_slots {
            assert_eq!(
                d.slot_target(slot).as_ref(),
                Some(&pool),
                "the frozen seed slot target must be the deployed pool"
            );
        }
        // Producer-to-reader law at the real scheduled seed coordinates:
        // the strict reader accepts exactly what the producer legitimately
        // settles, and a target-abandoning settlement never reaches disk.
        let journal = MarketJournal::open(&out);
        let signer: AccountId = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"
            .parse()
            .expect("fixture signer");
        let wrong: ContractId = "CBBTYOXBO7SO3JJ5EM5OBV4WTV5TML4G2K327TYSX422WL47B6RVMJRI"
            .parse()
            .expect("fixture witness target");
        for slot in &seed_slots {
            let (phase, numbered) = (slot.phase, slot.slot);
            let mut custody = MarketCustodyRecord {
                phase,
                kind: slot.kind,
                action: Safe::literal(slot.action),
                network: MarketTestnet::Testnet,
                signer: signer.clone(),
                payload_digest: Sha256Hex::default(),
                target: Some(pool.clone()),
                artifact_digest: None,
                predicted: Some(pool.clone()),
                pre_state: vec![Safe::literal("pool status = 1")],
                tool_error: None,
            };
            custody.payload_digest = custody_payload_digest(&custody).expect("fixture digest");
            begin_market_pending(&out, phase, numbered, &custody)
                .expect("seed intent reserves at its scheduled coordinate");
            let mut step = MarketStepRecord {
                phase,
                action: custody.action.clone(),
                command: Safe::literal("test settlement"),
                outcome: Safe::literal("settled"),
                target: Some(wrong.clone()),
                violations: Vec::new(),
            };
            assert!(
                journal
                    .finish(
                        phase,
                        numbered,
                        &MarketPendingResult {
                            custody: custody.clone(),
                            step: step.clone(),
                        },
                    )
                    .is_err(),
                "a seed settlement that abandons the pool for another target cannot be written"
            );
            assert!(
                journal
                    .result(phase, numbered)
                    .expect("an open coordinate is readable")
                    .is_none(),
                "a rejected settlement must leave the coordinate unresolved"
            );
            step.target = Some(pool.clone());
            journal
                .finish(
                    phase,
                    numbered,
                    &MarketPendingResult {
                        custody: custody.clone(),
                        step,
                    },
                )
                .expect("the pool-bound settlement the producer writes must be accepted");
            assert!(
                journal
                    .result(phase, numbered)
                    .expect("a settled coordinate is readable")
                    .is_some(),
                "the strict journal reader must accept the settlement the producer actually wrote"
            );
        }
        assert!(
            crate::market_status::journal_clear_for_action(&out),
            "settled seed coordinates must clear the action gate so a legitimate resume proceeds"
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A contract address per index, produced by the tool's own strkey encoder
    /// so no fixture carries a byte string that could drift out of validity.
    /// Index 0 is always another run's address; the roles a fixture run deploys
    /// take the indices after it, so no two roles can share an address and a
    /// manifest naming the pool cannot be naming some other role by accident.
    fn fixture_contract(index: u8) -> ContractId {
        strkey_encode(STRKEY_C, &[index; 32])
            .parse()
            .expect("checksum-valid fixture contract")
    }

    /// The same for operator-shaped addresses: index 0 is a fixture run's
    /// operator, and the indices after it are its run-local actors.
    fn fixture_actor(index: u8) -> AccountId {
        strkey_encode(STRKEY_G, &[index; 32])
            .parse()
            .expect("checksum-valid fixture actor")
    }

    /// A `stellar` stand-in. It answers the read-only calls a market run makes
    /// before it publishes (the CLI version probe, the RPC network identity,
    /// the operator lookup, and the ledger stamp a first publication needs),
    /// refuses every other command, and appends each invocation to a log, so a
    /// test proves a recovery issued no mutation instead of assuming none was.
    fn read_only_stellar(dir: &Path, admin: &AccountId, other: &AccountId, ledger: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let log = dir.join("stellar-calls.log");
        let script = dir.join("stellar");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1 $2\" in\n'network info') printf '{{\"passphrase\":\"{}\"}}\\n'; exit 0 ;;\n'keys public-key') case \"$3\" in market-admin) printf '{}\\n' ;; *) printf '{}\\n' ;; esac; exit 0 ;;\n'ledger latest') printf '{{\"sequence\":{}}}\\n'; exit 0 ;;\nesac\ncase \"$1\" in\n'--version') printf 'stellar 26.1.0 (1228cff)\\n'; exit 0 ;;\nesac\nprintf 'the read-only stand-in refuses this command\\n' >&2\nexit 1\n",
                log.display(),
                MARKET_TESTNET_PASSPHRASE,
                admin.as_str(),
                other.as_str(),
                ledger,
            ),
        )
        .expect("stub stellar script");
        std::fs::set_permissions(&script, PermissionsExt::from_mode(0o755))
            .expect("stub stellar must be executable");
        script
    }

    /// Every command the stand-in was asked to run that could change the chain.
    fn mutating_calls(log: &str) -> Vec<String> {
        const MUTATIONS: [&str; 5] = [
            "contract invoke",
            "contract deploy",
            "contract upload",
            "contract restore",
            "keys generate",
        ];
        log.lines()
            .filter(|line| MUTATIONS.iter().any(|verb| line.starts_with(*verb)))
            .map(str::to_string)
            .collect()
    }

    /// How many times the run observed the ledger.
    fn ledger_reads(log: &str) -> usize {
        log.lines()
            .filter(|line| line.starts_with("ledger latest"))
            .count()
    }

    /// Every scheduled reserve with every parameter explicit, as the deploy
    /// gate requires. Values are the fixture defaults, spelled out.
    const FIXTURE_RESERVES_TOML: &str = "\
[reserves.stable]\nc_factor = 9000000\nl_factor = 9500000\nutil = 8500000\nmax_util = 9500000\nsupply_cap = \"1000000000000000000\"\nr_base = 100000\nr_one = 500000\nr_two = 5000000\nr_three = 15000000\nreactivity = 20\n\n\
[reserves.xlm]\nc_factor = 7500000\nl_factor = 7500000\nutil = 5000000\nmax_util = 9500000\nsupply_cap = \"1000000000000000000\"\nr_base = 100000\nr_one = 500000\nr_two = 5000000\nr_three = 15000000\nreactivity = 20\n\n\
[reserves.weth]\nc_factor = 8000000\nl_factor = 8000000\nutil = 7000000\nmax_util = 9500000\nsupply_cap = \"170141183460469231731687303715884105727\"\nr_base = 100000\nr_one = 500000\nr_two = 5000000\nr_three = 15000000\nreactivity = 20\n\n";

    /// A market configuration that survives preflight: every pinned artifact
    /// present with the real digest of its own bytes, the frozen oracle and
    /// adapter parameters, and the three playable balances the mint schedule
    /// freezes. Returns the config path with the parsed configuration.
    fn fixture_market_config(out: &Path) -> (PathBuf, MarketConfig) {
        let cfg_dir = out.join("cfg");
        std::fs::create_dir_all(&cfg_dir).expect("fixture directory");
        let mut cfg_text = String::from(
            "[network]\npassphrase = \"Test SDF Network ; September 2015\"\nlabel = \"testnet\"\n\n",
        );
        for role in crate::manifest::MARKET_ARTIFACT_ROLES {
            let artifact = cfg_dir.join(format!("{role}.wasm"));
            let bytes = format!("fixture wasm bytes for {role}").into_bytes();
            let digest = sha256_hex(&bytes);
            std::fs::write(&artifact, &bytes).expect("fixture artifact");
            cfg_text.push_str(&format!(
                "[artifacts.{role}]\npath = \"{}\"\nsha256 = \"{digest}\"\n\n",
                artifact.display()
            ));
        }
        cfg_text.push_str("[oracle]\nsources = 3\nmin_sources = 3\nmax_age_secs = 3600\nmax_clock_drift_secs = 60\nbase = \"USD\"\ndecimals = 7\nresolution = 300\n\n[adapter]\ndecimals = 7\nresolution = 1\nbase = \"USD\"\n\n");
        cfg_text.push_str(FIXTURE_RESERVES_TOML);
        cfg_text.push_str("[[balances]]\nwho = \"frodo\"\ntoken = \"stable\"\namount = 100000000000\n\n[[balances]]\nwho = \"sam\"\ntoken = \"stable\"\namount = 200000000000\n\n[[balances]]\nwho = \"user\"\ntoken = \"stable\"\namount = 300000000000\n");
        let config = cfg_dir.join("config.toml");
        std::fs::write(&config, &cfg_text).expect("fixture config");
        let cfg = MarketConfig::load(&config)
            .expect("fixture config must load through the strict parser");
        (config, cfg)
    }

    /// The CLI context for a fixture run, bound to the read-only stand-in and
    /// the operator alias the caller names.
    fn fixture_market_context(out: &Path, stellar: &Path, alias: &str) -> MarketExecutionContext {
        for dir in ["admin", "actors", "xdg"] {
            std::fs::create_dir_all(out.join(dir)).expect("fixture directories");
        }
        MarketExecutionContext::new(
            "https://soroban-testnet.stellar.org".to_string(),
            Vec::new(),
            alias.to_string(),
            out.join("admin"),
            out.join("actors"),
            stellar.to_path_buf(),
            out.join("xdg"),
            PathBuf::from("/"),
        )
        .expect("fixture market context")
    }

    /// Settle every coordinate whose facts a publication assembles from: each
    /// deterministic deployment through the id its own custody record predicts,
    /// and each run-local actor through the signer its funding coordinate
    /// settles. Both records go through the journal's reservation and
    /// settlement path, so nothing the strict reader would reject can be
    /// planted here. Returns the pool this run settled.
    fn settle_publication_inputs(journal: &MarketJournal, admin: &AccountId) -> ContractId {
        let mut pool = None;
        let mut deployment = 1u8;
        let mut actor = 1u8;
        for slot in market_schedule() {
            let funding = matches!(&slot.op, MOp::FundKey { .. });
            let role = MarketDeployer::deployed_role(&slot.op);
            if !funding && role.is_none() {
                continue;
            }
            let predicted = if funding {
                None
            } else {
                let id = fixture_contract(deployment);
                deployment += 1;
                if role.as_deref() == Some("pool") {
                    pool = Some(id.clone());
                }
                Some(id)
            };
            let signer = if funding {
                let addr = fixture_actor(actor);
                actor += 1;
                addr
            } else {
                admin.clone()
            };
            let mut record = MarketCustodyRecord {
                phase: slot.phase,
                kind: slot.kind,
                action: Safe::literal(slot.action),
                network: MarketTestnet::Testnet,
                signer,
                payload_digest: Sha256Hex::default(),
                target: predicted.clone(),
                artifact_digest: None,
                predicted: predicted.clone(),
                pre_state: vec![Safe::literal("the coordinate settled")],
                tool_error: None,
            };
            record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
            journal
                .begin(slot.phase, slot.slot, &record)
                .expect("reserve the coordinate");
            journal
                .finish(
                    slot.phase,
                    slot.slot,
                    &MarketPendingResult {
                        custody: record,
                        step: MarketStepRecord {
                            phase: slot.phase,
                            action: Safe::literal(slot.action),
                            command: Safe::render_parsed(
                                Policy::Market,
                                &json!({"coordinate": format!("{:03}-{:02}", slot.phase, slot.slot)}),
                                256,
                            ),
                            outcome: Safe::literal("the coordinate settled"),
                            target: predicted,
                            violations: Vec::new(),
                        },
                    },
                )
                .expect("settle the coordinate");
        }
        pool.expect("the frozen graph deploys a pool")
    }

    /// A market run directory built the way a real run builds it: the
    /// deployer's own constructor writes the frozen plan after a preflight that
    /// really passed, then every coordinate a publication assembles from is
    /// settled through the strict journal path. The publication coordinate is
    /// left unreserved, so each test stages the interruption it means to
    /// recover from.
    struct ACompletedMarket {
        out: PathBuf,
        config: PathBuf,
        stellar: PathBuf,
        call_log: PathBuf,
        admin: AccountId,
        ledger: u32,
        pool: ContractId,
        publish: MSlot,
        manifest: MarketManifest,
    }

    fn a_completed_market(tag: &str, ledger: u32) -> ACompletedMarket {
        let admin = fixture_actor(0);
        let out = std::env::temp_dir().join(format!("abtest-deploy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).expect("scratch directory");
        let stellar = read_only_stellar(&out, &admin, &fixture_actor(9), ledger);
        let (config, cfg) = fixture_market_config(&out);
        // The run's start: preflight proves the CLI, the artifacts, and the
        // operator, and the plan is written once for the graph to freeze.
        MarketDeployer::new(
            cfg,
            fixture_market_context(&out, &stellar, "market-admin"),
            MarketJournal::open(&out),
        )
        .expect("a market whose preflight passes constructs");
        let pool = settle_publication_inputs(&MarketJournal::open(&out), &admin);
        // A restart with nothing pending: it rebuilds deployment and identity
        // state the way a resume does, and assembles the manifest the publisher
        // would have written at the ledger the stand-in reports.
        let d = MarketDeployer::new(
            MarketConfig::load(&config).expect("fixture config reloads"),
            fixture_market_context(&out, &stellar, "market-admin"),
            MarketJournal::open(&out),
        )
        .expect("a market with a reproducible plan constructs");
        let manifest = d
            .assemble_manifest(ledger)
            .expect("the settled state assembles a manifest");
        let publish = market_schedule()
            .into_iter()
            .find(|s| s.op == MOp::Publish)
            .expect("the graph publishes last");
        ACompletedMarket {
            call_log: out.join("stellar-calls.log"),
            admin,
            config,
            ledger,
            manifest,
            out,
            pool,
            publish,
            stellar,
        }
    }

    impl ACompletedMarket {
        /// The operator-resuming constructor: the real entrypoint behavior,
        /// including whatever reconciliation the no-replay scan performs.
        fn resume(&self, alias: &str) -> MarketDeployer {
            MarketDeployer::new(
                MarketConfig::load(&self.config).expect("fixture config reloads"),
                fixture_market_context(&self.out, &self.stellar, alias),
                MarketJournal::open(&self.out),
            )
            .expect("a market resume constructs and refuses through its own law")
        }

        /// Freeze the publication intent the way the confirmation gate does: no
        /// target, no prediction, no artifact, bound by its payload digest.
        fn freeze_publication(&self, signer: &AccountId) {
            let mut record = MarketCustodyRecord {
                phase: self.publish.phase,
                kind: self.publish.kind,
                action: Safe::literal(self.publish.action),
                network: MarketTestnet::Testnet,
                signer: signer.clone(),
                payload_digest: Sha256Hex::default(),
                target: None,
                artifact_digest: None,
                predicted: None,
                pre_state: vec![Safe::literal(
                    "no settled result exists at this coordinate; nothing has been replayed",
                )],
                tool_error: None,
            };
            record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
            MarketJournal::open(&self.out)
                .begin(self.publish.phase, self.publish.slot, &record)
                .expect("freeze the publication intent");
        }

        fn journal(&self) -> MarketJournal {
            MarketJournal::open(&self.out)
        }

        fn publication_settled(&self) -> bool {
            self.journal()
                .result(self.publish.phase, self.publish.slot)
                .expect("the publication coordinate is readable")
                .is_some()
        }

        fn published_bytes(&self) -> Option<Vec<u8>> {
            std::fs::read(self.out.join(MARKET_MANIFEST_FILE)).ok()
        }

        fn publish(&self, manifest: &MarketManifest) {
            std::fs::write(
                self.out.join(MARKET_MANIFEST_FILE),
                toml::to_string_pretty(manifest).expect("render the published manifest"),
            )
            .expect("plant the published manifest");
        }

        fn intent_bytes(&self) -> Vec<u8> {
            std::fs::read(
                crate::manifest::market_pending_dir(
                    &self.out,
                    self.publish.phase,
                    self.publish.slot,
                )
                .join("intent.json"),
            )
            .expect("read the frozen intent")
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(&self.call_log).unwrap_or_default()
        }
    }

    /// The manifest's pool comes from the same settled record every other
    /// deployed address comes from. The deployment arm records a pool only in
    /// the deployed map, so an assembly reading it from a mirror of its own
    /// refuses a forward run at its publication coordinate while a restart
    /// happens to work. This pins the one record both paths must read, and that
    /// an unsettled pool is never published as if it were proven.
    #[test]
    fn the_manifest_names_the_pool_the_deployment_settled() {
        let run = a_completed_market("manifest-pool", 1_234_567);
        let mut d = run.resume("market-admin");
        let manifest = d
            .assemble_manifest(run.ledger)
            .expect("a run that settled its pool assembles its manifest");
        assert_eq!(
            manifest.pool, run.pool,
            "the manifest names the pool this run deployed"
        );
        assert_eq!(manifest.deploy_ledger, run.ledger);
        d.deployed.remove("pool");
        assert!(
            d.assemble_manifest(run.ledger).is_err(),
            "publication never carries a pool this run did not settle"
        );
        let _ = std::fs::remove_dir_all(&run.out);
    }

    /// The crash this recovery exists for: the graph settled, the publication
    /// intent froze, and the manifest never reached disk. Nothing was ever
    /// submitted, so a real resume completes that local publication, names the
    /// pool this run deployed, stamps the ledger it actually read, and settles
    /// the coordinate whose intent was already frozen. The stand-in was never
    /// asked for a mutation, and only the one read-only ledger observation a
    /// first publication legitimately needs was made.
    #[test]
    fn a_publication_that_never_reached_disk_publishes_on_resume() {
        let run = a_completed_market("publish-missing", 4_242_424);
        assert!(
            run.published_bytes().is_none(),
            "nothing is published before the interruption"
        );
        run.freeze_publication(&run.admin);
        assert!(
            publication_is_unsettled(&run.out).expect("the journal is readable"),
            "an interrupted publication is admissible to resume"
        );
        let d = run.resume("market-admin");
        assert!(
            d.terminal.is_none(),
            "an interrupted local publication resumes instead of abandoning custody"
        );
        let manifest = d.manifest.expect("the resume completed the publication");
        assert_eq!(
            manifest.pool, run.pool,
            "the manifest names the pool this run deployed"
        );
        assert_eq!(manifest.deploy_ledger, run.ledger);
        assert!(run.published_bytes().is_some(), "the manifest is on disk");
        assert!(
            run.publication_settled(),
            "the coordinate whose intent was frozen settled"
        );
        assert_eq!(
            ledger_reads(&run.calls()),
            1,
            "a first publication stamps itself from one read-only ledger observation"
        );
        assert!(
            mutating_calls(&run.calls()).is_empty(),
            "resuming a publication submitted nothing: {:?}",
            mutating_calls(&run.calls())
        );
        assert!(
            !publication_is_unsettled(&run.out).expect("the journal is readable"),
            "a settled publication is never reported pending, so a completed run stays refused"
        );
        let _ = std::fs::remove_dir_all(&run.out);
    }

    /// The other half of the crash window: the manifest's atomic rename
    /// succeeded and the settlement did not. Resume honors the published file
    /// as the proof it is, and needs no chain observation at all to do it,
    /// because even the ledger stamp comes from the file being honored. The
    /// frozen intent survives byte-for-byte.
    #[test]
    fn an_already_published_manifest_completes_without_any_observation() {
        let run = a_completed_market("publish-settled", 8_675_309);
        run.freeze_publication(&run.admin);
        run.publish(&run.manifest);
        let frozen_intent = run.intent_bytes();
        let d = run.resume("market-admin");
        assert!(
            d.terminal.is_none(),
            "an interrupted settlement resumes from the published file"
        );
        assert!(
            run.publication_settled(),
            "the published file settles the coordinate it was written for"
        );
        assert_eq!(
            d.manifest.expect("the resumed run holds its manifest").pool,
            run.pool,
            "the resumed run completes with the pool this run deployed"
        );
        assert_eq!(
            ledger_reads(&run.calls()),
            0,
            "honoring a published manifest observed nothing, not even the ledger"
        );
        assert!(
            mutating_calls(&run.calls()).is_empty(),
            "completing a settlement submitted nothing: {:?}",
            mutating_calls(&run.calls())
        );
        assert_eq!(
            run.intent_bytes(),
            frozen_intent,
            "completing a publication never rewrites its frozen intent"
        );
        let _ = std::fs::remove_dir_all(&run.out);
    }

    /// A manifest that does not reproduce this run's settled facts is not this
    /// run's proof. It is refused as unresolved custody and left exactly where
    /// it was, never overwritten; and the same directory completes once the
    /// manifest that really belongs to it is in place.
    #[test]
    fn another_runs_published_manifest_is_refused_not_overwritten() {
        let run = a_completed_market("publish-foreign", 5_555_555);
        run.freeze_publication(&run.admin);
        let mut foreign = run.manifest.clone();
        foreign.pool = fixture_contract(0);
        run.publish(&foreign);
        let foreign_bytes = run.published_bytes().expect("the foreign file is planted");
        let d = run.resume("market-admin");
        assert!(
            matches!(
                d.terminal,
                Some(DeployEvent::Custody {
                    unresolved: true,
                    ..
                })
            ),
            "a manifest that does not reproduce this run is unresolved custody"
        );
        assert!(
            !run.publication_settled(),
            "a refused publication stays open for its operator"
        );
        assert_eq!(
            run.published_bytes().expect("still planted"),
            foreign_bytes,
            "a foreign manifest is honored as evidence, never overwritten"
        );
        assert!(
            mutating_calls(&run.calls()).is_empty(),
            "refusing a foreign manifest submitted nothing: {:?}",
            mutating_calls(&run.calls())
        );
        run.publish(&run.manifest);
        let d = run.resume("market-admin");
        assert!(
            d.terminal.is_none(),
            "the same directory completes once this run's manifest is in place"
        );
        assert_eq!(
            d.manifest.expect("the publication completed").pool,
            run.pool,
            "the completed run names the pool this run deployed"
        );
        assert_eq!(
            ledger_reads(&run.calls()),
            0,
            "completing from a published file needed no observation at all"
        );
        let _ = std::fs::remove_dir_all(&run.out);
    }

    /// A resume whose frozen plan does not reproduce is refused, and a refused
    /// run must leave every existing record exactly as it found it. Scanning
    /// can now settle a coordinate and write a manifest naming whoever the
    /// resumed configuration resolved, so the scan must not run at all behind
    /// that refusal: the records belong to the run the plan froze, and the
    /// operator who can reproduce it must still be able to recover them.
    #[test]
    fn a_refused_resume_neither_publishes_nor_settles() {
        let run = a_completed_market("refused", 3_141_592);
        run.freeze_publication(&run.admin);
        // A different but perfectly valid operator: preflight proves it, and
        // the plan the original run froze does not reproduce from it.
        let d = run.resume("someone-else");
        assert!(
            matches!(d.terminal, Some(DeployEvent::Failed(_))),
            "a plan that does not reproduce is refused, never re-planned"
        );
        assert!(d.manifest.is_none(), "a refused run holds no manifest");
        assert!(
            run.published_bytes().is_none(),
            "a refused resume wrote no manifest naming its own operator"
        );
        assert!(
            !run.publication_settled(),
            "a refused resume settled no coordinate"
        );
        assert!(
            mutating_calls(&run.calls()).is_empty(),
            "a refused resume submitted nothing: {:?}",
            mutating_calls(&run.calls())
        );
        let d = run.resume("market-admin");
        assert!(
            d.terminal.is_none(),
            "the operator the plan froze still recovers the same directory"
        );
        assert_eq!(
            d.manifest.expect("the publication completed").pool,
            run.pool,
            "the records a refused resume preserved are the ones the recovery needed"
        );
        let _ = std::fs::remove_dir_all(&run.out);
    }
}
