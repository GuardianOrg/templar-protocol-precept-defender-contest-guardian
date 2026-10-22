//! Pair configuration (TOML in), deployed manifests (TOML out), run state (JSON).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::stellar::{AccountId, ContractId, Invocation, Net, Safe};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    StockVsFork,
    ForkVsFork,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    Admin,
    Frodo,
    Sam,
    User,
}

impl Who {
    pub const ALL: [Who; 4] = [Who::Admin, Who::Frodo, Who::Sam, Who::User];
    pub fn key_name(self) -> String {
        format!("abtest-{}", self.as_str())
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Who::Admin => "admin",
            Who::Frodo => "frodo",
            Who::Sam => "sam",
            Who::User => "user",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tok {
    Blnd,
    Usdc,
    Weth,
    Xlm,
    Stable,
    Lp,
}

impl Tok {
    /// Mock sep-41 tokens deployed per side, in deploy order.
    pub const MOCK: [Tok; 5] = [Tok::Blnd, Tok::Usdc, Tok::Weth, Tok::Xlm, Tok::Stable];
    pub const ALL: [Tok; 6] = [
        Tok::Blnd,
        Tok::Usdc,
        Tok::Weth,
        Tok::Xlm,
        Tok::Stable,
        Tok::Lp,
    ];
    /// Reserve assets in the fixture's oracle order.
    pub const ORACLE_ORDER: [Tok; 4] = [Tok::Weth, Tok::Usdc, Tok::Xlm, Tok::Stable];
    pub const RESERVES: [Tok; 3] = [Tok::Stable, Tok::Xlm, Tok::Weth];
    pub fn decimals(self) -> u32 {
        match self {
            Tok::Weth => 9,
            Tok::Stable => 6,
            _ => 7,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Tok::Blnd => "blnd",
            Tok::Usdc => "usdc",
            Tok::Weth => "weth",
            Tok::Xlm => "xlm",
            Tok::Stable => "stable",
            Tok::Lp => "lp",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedWasm {
    pub token_wasm: PathBuf,
    pub oracle_wasm: PathBuf,
    pub comet_wasm: PathBuf,
    pub emitter_wasm: PathBuf,
}

/// Per-reserve overrides (7-decimal fixed point except supply_cap, which is a
/// base-unit i128 string). Pair runs may set any subset; a market deploy must
/// set every field explicitly (`MarketConfig::require_explicit_reserves`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReserveOverride {
    pub c_factor: Option<u32>,
    pub l_factor: Option<u32>,
    pub util: Option<u32>,
    pub max_util: Option<u32>,
    pub supply_cap: Option<String>,
    pub r_base: Option<u32>,
    pub r_one: Option<u32>,
    pub r_two: Option<u32>,
    pub r_three: Option<u32>,
    pub reactivity: Option<u32>,
}

impl ReserveOverride {
    /// Names of the fields still falling back to a fixture default.
    pub fn missing_fields(&self) -> Vec<&'static str> {
        [
            (self.c_factor.is_some(), "c_factor"),
            (self.l_factor.is_some(), "l_factor"),
            (self.util.is_some(), "util"),
            (self.max_util.is_some(), "max_util"),
            (self.supply_cap.is_some(), "supply_cap"),
            (self.r_base.is_some(), "r_base"),
            (self.r_one.is_some(), "r_one"),
            (self.r_two.is_some(), "r_two"),
            (self.r_three.is_some(), "r_three"),
            (self.reactivity.is_some(), "reactivity"),
        ]
        .into_iter()
        .filter(|(set, _)| !set)
        .map(|(_, name)| name)
        .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideSpec {
    pub label: String,
    pub pool_wasm: PathBuf,
    pub backstop_wasm: PathBuf,
    pub factory_wasm: PathBuf,
    #[serde(default)]
    pub reserves: BTreeMap<Tok, ReserveOverride>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairConfig {
    pub profile: Profile,
    /// What this pair is comparing, shown in the TUI's info overlay.
    #[serde(default)]
    pub description: Option<String>,
    pub shared: SharedWasm,
    pub a: SideSpec,
    pub b: SideSpec,
}

impl PairConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let mut cfg: PairConfig =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        for p in [
            &mut cfg.shared.token_wasm,
            &mut cfg.shared.oracle_wasm,
            &mut cfg.shared.comet_wasm,
            &mut cfg.shared.emitter_wasm,
            &mut cfg.a.pool_wasm,
            &mut cfg.a.backstop_wasm,
            &mut cfg.a.factory_wasm,
            &mut cfg.b.pool_wasm,
            &mut cfg.b.backstop_wasm,
            &mut cfg.b.factory_wasm,
        ] {
            *p = expand_home(p);
            // Pin to absolute now: existence is checked here, but the upload happens
            // later with the run dir as the child's cwd.
            if !p.is_absolute() {
                *p = std::env::current_dir()
                    .context("resolve relative wasm path")?
                    .join(&*p);
            }
            anyhow::ensure!(p.is_file(), "wasm not a file: {}", p.display());
        }
        anyhow::ensure!(
            !cfg.a.label.trim().is_empty()
                && !cfg.b.label.trim().is_empty()
                && cfg.a.label != cfg.b.label,
            "side labels must be non-empty and distinct (a={:?}, b={:?})",
            cfg.a.label,
            cfg.b.label
        );
        Ok(cfg)
    }
}

pub fn expand_home(p: &Path) -> PathBuf {
    let s = p.display().to_string();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    p.to_path_buf()
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Contracts {
    pub blnd: String,
    pub usdc: String,
    pub weth: String,
    pub xlm: String,
    pub stable: String,
    pub lp: String,
    pub emitter: String,
    pub backstop: String,
    pub pool_factory: String,
    pub oracle: String,
    pub pool: String,
}

impl Contracts {
    pub fn token(&self, t: Tok) -> &str {
        match t {
            Tok::Blnd => &self.blnd,
            Tok::Usdc => &self.usdc,
            Tok::Weth => &self.weth,
            Tok::Xlm => &self.xlm,
            Tok::Stable => &self.stable,
            Tok::Lp => &self.lp,
        }
    }
    pub fn set_token(&mut self, t: Tok, id: String) {
        match t {
            Tok::Blnd => self.blnd = id,
            Tok::Usdc => self.usdc = id,
            Tok::Weth => self.weth = id,
            Tok::Xlm => self.xlm = id,
            Tok::Stable => self.stable = id,
            Tok::Lp => self.lp = id,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReserveRef {
    pub index: u32,
    pub decimals: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SideManifest {
    pub label: String,
    pub deploy_ledger: u32,
    /// wasm role -> hash
    pub wasm: BTreeMap<String, String>,
    pub contracts: Contracts,
    pub reserves: BTreeMap<Tok, ReserveRef>,
    /// Total minted per mock token, for conservation checks (fits i64 by construction).
    pub minted: BTreeMap<Tok, i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairManifest {
    pub profile: Profile,
    #[serde(default)]
    pub description: Option<String>,
    pub net: Net,
    /// identity -> G address (shared by both sides)
    pub identities: BTreeMap<Who, String>,
    pub a: SideManifest,
    pub b: SideManifest,
}

impl PairManifest {
    pub fn path(out: &Path) -> PathBuf {
        out.join("manifest.toml")
    }
    pub fn save(&self, out: &Path) -> anyhow::Result<()> {
        // Atomic, like `RunState::save`: a kill mid-write would otherwise
        // truncate the only record of deployed contracts, and the deployed
        // value is already spent — the run dir becomes unrecoverable.
        std::fs::create_dir_all(out)?;
        let p = Self::path(out);
        let tmp = out.join("manifest.toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?).context("staging manifest")?;
        fsync(&tmp, "staged manifest")?;
        std::fs::rename(&tmp, &p).context("publishing manifest")?;
        fsync(out, "run directory")?;
        Ok(())
    }
    pub fn load(out: &Path) -> anyhow::Result<Self> {
        let p = Self::path(out);
        let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        Ok(toml::from_str(&text)?)
    }
    pub fn addr(&self, who: Who) -> &str {
        &self.identities[&who]
    }
}

/// Mutable run state persisted between `run` invocations.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunState {
    /// scenario -> last completed step label, per side pair
    pub completed: BTreeMap<String, Vec<String>>,
    /// Exact requested labels, including waits; operator actions are not a campaign.
    #[serde(default)]
    pub requested: BTreeMap<String, Vec<String>>,
    /// side label -> extra minted since deploy (conservation)
    pub minted: BTreeMap<String, BTreeMap<Tok, i64>>,
    /// side label -> auction owner -> auction start block from `new_auction`
    pub auction_blocks: BTreeMap<String, BTreeMap<Who, u32>>,
    pub frozen: bool,
    /// Scenarios in the order they were executed; the report replays cross-step
    /// context (delta mode, pool-loss custody) in this order.
    #[serde(default)]
    pub scenario_order: Vec<String>,
}

impl RunState {
    pub fn path(out: &Path) -> PathBuf {
        out.join("state.json")
    }
    pub fn load(out: &Path) -> anyhow::Result<Self> {
        let p = Self::path(out);
        if !p.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
    }
    pub fn save(&self, out: &Path) -> anyhow::Result<()> {
        // Atomic: a kill between write and completion must leave the previous
        // resumable state readable, never a half-written JSON document.
        let p = Self::path(out);
        let tmp = out.join("state.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?).context("staging run state")?;
        fsync(&tmp, "staged run state")?;
        std::fs::rename(&tmp, &p).context("publishing run state")?;
        fsync(out, "run directory")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideOutcome {
    Declined,
    Unmutating,
    Invocation(Invocation),
    ToolError(String),
}

impl SideOutcome {
    fn settled(&self) -> bool {
        matches!(self, Self::Declined | Self::Unmutating)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingIntent {
    pub scenario: String,
    pub index: usize,
    pub label: String,
    pub side: char,
}

pub fn pending_step_dir(out: &Path, scenario: &str, index: usize, label: &str) -> PathBuf {
    out.join("pending")
        .join(scenario)
        .join(format!("{index:03}-{label}"))
}

fn fsync(path: &Path, what: &str) -> anyhow::Result<()> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("opening {what} {} for sync", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {what} {}", path.display()))
}

/// Durably reserve one side's pre-invocation intent. `create_new` makes a second
/// attempt fail before it can reach the chain, even if a prior process died after
/// opening the intent but before recording its outcome.
pub fn begin_pending(out: &Path, intent: &PendingIntent) -> anyhow::Result<()> {
    use std::io::Write;
    let side = intent.side;
    let dir = pending_step_dir(out, &intent.scenario, intent.index, &intent.label);
    std::fs::create_dir_all(&dir).context("creating pending step directory")?;
    let result = dir.join(format!("{side}.result.json"));
    anyhow::ensure!(
        !result.exists(),
        "pending step {} {} already has side {} outcome {}",
        intent.scenario,
        intent.label,
        side,
        result.display()
    );
    let path = dir.join(format!("{side}.intent.json"));
    let bytes = serde_json::to_vec_pretty(intent)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("reserving side {side} intent {}", path.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("writing side {side} intent {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing side {side} intent {}", path.display()))?;
    fsync(&dir, "pending step directory")
}

pub fn finish_pending(
    out: &Path,
    scenario: &str,
    index: usize,
    label: &str,
    side: char,
    outcome: SideOutcome,
) -> anyhow::Result<()> {
    use std::io::Write;
    let dir = pending_step_dir(out, scenario, index, label);
    let path = dir.join(format!("{side}.result.json"));
    let bytes = serde_json::to_vec_pretty(&outcome)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("reserving side {side} outcome {}", path.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("writing side {side} outcome {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing side {side} outcome {}", path.display()))?;
    fsync(&dir, "pending outcome directory")
}

pub fn clear_pending(out: &Path, scenario: &str, index: usize, label: &str) -> anyhow::Result<()> {
    let step = pending_step_dir(out, scenario, index, label);
    if step.exists() {
        std::fs::remove_dir_all(&step)
            .with_context(|| format!("clearing pending step {}", step.display()))?;
    }
    let scenario_dir = out.join("pending").join(scenario);
    if scenario_dir.exists() && scenario_dir.read_dir()?.next().is_none() {
        let _ = std::fs::remove_dir(&scenario_dir);
    }
    let root = out.join("pending");
    if root.exists() && root.read_dir()?.next().is_none() {
        let _ = std::fs::remove_dir(&root);
    }
    Ok(())
}

/// Remove attempts proved to have touched no chain state. Anything else is an
/// unresolved attempt and is returned rather than silently dropped.
pub fn reap_safe_pending(out: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let root = out.join("pending");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut unsafe_steps = Vec::new();
    for scenario in std::fs::read_dir(&root)?.flatten() {
        let scenario_dir = scenario.path();
        if !scenario_dir.is_dir() {
            unsafe_steps.push(scenario_dir);
            continue;
        }
        for step in std::fs::read_dir(&scenario_dir)?.flatten() {
            let step_dir = step.path();
            if !step_dir.is_dir() {
                unsafe_steps.push(step_dir);
                continue;
            }
            let mut intents = std::fs::read_dir(&step_dir)?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".intent.json"))
                })
                .collect::<Vec<_>>();
            intents.sort();
            let settled = !intents.is_empty()
                && intents.iter().all(|intent| {
                    let name = intent
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let result = step_dir.join(name.replace(".intent.json", ".result.json"));
                    std::fs::read(&result)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<SideOutcome>(&bytes).ok())
                        .is_some_and(|outcome| outcome.settled())
                });
            if settled {
                let _ = std::fs::remove_dir_all(&step_dir);
            } else {
                unsafe_steps.push(step_dir);
            }
        }
        if scenario_dir.exists() && scenario_dir.read_dir()?.next().is_none() {
            let _ = std::fs::remove_dir(&scenario_dir);
        }
    }
    if root.exists() && root.read_dir()?.next().is_none() {
        let _ = std::fs::remove_dir(&root);
    }
    unsafe_steps.sort();
    Ok(unsafe_steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trips_through_toml() {
        let side = SideManifest {
            label: "fork".into(),
            deploy_ledger: 42,
            wasm: [("pool".to_string(), "ab".to_string())].into(),
            contracts: Contracts {
                pool: "CPOOL".into(),
                ..Default::default()
            },
            reserves: [(
                Tok::Stable,
                ReserveRef {
                    index: 0,
                    decimals: 6,
                },
            )]
            .into(),
            minted: [(Tok::Stable, 100_000_000_000i64)].into(),
        };
        let m = PairManifest {
            profile: Profile::StockVsFork,
            description: Some("test pair".into()),
            net: Net {
                rpc_url: "u".into(),
                passphrase: "p".into(),
                config_dir: "/c".into(),
                headers: Vec::new(),
            },
            identities: [(Who::Admin, "GADMIN".into())].into(),
            a: side.clone(),
            b: side,
        };
        let text = toml::to_string_pretty(&m).unwrap();
        let back: PairManifest = toml::from_str(&text).unwrap();
        assert_eq!(back.a.reserves[&Tok::Stable].decimals, 6);
        assert_eq!(back.addr(Who::Admin), "GADMIN");
        assert_eq!(back.profile, Profile::StockVsFork);
    }

    #[test]
    fn pair_config_parses_overrides() {
        let text = r#"
profile = "fork-vs-fork"
[shared]
token_wasm = "/x/t.wasm"
oracle_wasm = "/x/o.wasm"
comet_wasm = "/x/c.wasm"
emitter_wasm = "/x/e.wasm"
[a]
label = "a"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
[b]
label = "b"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
[b.reserves.xlm]
c_factor = 6000000
"#;
        let cfg: PairConfig = toml::from_str(text).unwrap();
        assert_eq!(cfg.b.reserves[&Tok::Xlm].c_factor, Some(6_000_000));
        assert!(cfg.a.reserves.is_empty());
    }

    /// A typo'd key used to be dropped silently, deploying a pair the operator
    /// did not configure.
    #[test]
    fn unknown_toml_keys_are_rejected_not_dropped() {
        let text = r#"profile = "fork-vs-fork"
[shared]
token_wasm = "/x/t.wasm"
oracle_wasm = "/x/o.wasm"
comet_wasm = "/x/c.wasm"
emitter_wasm = "/x/e.wasm"
[a]
label = "a"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
typo_field = true
[b]
label = "b"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
"#;
        let err = toml::from_str::<PairConfig>(text).unwrap_err();
        assert!(
            err.to_string().contains("typo_field"),
            "error should name the unknown key: {err}"
        );
    }

    /// An empty path absolutizes to the cwd directory, which `exists()` accepts;
    /// only an `is_file` check rejects the typo before any chain work starts.
    #[test]
    fn an_empty_wasm_path_is_rejected_before_deploy() {
        let dir = std::env::temp_dir().join(format!("abtest-emptywasm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"profile = "fork-vs-fork"
[shared]
token_wasm = ""
oracle_wasm = "/x/o.wasm"
comet_wasm = "/x/c.wasm"
emitter_wasm = "/x/e.wasm"
[a]
label = "a"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
[b]
label = "b"
pool_wasm = "/x/p.wasm"
backstop_wasm = "/x/b.wasm"
factory_wasm = "/x/f.wasm"
"#;
        let path = dir.join("pair.toml");
        std::fs::write(&path, text).unwrap();
        let err = PairConfig::load(&path).unwrap_err();
        assert!(err.to_string().contains("wasm not a file"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lock_stamped_with_our_recycled_pid_reads_as_stale() {
        let dir = std::env::temp_dir().join(format!("abtest-ownpid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Our pid with a start time that cannot be ours: the pid was recycled.
        std::fs::write(
            dir.join("run.lock"),
            format!("{} 9999999999999", std::process::id()),
        )
        .unwrap();
        assert_eq!(
            RunLock::owner(&dir),
            None,
            "own recycled pid must not read live"
        );
        assert!(RunLock::acquire(&dir).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A racing reclaim can unlink our file between create and stamp; only a
    /// file still carrying our stamp may be trusted, and Drop must never release
    /// someone else's lock.
    #[test]
    fn drop_does_not_release_a_reclaimed_lock() {
        let dir = std::env::temp_dir().join(format!("abtest-reclaim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = RunLock::acquire(&dir).unwrap().unwrap();
        // Another "process" reclaims the lock file under us.
        std::fs::write(dir.join("run.lock"), "4294967294 1").unwrap();
        drop(lock);
        assert!(
            dir.join("run.lock").exists(),
            "Drop must not unlink a lock we no longer hold"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `save` writes via tmp+rename, so an interrupted write can never leave
    /// half-JSON where the previous resumable state used to be.
    #[test]
    fn state_round_trips_with_no_temp_residue() {
        let dir = std::env::temp_dir().join(format!("abtest-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = RunState::default();
        st.completed.insert("lending".into(), vec!["borrow".into()]);
        st.requested.insert("lending".into(), vec!["borrow".into()]);
        st.scenario_order.push("lending".into());
        st.save(&dir).unwrap();
        let back = RunState::load(&dir).unwrap();
        assert_eq!(back.scenario_order, vec!["lending"]);
        assert_eq!(back.completed["lending"], vec!["borrow"]);
        assert_eq!(back.requested["lending"], vec!["borrow"]);
        assert!(!dir.join("state.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Our own stamp always carries a nonzero start tick; a `start == 0` file
    /// was written by an external tool and its bare pid must never be trusted
    /// as a live owner (pid 1 is always alive).
    #[test]
    fn externally_written_lock_is_foreign() {
        let dir = std::env::temp_dir().join(format!("abtest-foreign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.lock"), "1 0").unwrap();
        assert_eq!(RunLock::owner(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A malformed market manifest fails with a fixed input-free message: the
    /// rejected value and any terminal controls planted inside it must never
    /// reach an error display, not even through the source chain.
    #[test]
    fn malformed_market_manifest_errors_never_echo_the_rejected_value() {
        let dir = std::env::temp_dir().join(format!("abtest-badmanifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("market-manifest.toml"),
            "pool = \"SENTINELVALUE\u{1b}[2Jpaid\u{7}\"\n",
        )
        .unwrap();
        let err = MarketManifest::load(&dir).unwrap_err();
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("market manifest is malformed"),
            "refusal must carry the fixed message: {rendered:?}"
        );
        for source in err.chain() {
            let text = source.to_string();
            assert!(
                !text.contains("SENTINELVALUE"),
                "error chain leaked the planted value: {text:?}"
            );
            assert!(
                !text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
                "error chain leaked terminal controls: {text:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A single-source bridge manifest carries no governance contract: the
    /// absence parses as absence, round-trips as absence, and serializes as
    /// absence. The absence must never gain an address on its own.
    #[test]
    fn a_market_manifest_may_omit_governance_without_fabricating_one() {
        use std::str::FromStr;
        let c = || {
            ContractId::from_str("CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM")
                .unwrap()
        };
        let manifest = MarketManifest {
            network: MarketNetworkRecord {
                label: Safe::literal(MARKET_NETWORK_LABEL),
                passphrase: Safe::literal(MARKET_TESTNET_PASSPHRASE),
            },
            admin: AccountId::from_str("GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF")
                .unwrap(),
            rpc_host: Safe::literal("rpc.testnet.local"),
            oracle: OracleStackManifest {
                runtime: c(),
                governance: None,
                adapter: c(),
                sources: vec![c()],
                min_sources: 1,
                max_age_secs: 3600,
                max_clock_drift_secs: 60,
                decimals: 7,
                resolution: 300,
                base: Safe::literal("USD"),
            },
            tokens: BTreeMap::new(),
            pool: c(),
            backstop: c(),
            factory: c(),
            comet: c(),
            emitter: c(),
            reserves: BTreeMap::new(),
            identities: BTreeMap::new(),
            deploy_ledger: 1,
        };
        assert!(manifest.oracle.governance.is_none());
        let dir = std::env::temp_dir().join(format!("abtest-nogov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("market-manifest.toml"),
            toml::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let back = MarketManifest::load(&dir).unwrap();
        assert!(
            back.oracle.governance.is_none(),
            "an omitted governance field must load back as none"
        );
        let rendered = toml::to_string(&back).unwrap();
        assert!(
            !rendered.contains("governance"),
            "governance must not serialize when absent: {rendered}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
// --------------------------------------------------------------------------
// Run lock
//
// Two runners on one output directory interleave writes to `state.json`, so the
// directory is owned by at most one live process. The TUI uses the same lock to
// decide between driving and watching.
// --------------------------------------------------------------------------

pub struct RunLock {
    path: PathBuf,
}

impl RunLock {
    fn file(out: &Path) -> PathBuf {
        out.join("run.lock")
    }

    /// `pid start_ticks`; the start time disambiguates a recycled pid.
    fn stamp() -> String {
        format!(
            "{} {}",
            std::process::id(),
            proc_start_ticks(std::process::id()).unwrap_or(0)
        )
    }

    /// `Ok(None)` when a live process already owns this directory.
    /// `create_new` alone does not exclude a racing reclaim (unlink-then-recreate),
    /// so after stamping we only trust a file still carrying our own stamp.
    pub fn acquire(out: &Path) -> anyhow::Result<Option<RunLock>> {
        let path = Self::file(out);
        let mine = Self::stamp();
        for _ in 0..3 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    f.write_all(mine.as_bytes())?;
                    drop(f);
                    if Self::read_stamp(&path).as_deref() == Some(mine.as_str()) {
                        return Ok(Some(RunLock { path }));
                    }
                    // A racing reclaim replaced our file; the next round sees it.
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Self::owner(out).is_some() {
                        return Ok(None);
                    }
                    let _ = std::fs::remove_file(&path); // recorded but dead: reclaim
                }
                Err(e) => return Err(e).with_context(|| format!("lock {}", path.display())),
            }
        }
        Ok(None) // lost the race repeatedly
    }

    fn read_stamp(path: &Path) -> Option<String> {
        std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_owned())
    }

    /// The pid in the lock file, if that process is still the one that wrote it.
    /// A recycled pid has a different `/proc` start time and reads as stale —
    /// including when the recycled pid happens to be ours.
    pub fn owner(out: &Path) -> Option<u32> {
        let text = std::fs::read_to_string(Self::file(out)).ok()?;
        let mut it = text.split_whitespace();
        let pid: u32 = it.next()?.parse().ok()?;
        let start: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        // `start == 0` never comes from our own stamp on Linux (a live pid
        // always has /proc ticks): it marks a file written by an external
        // writer, so it is foreign — stale, not "unknown, trust the pid".
        if start == 0 {
            return None;
        }
        proc_start_ticks(pid).map(|t| t == start)?.then_some(pid)
    }
}

/// 100 Hz process start time (`/proc/<pid>/stat` field 22); `None` when the pid is gone.
fn proc_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

impl Drop for RunLock {
    fn drop(&mut self) {
        // Never release a lock another holder reclaimed under us.
        let mine = Self::stamp();
        if Self::read_stamp(&self.path).as_deref() == Some(mine.as_str()) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_then_released() {
        let dir = std::env::temp_dir().join(format!("abtest-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let held = RunLock::acquire(&dir).unwrap().expect("first acquire wins");
        assert_eq!(RunLock::owner(&dir), Some(std::process::id()));
        assert!(
            RunLock::acquire(&dir).unwrap().is_none(),
            "second acquire must observe the live owner"
        );
        drop(held);
        assert_eq!(RunLock::owner(&dir), None);
        assert!(RunLock::acquire(&dir).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let dir = std::env::temp_dir().join(format!("abtest-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.lock"), "4294967294").unwrap(); // no such pid
        assert_eq!(RunLock::owner(&dir), None);
        assert!(RunLock::acquire(&dir).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// --------------------------------------------------------------------------
// Market mode: one disposable protected Blend market on Stellar testnet.
//
// Everything above is pair-only and stays unchanged. Market deployment and
// play get their own strict config, manifests, and custody shapes (plan §2/§4)
// so a single market is never dressed up as an A/B pair. Market records carry
// only rendered `Safe` text and item-7 validated identifiers — there is no
// `Invocation`/`ErrorKind`/`RawText` field here, so raw CLI bytes are
// structurally absent from every market surface.
// --------------------------------------------------------------------------

/// Exact Stellar testnet passphrase: a market-mode invariant that no config or
/// manifest can relax, rechecked before every mutation.
pub const MARKET_TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";

/// Credential-free network label persisted in `MarketManifest` (never a
/// serializable pair `Net`).
pub const MARKET_NETWORK_LABEL: &str = "testnet";

/// Synthetic quorum shape required by `validate_proxy_config`
/// (`contracts/contract/proxy-oracle/soroban/common/src/lib.rs:158-191`).
pub const MARKET_SOURCE_COUNT: usize = 3;
pub const MARKET_MIN_SOURCES: u32 = 3;
pub const MARKET_MAX_AGE_SECS: u32 = 3600;
pub const MARKET_MAX_CLOCK_DRIFT_SECS: u32 = 60;
/// SEP-40 feed granularity; deliberately different from the adapter's own
/// resolution — the adapter is its own Blend oracle (plan §4 phase 4).
pub const MARKET_SOURCE_RESOLUTION: u32 = 300;
pub const MARKET_SOURCE_DECIMALS: u32 = 7;
pub const MARKET_ADAPTER_DECIMALS: u32 = 7;
pub const MARKET_ADAPTER_RESOLUTION: u32 = 1;
pub const MARKET_BASE: &str = "USD";

/// Artifact roles whose optimized Wasm digests are required before the first
/// network call. One qualified source digest is intentionally reused for the
/// three same-admin source roles (plan §2).
pub const MARKET_ARTIFACT_ROLES: [&str; 10] = [
    "pool",
    "backstop",
    "factory",
    "token",
    "source",
    "comet",
    "emitter",
    "proxy-runtime",
    "proxy-governance",
    "adapter",
];

/// Variant order is the wire-name order: the `BTreeMap` ordering, the string
/// the deployer looks roles up by (`Borrow<str>`), and the TOML key order all
/// have to agree, so adding a role means inserting it at its alphabetical spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    Adapter,
    Backstop,
    Comet,
    Emitter,
    Factory,
    Pool,
    #[serde(rename = "proxy-governance")]
    ProxyGovernance,
    #[serde(rename = "proxy-runtime")]
    ProxyRuntime,
    Source,
    Token,
}

impl std::borrow::Borrow<str> for ArtifactRole {
    fn borrow(&self) -> &str {
        match self {
            Self::Adapter => "adapter",
            Self::Backstop => "backstop",
            Self::Comet => "comet",
            Self::Emitter => "emitter",
            Self::Factory => "factory",
            Self::Pool => "pool",
            Self::ProxyGovernance => "proxy-governance",
            Self::ProxyRuntime => "proxy-runtime",
            Self::Source => "source",
            Self::Token => "token",
        }
    }
}

impl std::fmt::Display for ArtifactRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(std::borrow::Borrow::<str>::borrow(self))
    }
}

/// A persisted SHA-256 digest; parsing never normalizes malformed input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Sha256Hex(String);

impl Sha256Hex {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Sha256Hex {
    fn default() -> Self {
        // The canonical custody payload blanks this slot before hashing.
        Self("0".repeat(64))
    }
}

impl TryFrom<String> for Sha256Hex {
    type Error = anyhow::Error;

    fn try_from(value: String) -> anyhow::Result<Self> {
        anyhow::ensure!(
            is_sha256_hex(&value),
            "SHA-256 digest must be 64 lowercase hex characters"
        );
        Ok(Self(value))
    }
}

impl std::ops::Deref for Sha256Hex {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

/// The only network a persisted market custody record can represent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketTestnet {
    #[serde(rename = "Test SDF Network ; September 2015")]
    Testnet,
}

/// A positive signed amount, retaining the existing i64 wire range.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(try_from = "i64")]
pub struct PositiveAmount(i64);

impl TryFrom<i64> for PositiveAmount {
    type Error = anyhow::Error;

    fn try_from(value: i64) -> anyhow::Result<Self> {
        anyhow::ensure!(value > 0, "initial balance amount must be positive");
        Ok(Self(value))
    }
}

impl std::fmt::Display for PositiveAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// One pinned Wasm artifact: path plus its required SHA-256 digest.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPin {
    pub path: PathBuf,
    pub sha256: Sha256Hex,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketNetworkConfig {
    pub passphrase: String,
    pub label: String,
}

/// Proxy-oracle stack wiring: three same-admin synthetic sources, quorum, and
/// freshness bounds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OracleStackConfig {
    pub sources: usize,
    pub min_sources: u32,
    pub max_age_secs: u32,
    pub max_clock_drift_secs: u32,
    pub base: String,
    pub decimals: u32,
    pub resolution: u32,
}

/// Immutable `PoolOracleAdapter` metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    pub decimals: u32,
    pub resolution: u32,
    pub base: String,
}

/// Initial fake balance minted to a run-local actor. Test assets only.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialBalance {
    pub who: Who,
    pub token: Tok,
    pub amount: PositiveAmount,
}

/// Strict single-market configuration
/// (`tools/abtest/markets/protected-testnet.toml`). Unknown fields are
/// rejected, never dropped.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketConfig {
    pub network: MarketNetworkConfig,
    pub artifacts: BTreeMap<ArtifactRole, ArtifactPin>,
    pub oracle: OracleStackConfig,
    pub adapter: AdapterConfig,
    #[serde(default)]
    pub reserves: BTreeMap<Tok, ReserveOverride>,
    #[serde(default)]
    pub balances: Vec<InitialBalance>,
}

impl MarketConfig {
    /// Load and fully validate a market config. Every failure here happens
    /// before any executor call — no executor is even constructed.
    pub fn load(path: &Path) -> anyhow::Result<MarketConfig> {
        let text = std::fs::read_to_string(path).context("reading market config")?;
        let cfg: MarketConfig = toml::from_str(&text).context("parsing market config")?;
        cfg.validate().context("validating market config")?;
        cfg.check_artifacts()
            .context("validating market artifacts")?;
        Ok(cfg)
    }

    /// Structural invariants (plan §2). Rejection text is input-free: config
    /// and CLI bytes are never echoed onto market error surfaces.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.network.passphrase == MARKET_TESTNET_PASSPHRASE,
            "market config requires the exact Stellar testnet passphrase"
        );
        anyhow::ensure!(
            self.network.label == MARKET_NETWORK_LABEL,
            "market config requires the testnet network label"
        );
        anyhow::ensure!(
            self.oracle.sources == MARKET_SOURCE_COUNT,
            "market config requires exactly three oracle sources"
        );
        anyhow::ensure!(
            self.oracle.min_sources == MARKET_MIN_SOURCES,
            "market config requires min_sources = 3"
        );
        anyhow::ensure!(
            self.oracle.max_age_secs == MARKET_MAX_AGE_SECS,
            "market config requires proxy max_age_secs = 3600"
        );
        anyhow::ensure!(
            self.oracle.max_clock_drift_secs == MARKET_MAX_CLOCK_DRIFT_SECS,
            "market config requires proxy max_clock_drift_secs = 60"
        );
        anyhow::ensure!(
            self.oracle.base == MARKET_BASE,
            "market config requires oracle base USD"
        );
        anyhow::ensure!(
            self.oracle.decimals == MARKET_SOURCE_DECIMALS,
            "market config requires source decimals = 7"
        );
        anyhow::ensure!(
            self.oracle.resolution == MARKET_SOURCE_RESOLUTION,
            "market config requires source resolution = 300"
        );
        anyhow::ensure!(
            self.adapter.decimals == MARKET_ADAPTER_DECIMALS,
            "market config requires adapter decimals = 7"
        );
        anyhow::ensure!(
            self.adapter.resolution == MARKET_ADAPTER_RESOLUTION,
            "market config requires adapter resolution = 1"
        );
        anyhow::ensure!(
            self.adapter.base == MARKET_BASE,
            "market config requires adapter base USD"
        );
        Ok(())
    }

    /// Every required artifact role must be pinned to an existing file.
    /// Types enforce role names and digest syntax; deployer preflight checks
    /// the artifact bytes against the digest.
    pub fn check_artifacts(&self) -> anyhow::Result<()> {
        for role in MARKET_ARTIFACT_ROLES {
            anyhow::ensure!(
                self.artifacts.contains_key(role),
                "market config is missing required artifact role '{role}'"
            );
        }
        for (role, pin) in &self.artifacts {
            anyhow::ensure!(
                expand_home(&pin.path).is_file(),
                "market config artifact '{role}' file is missing"
            );
        }
        Ok(())
    }

    /// Deploy-time gate: every scheduled reserve must set every parameter
    /// explicitly. Fixture defaults (`deploy::reserve_config`) exist for pair
    /// runs; a market that lends real value may not inherit an unreviewed
    /// cap, factor or rate curve. `load` does not call this so an existing
    /// market's config still attaches; only deploy and resume refuse.
    pub fn require_explicit_reserves(&self) -> anyhow::Result<()> {
        for tok in Tok::RESERVES {
            let missing = self
                .reserves
                .get(&tok)
                .map(ReserveOverride::missing_fields)
                .unwrap_or_else(|| ReserveOverride::default().missing_fields());
            anyhow::ensure!(
                missing.is_empty(),
                "market config reserve '{}' must set every parameter explicitly; missing: {}",
                tok.as_str(),
                missing.join(", ")
            );
        }
        Ok(())
    }
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Network facts a market manifest may persist: the exact testnet passphrase
/// and a credential-free label. Holds `Safe` literals only.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketNetworkRecord {
    pub label: Safe,
    pub passphrase: Safe,
}

/// Deployed proxy-oracle stack. Identifiers are checksum-validated newtypes
/// that bind verbatim; labels are tool-chosen `Safe` literals.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleStackManifest {
    pub runtime: ContractId,
    /// Proxy governance contract, or absent for a direct testnet bridge that
    /// has no governance contract. Omission is not an empty address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance: Option<ContractId>,
    pub adapter: ContractId,
    /// Three distinct deployed source contracts — an explicitly single-admin
    /// synthetic quorum, not independent data.
    pub sources: Vec<ContractId>,
    pub min_sources: u32,
    pub max_age_secs: u32,
    pub max_clock_drift_secs: u32,
    pub decimals: u32,
    pub resolution: u32,
    pub base: Safe,
}

/// Deployed single-market manifest (`market-manifest.toml`). Reuses `Tok`,
/// `Who`, and `ReserveRef` from the pair vocabulary but never the
/// serializable pair `Net`: only the exact testnet passphrase, a
/// credential-free RPC host label, the resolved admin G-address, and
/// validated contract identifiers are persisted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketManifest {
    pub network: MarketNetworkRecord,
    /// Resolved external operator identity — never the alias or config path.
    pub admin: AccountId,
    /// Credential-free RPC host label or redacted endpoint identity.
    pub rpc_host: Safe,
    pub oracle: OracleStackManifest,
    pub tokens: BTreeMap<Tok, ContractId>,
    pub pool: ContractId,
    pub backstop: ContractId,
    pub factory: ContractId,
    pub comet: ContractId,
    pub emitter: ContractId,
    pub reserves: BTreeMap<Tok, ReserveRef>,
    /// Run-local actor identities.
    pub identities: BTreeMap<Who, AccountId>,
    pub deploy_ledger: u32,
}

impl MarketManifest {
    /// Load the completed market manifest for a run directory. Parsing the
    /// strict shape is the whole contract: a partial or malformed file is
    /// an error, never a best-effort market.
    pub fn load(out: &Path) -> anyhow::Result<Self> {
        let p = out.join("market-manifest.toml");
        let text = std::fs::read_to_string(&p).context("read market manifest failed")?;
        toml::from_str(&text).map_err(|_| anyhow::anyhow!("market manifest is malformed"))
    }
}

/// Market evidence for one executed action: rendered text and validated
/// identifiers only, produced by the pinned `MarketRunner` conversion seam.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketStepRecord {
    /// Numbered deployment/play phase.
    pub phase: u32,
    /// Checked operation label: a tool literal for deployment/play or a
    /// bounded rendered role/method for a generic call. Never a path.
    pub action: Safe,
    /// Sanitized command summary, or the same checked generic-call label.
    pub command: Safe,
    /// Rendered outcome classification.
    pub outcome: Safe,
    /// Mutated or read contract where applicable.
    pub target: Option<ContractId>,
    pub violations: Vec<Safe>,
}

/// Mutation kinds, each with a distinct reconciliation proof (plan §4 resume law).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketMutationKind {
    Upload,
    Deploy,
    Invoke,
    FundKey,
    Config,
    Governance,
}

/// Pre-send intent and settled custody for one market mutation: rendered
/// `Safe` text, checksum-validated identifiers, and shape-validated hex
/// digests. Never an `Invocation`, `ErrorKind`, or `RawText`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketCustodyRecord {
    pub phase: u32,
    pub kind: MarketMutationKind,
    /// Checked operation label: tool literal for deployment/play; generic
    /// actions store bounded rendered role/method from the live ABI. Journal
    /// directories use numeric phase/slot coordinates, never this field.
    pub action: Safe,
    /// Exact testnet passphrase enforced while deserializing.
    pub network: MarketTestnet,
    /// Frozen signer for this mutation.
    pub signer: AccountId,
    /// Redacted normalized payload digest.
    pub payload_digest: Sha256Hex,
    /// Mutated contract where applicable.
    pub target: Option<ContractId>,
    /// Artifact or constructor digest where applicable.
    pub artifact_digest: Option<Sha256Hex>,
    /// Deterministic predicted contract id where applicable.
    pub predicted: Option<ContractId>,
    /// Rendered pre-state anchors.
    pub pre_state: Vec<Safe>,
    /// Rendered tool-failure text; `None` unless the tool itself failed.
    pub tool_error: Option<Safe>,
}

/// Settled result for one market mutation: exactly one record set — the
/// custody record plus its rendered step. A pair `SideOutcome` never reaches
/// a market journal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketPendingResult {
    pub custody: MarketCustodyRecord,
    pub step: MarketStepRecord,
}

/// Test-only coordinate path for asserting journal effects. Production opens
/// pinned descriptors and never trusts a caller-supplied path to a coordinate.
#[cfg(test)]
pub fn market_pending_dir(out: &Path, phase: u32, slot: u32) -> PathBuf {
    out.join("market-pending")
        .join(format!("{phase:03}-{slot:02}"))
}

/// Fixed input-free refusal for a record the pinned reader could never
/// return. Enforced before any journal directory or file is created.
pub const MARKET_JOURNAL_RECORD_OVERSIZE: &str =
    "market journal record exceeds the reader cap; nothing was written";

/// Enforce the shared reader cap before any journal write. This runs
/// before create_new for intent and result records alike, so no journal
/// write can strand an unreadable oversized record, and it never truncates
/// or fabricates content: the whole record is refused whole.
fn ensure_market_record_within_cap(bytes: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(
        bytes.len() <= crate::market_status::PENDING_RECORD_CAP,
        "{}",
        Safe::literal(MARKET_JOURNAL_RECORD_OVERSIZE)
    );
    Ok(())
}

/// Linux procfs supplies descriptor-relative paths without unsafe code; libc
/// supplies the per-architecture `O_*` values (x86_64 literals differ on arm).
/// Keep every directory pinned until the exclusive write and directory syncs
/// finish, then refuse a detectable rename/replacement before permitting a send.
fn write_market_pending(
    out: &Path,
    phase: u32,
    slot: u32,
    settled: bool,
    bytes: &[u8],
) -> anyhow::Result<()> {
    use std::io::{Error, ErrorKind, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let write = || -> std::io::Result<()> {
        let open_dir = |path: &Path| {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
                .open(path)
        };
        let create_dir = |path: &Path| match std::fs::create_dir(path) {
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
            result => result,
        };
        if !settled {
            std::fs::create_dir_all(out)?;
        }
        let out_dir = open_dir(out)?;
        let root_path = PathBuf::from(format!(
            "/proc/self/fd/{}/market-pending",
            out_dir.as_raw_fd()
        ));
        if !settled {
            create_dir(&root_path)?;
        }
        let root = open_dir(&root_path)?;
        let coordinate_path = PathBuf::from(format!(
            "/proc/self/fd/{}/{phase:03}-{slot:02}",
            root.as_raw_fd()
        ));
        if !settled {
            create_dir(&coordinate_path)?;
        }
        let dir = open_dir(&coordinate_path)?;
        let pinned = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd()));
        if !settled {
            match std::fs::symlink_metadata(pinned.join("result.json")) {
                Ok(_) => return Err(ErrorKind::AlreadyExists.into()),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let name = if settled {
            "result.json"
        } else {
            "intent.json"
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(pinned.join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        dir.sync_all()?;
        root.sync_all()?;
        out_dir.sync_all()?;
        for (path, held) in [
            (out, &out_dir),
            (root_path.as_path(), &root),
            (coordinate_path.as_path(), &dir),
        ] {
            let current = std::fs::symlink_metadata(path)?;
            let original = held.metadata()?;
            if !current.is_dir()
                || current.dev() != original.dev()
                || current.ino() != original.ino()
            {
                return Err(ErrorKind::Other.into());
            }
        }
        Ok(())
    };
    // Preserve AlreadyExists for the play runner's occupied-slot classifier,
    // but never expose paths, symlink targets, or operating-system text.
    write().map_err(|error| Error::new(error.kind(), "market pending journal unavailable").into())
}

/// Durably reserve a market mutation intent before any executor call.
/// `create_new` mirrors the pair journal: a replayed attempt fails before it
/// can reach the chain, even if a prior process died after opening the
/// intent but before recording its outcome.
pub fn begin_market_pending(
    out: &Path,
    phase: u32,
    slot: u32,
    record: &MarketCustodyRecord,
) -> anyhow::Result<()> {
    anyhow::ensure!(record.phase == phase, "market intent phase mismatch");
    // The reader re-derives this digest, so an unbound one (e.g. the
    // `Sha256Hex::default()` placeholder) would strand the coordinate.
    anyhow::ensure!(
        crate::market::custody_payload_digest(record)? == record.payload_digest,
        "market intent payload digest is not bound to its record"
    );
    let bytes = serde_json::to_vec_pretty(record)?;
    ensure_market_record_within_cap(&bytes)?;
    write_market_pending(out, phase, slot, false, &bytes)
}

/// Write the settled result for a market mutation after a definite outcome.
/// Persists exactly one record set (`MarketCustodyRecord` + `MarketStepRecord`).
pub fn finish_market_pending(
    out: &Path,
    phase: u32,
    slot: u32,
    result: &MarketPendingResult,
) -> anyhow::Result<()> {
    // The same pinned reader and settlement law used on resume must accept
    // these bytes before an append-only result can be written. Otherwise a
    // valid-looking run could become permanently unreadable at first send.
    let Some((intent, None)) = crate::market_status::read_journal_coordinate(out, phase, slot)?
    else {
        anyhow::bail!("market settlement requires an open pinned intent");
    };
    anyhow::ensure!(
        crate::market_status::verify_settled_custody((phase, slot), &intent, result),
        "market settlement does not match the frozen intent"
    );
    let bytes = serde_json::to_vec_pretty(result)?;
    ensure_market_record_within_cap(&bytes)?;
    write_market_pending(out, phase, slot, true, &bytes)
}

/// Read a journaled market intent for reconciliation. This delegates to the
/// strict pinned journal reader: validation runs on the stored bytes and the
/// canonical payload digest is re-derived, so an intent written by an older
/// build or edited on disk cannot be reconciled against a relaxed network or
/// a malformed digest. An absent coordinate is an error, not a default.
pub fn read_market_intent(
    out: &Path,
    phase: u32,
    slot: u32,
) -> anyhow::Result<MarketCustodyRecord> {
    crate::market_status::read_journal_coordinate(out, phase, slot)?
        .map(|(intent, _result)| intent)
        .ok_or_else(|| anyhow::anyhow!("market journal coordinate is unreserved"))
}

#[cfg(test)]
mod market_tests {
    use super::*;
    use crate::market::custody_payload_digest;
    use crate::market_status::journal_clear_for_action;
    use std::str::FromStr;

    /// Canonical zero-key StrKeys: checksum-valid so the item-7 newtypes
    /// accept them. Swap fixtures only if the boundary lane's `FromStr`
    /// disagrees — the journal logic under test is filesystem custody.
    const G_ADMIN: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
    const C_POOL: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";

    /// A temp dir following the existing lock-test convention.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("abtest-market-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A complete, exact config fixture with one dummy artifact file per role.
    /// Writes the artifact files into `dir` and returns the TOML text.
    fn exact_config(dir: &Path) -> String {
        let mut text = String::from(
            "[network]\n\
             passphrase = \"Test SDF Network ; September 2015\"\n\
             label = \"testnet\"\n\n",
        );
        for role in MARKET_ARTIFACT_ROLES {
            let file = dir.join(format!("{role}.wasm"));
            std::fs::write(&file, b"wasm").unwrap();
            text.push_str(&format!(
                "[artifacts.{role}]\n\
                 path = \"{}\"\n\
                 sha256 = \"{}\"\n\n",
                file.display(),
                "0".repeat(64)
            ));
        }
        text.push_str(
            "[oracle]\n\
             sources = 3\n\
             min_sources = 3\n\
             max_age_secs = 3600\n\
             max_clock_drift_secs = 60\n\
             base = \"USD\"\n\
             decimals = 7\n\
             resolution = 300\n\n\
             [adapter]\n\
             decimals = 7\n\
             resolution = 1\n\
             base = \"USD\"\n\n\
             [reserves.stable]\n\
             c_factor = 9000000\n\n\
             [[balances]]\n\
             who = \"frodo\"\n\
             token = \"stable\"\n\
             amount = 100000000000\n",
        );
        text
    }

    /// Load fixture text through the real parser. No executor is constructed
    /// anywhere in this module: every rejection below happens before any
    /// executor call by construction.
    fn parse(dir: &Path, text: &str) -> anyhow::Result<MarketConfig> {
        let file = dir.join("config.toml");
        std::fs::write(&file, text).unwrap();
        MarketConfig::load(&file)
    }

    #[test]
    fn market_config_parses_exact_testnet_fixture() {
        let dir = scratch("exact");
        let cfg = parse(&dir, &exact_config(&dir)).expect("exact fixture must load");
        assert_eq!(cfg.network.passphrase, MARKET_TESTNET_PASSPHRASE);
        assert_eq!(cfg.oracle.sources, 3);
        assert_eq!(cfg.oracle.min_sources, 3);
        assert_eq!(cfg.oracle.max_age_secs, 3600);
        assert_eq!(cfg.oracle.max_clock_drift_secs, 60);
        assert_eq!(cfg.adapter.decimals, 7);
        assert_eq!(cfg.adapter.resolution, 1);
        assert_eq!(cfg.artifacts.len(), MARKET_ARTIFACT_ROLES.len());
        assert_eq!(cfg.reserves[&Tok::Stable].c_factor, Some(9_000_000));
        assert_eq!(cfg.balances[0].who, Who::Frodo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_unknown_fields() {
        let dir = scratch("unknown");
        let mut text = exact_config(&dir);
        text.push_str("typo = 1\n");
        assert!(
            parse(&dir, &text).is_err(),
            "unknown top-level key must be rejected"
        );
        let mut nested = exact_config(&dir);
        nested
            .push_str("[[balances]]\nwho = \"frodo\"\ntoken = \"xlm\"\namount = 1\nextra = true\n");
        assert!(
            parse(&dir, &nested).is_err(),
            "unknown nested key must be rejected"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_missing_artifact_file() {
        let dir = scratch("missing-file");
        let text = exact_config(&dir).replace("pool.wasm", "pool-absent.wasm");
        let err = format!("{:#}", parse(&dir, &text).unwrap_err());
        assert!(err.contains("file is missing"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_wrong_digest() {
        let dir = scratch("digest");
        let text = exact_config(&dir).replace(&"0".repeat(64), "not-a-digest");
        let err = format!("{:#}", parse(&dir, &text).unwrap_err());
        assert!(
            err.contains("digest must be 64 lowercase hex characters"),
            "{err}"
        );
        let upper = exact_config(&dir).replace(&"0".repeat(64), &"A".repeat(64));
        assert!(
            parse(&dir, &upper).is_err(),
            "uppercase hex must be rejected"
        );
        // A missing or unknown role is equally fatal before any executor call.
        let short = exact_config(&dir).replace("[artifacts.adapter]", "[artifacts.adaptr]");
        assert!(
            parse(&dir, &short).is_err(),
            "missing role must be rejected"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_non_testnet_network() {
        let dir = scratch("mainnet");
        let text = exact_config(&dir).replace(
            "Test SDF Network ; September 2015",
            "Public Global Stellar Network ; September 2015",
        );
        let err = format!("{:#}", parse(&dir, &text).unwrap_err());
        assert!(err.contains("exact Stellar testnet passphrase"), "{err}");
        let label = exact_config(&dir).replace("label = \"testnet\"", "label = \"public\"");
        assert!(parse(&dir, &label).is_err(), "wrong network label rejected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_invalid_freshness() {
        let dir = scratch("freshness");
        let stale = exact_config(&dir).replace("\nmax_age_secs = 3600\n", "\nmax_age_secs = 60\n");
        assert!(
            parse(&dir, &stale).is_err(),
            "max_age_secs other than 3600 must be rejected before any executor call"
        );
        let drift = exact_config(&dir).replace(
            "\nmax_clock_drift_secs = 60\n",
            "\nmax_clock_drift_secs = 600\n",
        );
        assert!(
            parse(&dir, &drift).is_err(),
            "max_clock_drift_secs other than 60 must be rejected"
        );
        let quorum = exact_config(&dir).replace("\nmin_sources = 3\n", "\nmin_sources = 2\n");
        assert!(
            parse(&dir, &quorum).is_err(),
            "min_sources < 3 must be rejected"
        );
        let sources = exact_config(&dir).replace("\nsources = 3\n", "\nsources = 2\n");
        assert!(
            parse(&dir, &sources).is_err(),
            "source count != 3 must be rejected"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn market_config_rejects_invalid_metadata() {
        let dir = scratch("metadata");
        let bad_adapter = exact_config(&dir)
            .replace("\n[adapter]\ndecimals = 7\n", "\n[adapter]\ndecimals = 8\n");
        assert!(
            parse(&dir, &bad_adapter).is_err(),
            "adapter decimals other than 7 must be rejected"
        );
        let zero_resolution =
            exact_config(&dir).replace("\nresolution = 1\nbase", "\nresolution = 0\nbase");
        assert!(
            parse(&dir, &zero_resolution).is_err(),
            "zero adapter resolution must be rejected"
        );
        let bad_base = exact_config(&dir).replace(
            "\nresolution = 1\nbase = \"USD\"\n",
            "\nresolution = 1\nbase = \"EUR\"\n",
        );
        assert!(
            parse(&dir, &bad_base).is_err(),
            "adapter base other than USD must be rejected"
        );
        let bad_source_base = exact_config(&dir).replace(
            "\nbase = \"USD\"\ndecimals = 7\n",
            "\nbase = \"EUR\"\ndecimals = 7\n",
        );
        assert!(
            parse(&dir, &bad_source_base).is_err(),
            "source base other than USD must be rejected"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn custody(phase: u32) -> MarketCustodyRecord {
        let mut record = MarketCustodyRecord {
            phase,
            kind: MarketMutationKind::Deploy,
            action: Safe::literal("deploy-pool"),
            network: MarketTestnet::Testnet,
            signer: AccountId::from_str(G_ADMIN).unwrap(),
            payload_digest: Sha256Hex::default(),
            target: Some(ContractId::from_str(C_POOL).unwrap()),
            artifact_digest: Some("1".repeat(64).try_into().unwrap()),
            predicted: Some(ContractId::from_str(C_POOL).unwrap()),
            pre_state: vec![Safe::literal("pool status = 6")],
            tool_error: None,
        };
        record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
        record
    }

    fn test_safe(text: String) -> Safe {
        serde_json::from_value(serde_json::Value::String(text)).expect("test Safe text")
    }

    #[test]
    fn market_journal_reserves_intent_once_and_settles() {
        let dir = scratch("journal");
        let record = custody(9);
        begin_market_pending(&dir, 9, 0, &record).expect("first intent reserves");
        assert!(
            begin_market_pending(&dir, 9, 0, &record).is_err(),
            "a replayed attempt must fail before it can reach the chain"
        );
        assert!(
            !journal_clear_for_action(&dir),
            "an unsettled reservation must block admission"
        );
        assert!(market_pending_dir(&dir, 9, 0).join("intent.json").exists());
        assert!(!market_pending_dir(&dir, 9, 0).join("result.json").exists());
        let unbound_result = MarketPendingResult {
            custody: record.clone(),
            step: MarketStepRecord {
                phase: 9,
                action: record.action.clone(),
                command: Safe::literal("stellar contract deploy"),
                outcome: Safe::literal("ok"),
                target: None,
                violations: vec![],
            },
        };
        assert!(
            finish_market_pending(&dir, 9, 0, &unbound_result).is_err(),
            "writer must refuse a result the pinned reader could not settle"
        );
        assert!(
            !market_pending_dir(&dir, 9, 0).join("result.json").exists()
                && !journal_clear_for_action(&dir),
            "the refused result cannot close or alter the open intent"
        );
        finish_market_pending(
            &dir,
            9,
            0,
            &MarketPendingResult {
                custody: record.clone(),
                step: MarketStepRecord {
                    phase: 9,
                    action: Safe::literal("deploy-pool"),
                    command: Safe::literal("stellar contract deploy"),
                    outcome: Safe::literal("ok"),
                    target: record.target.clone(),
                    violations: vec![],
                },
            },
        )
        .expect("definite outcome settles");
        assert!(
            journal_clear_for_action(&dir),
            "definite settlement must clear admission"
        );
        // Custody is append-only: settlement retains the intent and the
        // settled result, and the occupied coordinate refuses reuse.
        let coordinate = market_pending_dir(&dir, 9, 0);
        let intent_path = coordinate.join("intent.json");
        let result_path = coordinate.join("result.json");
        let intent_before =
            std::fs::read(&intent_path).expect("settled custody retains the intent");
        let result_before =
            std::fs::read(&result_path).expect("settlement retains the settled result");
        let err = begin_market_pending(&dir, 9, 0, &record)
            .expect_err("a settled coordinate must refuse a new intent");
        assert_eq!(
            err.downcast_ref::<std::io::Error>().map(|e| e.kind()),
            Some(std::io::ErrorKind::AlreadyExists),
            "reuse of a settled coordinate is refused as an occupied slot"
        );
        assert_eq!(
            std::fs::read(&intent_path).expect("intent retained after refusal"),
            intent_before,
            "a refused reuse cannot alter the settled intent bytes"
        );
        assert_eq!(
            std::fs::read(&result_path).expect("result retained after refusal"),
            result_before,
            "a refused reuse cannot alter the settled result bytes"
        );
        assert!(
            journal_clear_for_action(&dir),
            "retained settlement evidence must keep admission clear"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The journal is addressed only by numbers: nothing a record or an
    /// on-disk name carries can move a write outside `market-pending/`.
    #[test]
    fn market_journal_anomalies_block_admission_and_are_never_removed() {
        let dir = scratch("stray");
        let root = dir.join("market-pending");
        // A malformed step name and a loose file are both custody anomalies:
        // they block play and are retained, never skipped or removed.
        std::fs::create_dir_all(root.join("00a-01")).unwrap();
        std::fs::write(root.join("loose-file"), b"x").unwrap();
        assert!(
            !journal_clear_for_action(&dir),
            "custody anomalies must never be skipped as clear"
        );
        assert!(
            root.join("00a-01").is_dir() && root.join("loose-file").is_file(),
            "a blocking anomaly stays where the operator can inspect it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Malformed persisted custody cannot exist as a value at all: the strict
    /// field types refuse it while deserializing, so no code path can skip a
    /// shape check. The wire shape is unchanged — a plain passphrase string and
    /// lowercase-hex digest strings.
    #[test]
    fn malformed_custody_fails_at_deserialization() {
        let record = custody(2);
        let base = serde_json::to_value(&record).expect("fixture custody serializes");
        assert_eq!(
            base["network"],
            serde_json::json!(MARKET_TESTNET_PASSPHRASE)
        );
        assert_eq!(
            base["artifact_digest"],
            serde_json::json!("1".repeat(64)),
            "digests must still serialize as plain strings"
        );
        for (field, value) in [
            (
                "network",
                serde_json::json!("Public Global Stellar Network ; September 2015"),
            ),
            ("network", serde_json::json!("testnet")),
            ("network", serde_json::json!(null)),
            ("payload_digest", serde_json::json!("short")),
            ("payload_digest", serde_json::json!("0".repeat(63))),
            ("payload_digest", serde_json::json!("A".repeat(64))),
            ("artifact_digest", serde_json::json!("zz")),
        ] {
            let mut tampered = base.clone();
            tampered[field] = value;
            assert!(
                serde_json::from_value::<MarketCustodyRecord>(tampered).is_err(),
                "{field} must be refused while deserializing, not afterwards"
            );
        }
        let bytes = serde_json::to_vec(&record).expect("custody serializes");
        let again: MarketCustodyRecord =
            serde_json::from_slice(&bytes).expect("a valid custody record parses back");
        assert_eq!(
            serde_json::to_vec(&again).expect("custody re-serializes"),
            bytes,
            "the custody wire format must stay byte-identical"
        );
    }

    /// Artifact roles, pinned digests, and minted amounts are refused while
    /// the config is parsed: an unknown role, a non-hex pin, or a
    /// non-positive amount can never become a `MarketConfig` value.
    #[test]
    fn market_config_refuses_malformed_values_at_parse() {
        let dir = scratch("typed-parse");
        let template = exact_config(&dir);
        let hex = format!("sha256 = \"{}\"", "0".repeat(64));
        let refuse = |note: &str, text: String| {
            assert!(
                parse(&dir, &text).is_err(),
                "{note} must be refused while parsing"
            );
        };
        refuse(
            "a non-hex artifact pin",
            template.replace(&hex, "sha256 = \"TBD\""),
        );
        refuse(
            "an uppercase artifact pin",
            template.replace(&hex, &format!("sha256 = \"{}\"", "A".repeat(64))),
        );
        refuse(
            "an unknown artifact role",
            template.replace("[artifacts.adapter]", "[artifacts.adaptr]"),
        );
        refuse(
            "a zero initial balance",
            template.replace("amount = 100000000000", "amount = 0"),
        );
        refuse(
            "a negative initial balance",
            template.replace("amount = 100000000000", "amount = -1"),
        );
        refuse(
            "a fractional initial balance",
            template.replace("amount = 100000000000", "amount = 1.5"),
        );
        // The accepted shape still serializes as plain TOML table names,
        // strings, and integers, so the frozen config digest is unchanged.
        let cfg = parse(&dir, &template).expect("exact config parses");
        let canonical = toml::to_string_pretty(&cfg).expect("config serializes");
        let bare = canonical.replace('"', "");
        for role in MARKET_ARTIFACT_ROLES {
            assert!(
                bare.contains(&format!("[artifacts.{role}]")),
                "{role} must serialize under its TOML table name: {canonical}"
            );
        }
        assert!(
            bare.contains(&format!("sha256 = {}", "0".repeat(64))),
            "an artifact digest must still serialize as a plain hex string: {canonical}"
        );
        assert!(
            bare.contains("amount = 100000000000"),
            "an initial balance must still serialize as a plain integer: {canonical}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A market intent may not be journaled under coordinates that disagree
    /// with the record it carries, nor with a digest unbound to its payload.
    #[test]
    fn market_journal_rejects_coordinate_mismatch_and_unbound_digest() {
        let dir = scratch("mismatch");
        assert!(begin_market_pending(&dir, 5, 0, &custody(7)).is_err());
        let unbound = MarketCustodyRecord {
            payload_digest: Sha256Hex::default(),
            ..custody(7)
        };
        assert!(begin_market_pending(&dir, 7, 0, &unbound).is_err());
        assert!(
            !dir.join("market-pending").exists(),
            "a rejected intent leaves no reservation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reconciliation reads the stored bytes, so an intent edited on disk —
    /// here onto a non-testnet network — cannot be reconciled as valid.
    #[test]
    fn market_journal_refuses_tampered_intent_on_read_back() {
        let dir = scratch("tamper");
        let record = custody(6);
        begin_market_pending(&dir, 6, 1, &record).expect("intent reserves");
        assert_eq!(
            read_market_intent(&dir, 6, 1).unwrap().phase,
            6,
            "an untouched intent reads back valid"
        );
        let path = dir
            .join("market-pending")
            .join("006-01")
            .join("intent.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let tampered = text.replace(
            "Test SDF Network ; September 2015",
            "Public Global Stellar Network ; September 2015",
        );
        assert_ne!(tampered, text, "fixture must actually carry the passphrase");
        std::fs::write(&path, &tampered).unwrap();
        assert!(
            read_market_intent(&dir, 6, 1).is_err(),
            "an edited intent must be refused before reconciliation"
        );
        assert!(
            read_market_intent(&dir, 6, 2).is_err(),
            "reading an absent intent is an error, not a default"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn refused_text(error: &anyhow::Error) -> String {
        format!("{error:#}")
    }

    fn assert_no_leak(error: &anyhow::Error, sentinel: &[u8], outside: &Path) {
        let text = refused_text(error);
        assert!(
            !text.contains(&outside.display().to_string())
                && !text.contains(std::str::from_utf8(sentinel).unwrap()),
            "refusal leaked a path or target: {text}"
        );
    }

    fn hostile_settlement(record: &MarketCustodyRecord) -> MarketPendingResult {
        MarketPendingResult {
            custody: record.clone(),
            step: MarketStepRecord {
                phase: record.phase,
                action: record.action.clone(),
                command: Safe::literal("test settlement"),
                outcome: Safe::literal("settled"),
                target: record.target.clone(),
                violations: Vec::new(),
            },
        }
    }

    /// A planted root symlink must fail before the writer can reserve durable
    /// intent through it. The separate victim proves any followed link would
    /// otherwise be writable.
    #[test]
    fn market_journal_refuses_symlinked_root_before_writing() {
        let dir = scratch("symlink-root-writer");
        let outside = scratch("outside-root-writer");
        let victim = outside.join("market-pending");
        std::fs::create_dir_all(victim.join("901-00")).unwrap();
        let sentinel = victim.join("do-not-touch");
        std::fs::write(&sentinel, b"outside-root-victim").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("market-pending")).unwrap();
        let error = begin_market_pending(&dir, 901, 0, &custody(901)).unwrap_err();
        assert_no_leak(&error, b"outside-root-victim", &outside);
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside-root-victim");
        assert!(!victim.join("901-00").join("intent.json").exists());
        assert!(!victim.join("901-00").join("result.json").exists());
        std::fs::remove_file(dir.join("market-pending")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// Both writer entry points must refuse a planted coordinate symlink, even
    /// when the custody record and journal root are otherwise valid.
    #[test]
    fn market_journal_refuses_symlinked_coordinate_before_writing() {
        let dir = scratch("symlink-coordinate-writer");
        let outside = scratch("outside-coordinate-writer");
        let victim = outside.join("902-00");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::create_dir_all(dir.join("market-pending")).unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("market-pending").join("902-00")).unwrap();
        let record = custody(902);
        let begin_error = begin_market_pending(&dir, 902, 0, &record).unwrap_err();
        assert_no_leak(&begin_error, b"outside-coordinate-victim", &outside);
        let finish_error =
            finish_market_pending(&dir, 902, 0, &hostile_settlement(&record)).unwrap_err();
        assert_no_leak(&finish_error, b"outside-coordinate-victim", &outside);
        assert_eq!(
            std::fs::read_dir(&victim)
                .map(|entries| entries.count())
                .unwrap_or(usize::MAX),
            0,
            "neither writer may create an intent or result in the victim"
        );
        assert!(
            !journal_clear_for_action(&dir),
            "the planted coordinate symlink stays a blocking custody anomaly"
        );
        assert!(!dir
            .join("market-pending")
            .join("902-00")
            .join("intent.json")
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// An intent that the pinned reader could never return must be
    /// refused before any journal directory or file is created.
    #[test]
    fn market_journal_refuses_oversized_intent_before_any_write() {
        let dir = scratch("oversized-intent");
        let mut record = custody(12);
        record.pre_state.push(test_safe("9".repeat(64_953)));
        record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
        let error = begin_market_pending(&dir, 12, 0, &record).unwrap_err();
        assert_eq!(error.to_string(), MARKET_JOURNAL_RECORD_OVERSIZE);
        assert!(!dir.join("market-pending").exists());
        assert!(!journal_clear_for_action(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A realistic escaped/Unicode pre-state that the pinned reader could
    /// never return must be refused whole, input-free, before any write.
    #[test]
    fn market_journal_refuses_escaped_oversized_pre_state_before_any_write() {
        let dir = scratch("oversized-escaped");
        let mut record = custody(13);
        record.pre_state.push(test_safe(format!(
            "pool-status={}\u{1f600}{}\u{1f600}",
            "\\|".repeat(20_311),
            "`".repeat(4_000)
        )));
        record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
        let error = begin_market_pending(&dir, 13, 0, &record).unwrap_err();
        assert_eq!(error.to_string(), MARKET_JOURNAL_RECORD_OVERSIZE);
        assert!(!dir.join("market-pending").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Intent records at the exact shared cap are reserved and remain
    /// reconcilable; the settlement for such an intent is legitimately
    /// larger, so the writer must refuse it and the attempt must stay
    /// unresolved custody (never fabricated, never unreadable).
    #[test]
    fn market_journal_intent_at_cap_stays_readable_and_oversized_result_refuses() {
        let dir = scratch("exact-cap");
        let mut record = custody(14);
        record.pre_state.push(test_safe("0".repeat(64_952)));
        record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
        assert_eq!(
            serde_json::to_vec_pretty(&record).unwrap().len(),
            crate::market_status::PENDING_RECORD_CAP,
            "fixture must sit exactly at the shared cap"
        );
        begin_market_pending(&dir, 14, 0, &record).expect("exact-cap intent reserves");
        assert_eq!(
            read_market_intent(&dir, 14, 0).unwrap().phase,
            14,
            "a record at the cap stays reconcilable"
        );
        let oversized = hostile_settlement(&record);
        assert!(
            serde_json::to_vec_pretty(&oversized).unwrap().len()
                > crate::market_status::PENDING_RECORD_CAP
        );
        let error = finish_market_pending(&dir, 14, 0, &oversized).unwrap_err();
        assert_no_leak(&error, b"0000000", &dir);
        assert!(!dir
            .join("market-pending")
            .join("014-00")
            .join("result.json")
            .exists());
        assert!(
            !journal_clear_for_action(&dir),
            "an unsettled attempt stays blocking custody"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A legitimate record whose settlement sits exactly at the shared
    /// cap is reserved, settled, and clears the action gate.
    #[test]
    fn market_journal_accepts_and_settles_record_exactly_at_cap() {
        let dir = scratch("exact-cap-result");
        let mut record = custody(16);
        record.pre_state.push(test_safe("0".repeat(64_687)));
        record.payload_digest = custody_payload_digest(&record).expect("fixture digest");
        let settlement = hostile_settlement(&record);
        assert_eq!(
            serde_json::to_vec_pretty(&settlement).unwrap().len(),
            crate::market_status::PENDING_RECORD_CAP,
            "settlement fixture must sit exactly at the shared cap"
        );
        begin_market_pending(&dir, 16, 0, &record).expect("limit-bound intent reserves");
        finish_market_pending(&dir, 16, 0, &settlement).expect("exact-cap settlement is written");
        assert!(journal_clear_for_action(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Result bytes are outcome-dependent: an oversized settlement is
    /// refused before create_new, and the open intent stays unresolved
    /// custody rather than becoming fabricated or unreadable evidence.
    #[test]
    fn market_journal_refuses_oversized_result_before_any_write() {
        let dir = scratch("oversized-result");
        let record = custody(15);
        begin_market_pending(&dir, 15, 0, &record).expect("open intent reserves");
        let mut stranding = custody(15);
        stranding.pre_state.push(test_safe("0".repeat(64_700)));
        stranding.payload_digest = custody_payload_digest(&stranding).expect("fixture digest");
        let oversized = hostile_settlement(&stranding);
        assert!(
            serde_json::to_vec_pretty(&oversized).unwrap().len()
                > crate::market_status::PENDING_RECORD_CAP
        );
        let error = finish_market_pending(&dir, 15, 0, &oversized).unwrap_err();
        assert_no_leak(&error, b"0000000", &dir);
        assert!(!dir
            .join("market-pending")
            .join("015-00")
            .join("result.json")
            .exists());
        assert!(
            !journal_clear_for_action(&dir),
            "an unsettled attempt stays blocking custody"
        );
        assert_eq!(read_market_intent(&dir, 15, 0).unwrap().phase, 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The result is a fixed filename inside a pinned coordinate. A planted
    /// symlink there must not become the settlement target, even though the
    /// record otherwise satisfies custody validation.
    #[test]
    fn market_journal_refuses_symlinked_result_before_writing() {
        let dir = scratch("symlink-result-writer");
        let outside = scratch("outside-result-writer");
        let victim = outside.join("settled.json");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(&victim, b"outside-result-victim").unwrap();
        let result = dir
            .join("market-pending")
            .join("903-00")
            .join("result.json");
        std::fs::create_dir_all(result.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, &result).unwrap();
        let record = custody(903);
        let begin_error = begin_market_pending(&dir, 903, 0, &record).unwrap_err();
        assert_no_leak(&begin_error, b"outside-result-victim", &outside);
        let finish_error =
            finish_market_pending(&dir, 903, 0, &hostile_settlement(&record)).unwrap_err();
        assert_no_leak(&finish_error, b"outside-result-victim", &outside);
        assert_eq!(std::fs::read(&victim).unwrap(), b"outside-result-victim");
        assert!(!dir
            .join("market-pending")
            .join("903-00")
            .join("intent.json")
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// Removing all hostile links must restore the ordinary durability path;
    /// refusal cannot become a way to evade journal custody.
    #[test]
    fn market_journal_settles_after_refusing_hostile_links() {
        let dir = scratch("legitimate-after-links");
        let record = custody(904);
        begin_market_pending(&dir, 904, 0, &record).expect("legitimate intent reserves");
        finish_market_pending(&dir, 904, 0, &hostile_settlement(&record))
            .expect("legitimate result settles");
        assert!(dir
            .join("market-pending")
            .join("904-00")
            .join("result.json")
            .exists());
        assert!(
            journal_clear_for_action(&dir),
            "legitimate settlement after hostile refusals must clear admission"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
