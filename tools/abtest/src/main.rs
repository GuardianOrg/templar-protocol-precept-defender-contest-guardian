mod compare;
mod deploy;
mod manifest;
mod market;
mod market_call;
mod market_explorer;
mod market_guides;
mod market_history;
mod market_query;
mod market_status;
mod observe;
mod report;
mod run;
mod scenario;
mod stellar;
mod timeline;
mod tui;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

use crate::deploy::{
    publication_is_unsettled, DeployAct, DeployEvent, MarketDeployer, MarketExecutionContext,
    MarketJournal,
};
use crate::manifest::{
    MarketConfig, MarketManifest, PairConfig, PairManifest, MARKET_NETWORK_LABEL,
    MARKET_TESTNET_PASSPHRASE,
};

use crate::market::{MarketAct, MarketEvent, MarketRunner};
use crate::stellar::{
    Capture, ContractId, Net, Policy, RealExecutor, Safe, Stellar, MARKET_CAPTURE_STDERR_CAP,
    MARKET_CAPTURE_STDOUT_CAP, MARKET_MUTATION_WALL, MARKET_READ_WALL,
};

#[derive(Parser)]
#[command(
    name = "precept-abtest",
    about = "Deploy or explore Stellar testnet markets through the stellar CLI"
)]
struct Cli {
    /// Path to the stellar CLI (>= 23 for pairs, >= 26.1.0 for market commands)
    #[arg(long, env = "STELLAR_BIN", default_value = "stellar")]
    stellar_bin: PathBuf,
    #[arg(
        long,
        env = "STELLAR_RPC_URL",
        default_value = "https://soroban-testnet.stellar.org"
    )]
    rpc_url: String,
    #[arg(
        long,
        env = "STELLAR_NETWORK_PASSPHRASE",
        default_value = "Test SDF Network ; September 2015"
    )]
    passphrase: String,
    /// Run directory. Defaults to the most recent one under ./runs
    #[arg(long)]
    out: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Deploy both sides and write manifest.toml
    Deploy {
        #[arg(long)]
        pair: PathBuf,
    },
    /// Run scenarios against the deployed pair
    Run {
        /// Comma-separated canonical subset of exactly one isolated scenario
        /// family, on a fresh deployment for that family; there is
        /// deliberately no "all"
        #[arg(long)]
        scenario: Option<String>,
        /// Resume from this step label (skips earlier steps)
        #[arg(long)]
        from_step: Option<String>,
        /// Emit one JSON object per event on stdout instead of progress lines
        #[arg(long)]
        json: bool,
    },
    /// Write report.md from retained evidence
    Report,
    /// List run directories, newest first
    List,
    /// Operator terminal: drive a run, or watch one read-only
    Tui {
        /// Driving requires this; a comma-separated canonical subset of one
        /// family. `--watch` may omit it
        #[arg(long)]
        scenario: Option<String>,
        #[arg(long)]
        from_step: Option<String>,
        /// Never take the run lock; browse or follow read-only
        #[arg(long)]
        watch: bool,
    },
    /// Single protected testnet market: deploy, resume, TUI, play, or explore contracts
    Market {
        #[command(subcommand)]
        cmd: MarketCmd,
    },
}

#[derive(Subcommand)]
enum MarketCmd {
    /// Deploy one market from a market config into a fresh run directory
    Deploy {
        /// Path to the market config TOML
        #[arg(long)]
        market: PathBuf,
        /// Existing stellar CLI identity alias used as the market administrator
        #[arg(long)]
        admin_identity: String,
        /// Explicit stellar CLI config directory holding that identity
        #[arg(long)]
        admin_config_dir: PathBuf,
    },
    /// Resume a partial market deployment under its declared reconciliation proofs
    Resume {
        #[arg(long)]
        market: PathBuf,
        #[arg(long)]
        admin_identity: String,
        #[arg(long)]
        admin_config_dir: PathBuf,
    },
    /// Drive deployment, browse completed markets, or open the guarded explorer
    Tui {
        #[arg(long, value_enum)]
        mode: MarketTuiMode,
        #[arg(long)]
        market: Option<PathBuf>,
        #[arg(long)]
        admin_identity: Option<String>,
        #[arg(long)]
        admin_config_dir: Option<PathBuf>,
        /// Optional signing alias for explore mode; omitted means reads only
        #[arg(long)]
        signer: Option<String>,
        /// Explicit Stellar identity directory when --signer is provided
        #[arg(long)]
        signer_config_dir: Option<PathBuf>,
    },
    /// Headless counterpart to post-deploy TUI market actions
    Play {
        #[arg(long)]
        market: PathBuf,
        #[arg(long)]
        admin_identity: String,
        #[arg(long)]
        admin_config_dir: PathBuf,
        /// Read-only snapshot of the deployed market
        #[arg(long)]
        snapshot: bool,
        /// Acknowledge the current halt; no chain call
        #[arg(long = "continue")]
        cont: bool,
        /// Stop the market runner
        #[arg(long)]
        halt: bool,
        /// Run one market action by its palette step index
        #[arg(long)]
        step: Option<usize>,
    },
    /// List the contract roles and ids frozen in a completed market manifest
    Contracts,
    /// Inspect a bound market contract's live interface
    Inspect {
        role: String,
        method: Option<String>,
    },
    /// Attach a caller-provided contract to a new non-reserved market role
    Attach { role: String, contract_id: String },
    /// Read market contract state through the pinned market CLI seam
    Query {
        /// Contract role bound in the completed market manifest
        role: String,
        /// Function visible in the target contract's live interface
        method: String,
        /// Read argument as name=value; repeat once per argument
        #[arg(long = "arg")]
        args: Vec<String>,
    },
    /// Simulate any live-ABI function, or explicitly confirm one journaled send
    Call {
        role: String,
        method: String,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long)]
        signer: String,
        #[arg(long)]
        signer_config_dir: PathBuf,
        /// After preview, require a fresh exact confirmation on stdin
        #[arg(long)]
        execute: bool,
    },
    /// List optional recipes; generic call is never restricted to this list
    Guides,
    /// Prepare a manifest-bound recipe through the same generic guarded call
    Guide {
        category: String,
        action: String,
        #[arg(long)]
        asset: Option<String>,
        #[arg(long)]
        source: Option<usize>,
        #[arg(long = "arg")]
        args: Vec<String>,
        #[arg(long)]
        signer: String,
        #[arg(long)]
        signer_config_dir: PathBuf,
        #[arg(long)]
        execute: bool,
    },
    /// Read the market health and price readiness report
    Status,
    /// Settle an open invoke intent proven never applied by consumed account sequences
    Settle {
        /// Journal coordinate, PHASE-SLOT (for example 900-45)
        coordinate: String,
        /// Unrelated transaction hash from the signer's contiguous sequence history
        #[arg(long = "consumed-by", required = true)]
        consumed_by: Vec<String>,
    },
    /// Read market events for a bound role from exactly one position
    Events {
        /// Contract role bound in the completed market manifest
        role: String,
        /// Opaque continuation cursor handed back as `next_cursor` by a
        /// prior page; exactly one of `--cursor` or `--start-ledger`
        #[arg(long)]
        cursor: Option<String>,
        /// First ledger sequence to read; exactly one of `--cursor` or
        /// `--start-ledger`
        #[arg(long)]
        start_ledger: Option<u32>,
        /// Number of events to read, 1 through 100; defaults to 100
        #[arg(long)]
        count: Option<u32>,
    },
    /// Read a transaction receipt by its 64-hex-character hash
    Receipt {
        /// Transaction hash, 64 hex characters
        hash: String,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum MarketTuiMode {
    Deploy,
    Resume,
    Watch,
    Explore,
}

/// Run-directory markers owned by a pair run. Their presence says a
/// directory is spoken for by the A/B machinery.
pub(crate) const PAIR_RUN_MARKERS: [&str; 4] =
    ["manifest.toml", "state.json", "pending", "evidence"];

/// Run-directory markers owned by a protected-market run.
pub(crate) const MARKET_RUN_MARKERS: [&str; 5] = [
    "deployment-plan.toml",
    "market-state.json",
    "market-manifest.toml",
    "market-pending",
    "market-evidence",
];

fn marker_conflict(out: &Path, markers: &[&'static str]) -> Option<&'static str> {
    markers.iter().copied().find(|m| out.join(m).exists())
}

/// Market run directories are not pair run directories, and vice versa.
/// Every market command calls this before any executor or context is
/// constructed; a directory holding either shape of prior run state is
/// rejected outright.
pub(crate) fn reject_pair_markers(out: &Path) -> anyhow::Result<()> {
    if let Some(marker) = marker_conflict(out, &PAIR_RUN_MARKERS) {
        anyhow::bail!(
            "selected run directory holds a pair deployment ({}); market commands need a market run directory",
            marker
        );
    }
    Ok(())
}

/// The other direction of the same wall: pair deploy/run/report/tui/list
/// reject any directory carrying market markers, checked before any
/// executor call.
pub(crate) fn reject_market_markers(out: &Path) -> anyhow::Result<()> {
    if let Some(marker) = marker_conflict(out, &MARKET_RUN_MARKERS) {
        anyhow::bail!(
            "run dir {} holds a protected-market deployment ({}); pair commands need a fresh pair run directory",
            out.display(),
            marker
        );
    }
    Ok(())
}

fn main() {
    std::process::exit(match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    })
}

/// Run directories under ./runs, newest first. Market run directories
/// carry no `manifest.toml`, so a pair `list` never even reports them —
/// part of the two-way directory rejection required before any executor
/// use; the explicit check below covers `--out` pointing at one.
fn find_runs() -> Vec<PathBuf> {
    let mut runs: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir("runs")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("manifest.toml").exists())
        .map(|e| {
            let t = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (t, e.path())
        })
        .collect();
    runs.sort_by(|a, b| b.0.cmp(&a.0));
    runs.into_iter().map(|(_, p)| p).collect()
}

fn list_runs() -> anyhow::Result<i32> {
    let runs = find_runs();
    if runs.is_empty() {
        println!("no runs under ./runs (deploy one first)");
        return Ok(0);
    }
    println!("{:<22} {:>6}  {:<12}  OWNER", "RUN", "STEPS", "PROFILE");
    for r in runs {
        let steps = walkdir_count(&r.join("evidence"));
        let profile = manifest::PairManifest::load(&r)
            .map(|m| format!("{:?}", m.profile))
            .unwrap_or_else(|_| "?".into());
        let owner = match manifest::RunLock::owner(&r) {
            Some(pid) => format!("running (pid {pid})"),
            None => "idle".into(),
        };
        println!(
            "{:<22} {:>6}  {:<12}  {}",
            r.file_name().unwrap_or_default().to_string_lossy(),
            steps,
            profile,
            owner
        );
    }
    Ok(0)
}

fn walkdir_count(evidence: &std::path::Path) -> usize {
    std::fs::read_dir(evidence)
        .into_iter()
        .flatten()
        .flatten()
        .map(|sc| {
            std::fs::read_dir(sc.path())
                .into_iter()
                .flatten()
                .flatten()
                .count()
        })
        .sum()
}

fn real_main() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    // Two-way marker wall: pair commands reject market dirs here (market
    // commands reject pair dirs inside their own handlers — see
    // `market_out`/`reject_pair_markers`). Market play/watch must be able
    // to name their own completed run directory, so the market-marker
    // rejection applies only on the pair path below.
    let is_market = matches!(cli.cmd, Cmd::Market { .. });
    if !is_market {
        if let Some(p) = &cli.out {
            reject_market_markers(p)?;
        }
    }
    if matches!(cli.cmd, Cmd::List) {
        return list_runs();
    }
    if let Cmd::Market { cmd } = &cli.cmd {
        return market_main(&cli, cmd);
    }
    let chosen = match &cli.out {
        Some(p) => p.clone(),
        None => find_runs().into_iter().next().context(
            "no --out given and no run found under ./runs; deploy one first, or pass --out",
        )?,
    };
    std::fs::create_dir_all(&chosen)?;
    let out = chosen.canonicalize()?;
    // Two-way run-directory wall, before any executor is constructed: a
    // market deployment owns this directory or nothing here speaks for it.
    reject_market_markers(&out)?;
    let net = Net {
        rpc_url: cli.rpc_url,
        passphrase: cli.passphrase,
        config_dir: out.join(".stellar"),
        headers: Vec::new(),
    };
    let exec = RealExecutor {
        bin: cli.stellar_bin.clone(),
        xdg: out.join(".xdg"),
        cwd: out.clone(),
        capture: Capture::Pair,
    };
    let st = Stellar::new(&net, &exec, Policy::Pair);
    match cli.cmd {
        Cmd::Deploy { pair } => {
            // A run dir with prior state would "resume" steps against contracts that
            // never ran them: deploy only into a fresh dir.
            anyhow::ensure!(
                !out.join("manifest.toml").exists()
                    && !out.join("state.json").exists()
                    && !out.join("pending").exists()
                    && !out.join("evidence").exists(),
                "run dir {} already holds a deployment; deploy to a new --out (e.g. runs/$(date +%s))",
                out.display()
            );
            st.assert_version()?;
            let cfg = PairConfig::load(&pair)?;
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            let m = deploy::deploy_pair(&st, &cfg, nonce)?;
            m.save(&out)?;
            println!(
                "deployed: a={} b={} manifest={}",
                m.a.contracts.pool,
                m.b.contracts.pool,
                PairManifest::path(&out).display()
            );
        }
        Cmd::Tui {
            scenario,
            from_step,
            watch,
        } => {
            if !watch && scenario.as_deref().unwrap_or("").is_empty() {
                anyhow::bail!(
                    "driving the TUI requires --scenario; use --watch to browse a run owned \
                     elsewhere; {}",
                    scenario::SELECTION_GUIDANCE
                );
            }
            return tui::main(
                &out,
                cli.stellar_bin,
                net,
                scenario.unwrap_or_default(),
                from_step,
                watch,
            );
        }
        Cmd::Run {
            scenario,
            from_step,
            json,
        } => {
            let _lock = manifest::RunLock::acquire(&out)?.with_context(|| {
                format!(
                    "run {} is owned by pid {}",
                    out.display(),
                    manifest::RunLock::owner(&out).unwrap_or(0)
                )
            })?;
            let m = PairManifest::load(&out).context("no manifest; run deploy first")?;
            let mut r = run::Runner::new(&st, &m, &out)?;
            let emit = |ev: run::Ev| -> run::Act {
                println!("{}", run::json_line(Policy::Pair, &ev));
                // No operator exists in --json mode: a halt is final, and it must
                // exit 1 when it was a tool failure rather than a semantic stop.
                match ev {
                    run::Ev::Halt { .. } => run::Act::Abort,
                    _ => run::Act::Continue,
                }
            };
            if json {
                r.hook = Some(&emit);
            }
            r.run(scenario.as_deref(), from_step.as_deref())?;
            let steps = report::load_steps(&out, &m)?;
            let verdict = report::write_markdown(&out, &m, &steps)?;
            println!("{verdict}: {}", out.join("report.md").display());
            if let Some(reason) = &r.fatal {
                eprintln!("tool failure: {reason}");
                return Ok(1);
            }
            return Ok(report::verdict_code(&verdict));
        }
        Cmd::List => unreachable!("handled above"),
        Cmd::Report => {
            let m = PairManifest::load(&out)?;
            let steps = report::load_steps(&out, &m)?;
            let verdict = report::write_markdown(&out, &m, &steps)?;
            println!("{verdict}: {}", out.join("report.md").display());
            return Ok(report::verdict_code(&verdict));
        }
        Cmd::Market { .. } => unreachable!("handled above"),
    }
    Ok(0)
}

fn market_out(cli: &Cli) -> anyhow::Result<PathBuf> {
    let chosen = cli
        .out
        .clone()
        .context("market commands require an explicit --out run directory")?;
    // The two-way wall is a create-new style check on the chosen path
    // before any directory is created, executor built, or context made.
    reject_pair_markers(&chosen)?;
    std::fs::create_dir_all(&chosen)?;
    let out = chosen.canonicalize()?;
    reject_pair_markers(&out)?;
    Ok(out)
}

fn market_context(
    cli: &Cli,
    out: &Path,
    alias: &str,
    admin_config_dir: &Path,
) -> anyhow::Result<MarketExecutionContext> {
    // SEC001: deployment, resumption, play, and TUI mutations must use the
    // canonical resolved CLI executable and a controlled, dotenv-free
    // working directory — never the caller-selected binary or a
    // caller-supplied run directory that a planted `.env` could hijack.
    // The executable is resolved from the invocation context before any
    // child could start; a planted `.env` anywhere on the boundary refuses
    // the whole context before executor construction (fail closed).
    let bin = market_explorer_bin(&cli.stellar_bin)?;
    anyhow::ensure!(
        std::fs::symlink_metadata(Path::new("/").join(".env")).is_err()
            && std::fs::symlink_metadata(out.join(".env")).is_err(),
        "{}",
        Safe::literal(MARKET_CWD_UNSAFE)
    );
    MarketExecutionContext::new(
        cli.rpc_url.clone(),
        Vec::new(),
        alias.to_string(),
        admin_config_dir.to_path_buf(),
        out.join(".stellar"),
        bin,
        out.join(".xdg"),
        PathBuf::from("/"),
    )
}

/// Completed manifests and caller networks must prove they are the exact
/// testnet before an explorer can bind to them. Rejections are input-free.
fn ensure_market_manifest_testnet(manifest: &MarketManifest) -> anyhow::Result<()> {
    anyhow::ensure!(
        manifest.network.label.to_string() == MARKET_NETWORK_LABEL
            && manifest.network.passphrase.to_string() == MARKET_TESTNET_PASSPHRASE,
        "{}",
        Safe::literal("market manifest network is not the exact testnet")
    );
    Ok(())
}

/// Fixed market-execution rejections. Both are input-free: the caller's
/// path or network bytes are never echoed back.
const MARKET_CLI_UNSAFE: &str = "market stellar CLI could not be resolved safely";
const MARKET_CWD_UNSAFE: &str = "market execution cwd is not a controlled dotenv-free directory";

/// Resolve the caller-supplied market CLI once, before changing to the
/// controlled execution directory. Relative paths are relative to the
/// invocation directory; bare names search only absolute PATH entries.
/// Every market child receives an absolute regular-file executable.
fn market_explorer_bin(requested: &Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    anyhow::ensure!(
        !requested.as_os_str().is_empty(),
        "{}",
        Safe::literal(MARKET_CLI_UNSAFE)
    );
    let selected = if requested.is_absolute()
        || requested
            .to_string_lossy()
            .contains(std::path::MAIN_SEPARATOR)
    {
        requested.to_path_buf()
    } else {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .filter(|dir| dir.is_absolute())
                    .map(|dir| dir.join(requested))
                    .find(|candidate| {
                        candidate.metadata().is_ok_and(|meta| {
                            meta.is_file() && meta.permissions().mode() & 0o111 != 0
                        })
                    })
            })
            .ok_or_else(|| anyhow::anyhow!("{}", Safe::literal(MARKET_CLI_UNSAFE)))?
    };
    let resolved = selected
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("{}", Safe::literal(MARKET_CLI_UNSAFE)))?;
    anyhow::ensure!(
        resolved
            .metadata()
            .is_ok_and(|meta| { meta.is_file() && meta.permissions().mode() & 0o111 != 0 }),
        "{}",
        Safe::literal(MARKET_CLI_UNSAFE)
    );
    Ok(resolved)
}

/// The CLI auto-loads `.env` from its working directory, and a
/// `STELLAR_CONTRACT_ID` in one hijacks the `--id` that binds every
/// market read to the completed manifest. Market reads therefore run
/// from the controlled root, never a caller-supplied run directory,
/// and fail before any network call when that isolation is not real.
fn market_explorer_net(cli: &Cli, out: &Path) -> anyhow::Result<(Net, RealExecutor)> {
    anyhow::ensure!(
        cli.passphrase == MARKET_TESTNET_PASSPHRASE,
        "{}",
        Safe::literal("market execution network is not the exact testnet")
    );
    let bin = market_explorer_bin(&cli.stellar_bin)?;
    anyhow::ensure!(
        std::fs::symlink_metadata(std::path::Path::new("/").join(".env")).is_err(),
        "{}",
        Safe::literal(MARKET_CWD_UNSAFE)
    );
    let net = Net {
        rpc_url: cli.rpc_url.clone(),
        passphrase: cli.passphrase.clone(),
        config_dir: out.join(".stellar"),
        headers: Vec::new(),
    };
    let exec = RealExecutor {
        bin,
        xdg: out.join(".xdg"),
        cwd: PathBuf::from("/"),
        capture: Capture::Market {
            stdout_cap: MARKET_CAPTURE_STDOUT_CAP,
            stderr_cap: MARKET_CAPTURE_STDERR_CAP,
            wall: Some(MARKET_READ_WALL),
        },
    };
    Ok((net, exec))
}

/// Read surfaces require an existing completed run, never a new directory.
fn market_read_context(cli: &Cli) -> anyhow::Result<(PathBuf, MarketManifest, Net, RealExecutor)> {
    let chosen = cli
        .out
        .as_ref()
        .context("market commands require an explicit --out run directory")?;
    reject_pair_markers(chosen)?;
    let out = chosen
        .canonicalize()
        .context("market reads require an existing run directory")?;
    reject_pair_markers(&out)?;
    // Defense in depth on the dotenv boundary: market execution already
    // runs from the controlled root, so a run-directory `.env` cannot
    // reach the CLI. A market run directory is tool-owned and nothing in
    // this tool ever writes one, so its presence means foreign state was
    // planted where the CLI's `.env` auto-load and `STELLAR_CONTRACT_ID`
    // hijack would have found it. Reject it before any executor exists.
    anyhow::ensure!(
        std::fs::symlink_metadata(out.join(".env")).is_err(),
        "{}",
        Safe::literal(MARKET_CWD_UNSAFE)
    );
    let manifest =
        MarketManifest::load(&out).context("market reads need a completed market run directory")?;
    ensure_market_manifest_testnet(&manifest)?;
    let (net, exec) = market_explorer_net(cli, &out)?;
    Ok((out, manifest, net, exec))
}
/// Action contexts reuse the read-only testnet/dotenv boundary, then bind
/// native CLI signing to one explicitly selected identity directory.
fn market_action_context(
    cli: &Cli,
    signer_config_dir: &Path,
) -> anyhow::Result<(PathBuf, MarketManifest, Net, RealExecutor)> {
    let (out, manifest, mut net, mut exec) = market_read_context(cli)?;
    let dir = signer_config_dir.canonicalize().map_err(|_| {
        anyhow::anyhow!(
            "{}",
            Safe::literal("market signer directory is unavailable")
        )
    })?;
    anyhow::ensure!(
        dir.is_dir(),
        "{}",
        Safe::literal("market signer directory is unavailable")
    );
    net.config_dir = dir;
    exec.capture = Capture::Market {
        stdout_cap: MARKET_CAPTURE_STDOUT_CAP,
        stderr_cap: MARKET_CAPTURE_STDERR_CAP,
        wall: Some(MARKET_MUTATION_WALL),
    };
    Ok((out, manifest, net, exec))
}

/// A single owner drives deployment, play, or a confirmed action per market
/// run. Watch and read-only exploration deliberately never acquire it.
/// Intentional exclusion: a deploy/resume/play driver must finish or be
/// stopped before another process can prepare an explorer action. Even an
/// emergency governance call cannot interleave its journal writes.
fn market_drive_lock(out: &Path) -> anyhow::Result<manifest::RunLock> {
    manifest::RunLock::acquire(out)
        .map_err(|_| anyhow::anyhow!("{}", Safe::literal("market run lock is unavailable")))?
        .ok_or_else(|| anyhow::anyhow!("{}", Safe::literal("another process owns this market run")))
}

fn market_explorer_call<T>(result: anyhow::Result<T>) -> anyhow::Result<T> {
    result.map_err(|e| anyhow::anyhow!("{}", Safe::error_chain(Policy::Market, &e)))
}

/// Print one deployment event. Headless deployment and the TUI observe the
/// same event grammar; only the presentation differs.
fn print_deploy_event(ev: &DeployEvent) {
    match ev {
        DeployEvent::Phase { id, label } => println!("phase {id}: {label}"),
        DeployEvent::Confirm { summary, .. } => {
            println!("CONFIRM REQUIRED: {summary}");
            println!("nothing proceeds until the operator confirms; type exactly y");
        }
        DeployEvent::Custody { unresolved, note } => {
            println!("custody unresolved={unresolved}: {note}")
        }
        DeployEvent::Done(manifest) => {
            print_market_summary(manifest);
        }
        DeployEvent::Failed(reason) => eprintln!("deployment failed: {reason}"),
    }
}

fn print_market_summary(m: &MarketManifest) {
    println!("market deployment complete; the market is active on Stellar testnet only");
    println!("network: {}", m.network.label);
    println!("pool administrator (public G-address): {}", m.admin);
    println!("rpc endpoint (redacted): {}", m.rpc_host);
    println!("proxy runtime: {}", m.oracle.runtime);
    if let Some(governance) = &m.oracle.governance {
        println!("proxy governance: {governance}");
    }
    println!("oracle adapter (immutable): {}", m.oracle.adapter);
    for (n, source) in m.oracle.sources.iter().enumerate() {
        println!("oracle source {}: {source}", n + 1);
    }
    println!("oracle base: {}", m.oracle.base);
    println!(
        "single-authority synthetic quorum: {} sources, min_sources {}",
        m.oracle.sources.len(),
        m.oracle.min_sources
    );
    for (tok, id) in &m.tokens {
        println!("token {tok:?}: {id}");
    }
    println!("comet: {}", m.comet);
    println!("emitter: {}", m.emitter);
    println!("backstop: {}", m.backstop);
    println!("factory: {}", m.factory);
    println!("pool: {}", m.pool);
    for (tok, r) in &m.reserves {
        println!("reserve {tok:?}: index {} decimals {}", r.index, r.decimals);
    }
    for (who, id) in &m.identities {
        println!("identity {who:?}: {id}");
    }
    println!("deploy ledger height: {}", m.deploy_ledger);
}

/// Headless pump over the shared deployment state machine. Identical
/// engine to `market tui`; the only difference is that the operator's
/// confirmation arrives on stdin here instead of a TUI key.
fn market_pump(deployer: &mut MarketDeployer) -> anyhow::Result<i32> {
    let mut act = DeployAct::Init;
    loop {
        let ev = match deployer.step(act) {
            Ok(ev) => ev,
            Err(e) => {
                eprintln!(
                    "deployment stopped: {}",
                    stellar::Safe::error_chain(Policy::Market, &e)
                );
                return Ok(1);
            }
        };
        if matches!(&ev, DeployEvent::Confirm { .. }) {
            use std::io::IsTerminal;
            anyhow::ensure!(
                std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
                "market deployment needs an interactive terminal for exact confirmation"
            );
            drain_market_confirmation_input().map_err(|_| {
                anyhow::anyhow!("market deployment not sent: terminal input could not be cleared")
            })?;
        }
        print_deploy_event(&ev);
        match ev {
            DeployEvent::Done(_) => return Ok(0),
            DeployEvent::Failed(_) => return Ok(1),
            DeployEvent::Custody {
                unresolved: true, ..
            } => return Ok(1),
            DeployEvent::Confirm { token, .. } => {
                if !confirm_on_stdin()? {
                    let stop = deployer.step(DeployAct::Halt);
                    if let Ok(ev) = stop {
                        print_deploy_event(&ev);
                    }
                    return Ok(1);
                }
                act = DeployAct::Confirm { token };
                continue;
            }
            _ => {}
        }
        act = DeployAct::Continue;
    }
}

/// Exactly one `y` line confirms a deployment gate; anything else —
/// including an empty line (Enter alone) — refuses it. The binding
/// confirmation summary is already on screen via `print_deploy_event`; the
/// token itself is never echoed, so no bound authorization material leaks
/// to the terminal, CI logs, or the PTY capture. The gate re-checks the
/// token, so a stale refusal cannot satisfy a later phase.
fn confirm_on_stdin() -> anyhow::Result<bool> {
    println!("type exactly y then Enter to confirm; anything else refuses this mutation");
    let mut line = String::new();
    let read = std::io::stdin()
        .read_line(&mut line)
        .context("reading confirmation")?;
    let answer = line.trim_end_matches(['\r', '\n']);
    Ok(read > 0 && answer == "y")
}

/// Ceiling for renders of parsed market values, matching the explorer's
/// own bounded render cap. Every market read passes through this cap.
const MARKET_READ_RENDER_CAP: usize = 65_536;

/// Inclusive ceiling for `market events --count`.
const MARKET_EVENTS_MAX_COUNT: u32 = 100;

/// Trust-boundary guard for raw `market query --arg` inputs. Each input
/// is split exactly once at its first `=` into owned name and value, and
/// the collected pairs pass through the shared `market_query` validator
/// before any run context or invoke. Every rejection here is input-free:
/// the caller's bytes are never echoed.
fn parse_market_query_args(args: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    anyhow::ensure!(
        args.len() <= market_query::MAX_QUERY_ARGS,
        "{}",
        Safe::literal("market query accepts at most 32 --arg inputs")
    );
    let mut parsed = Vec::with_capacity(args.len());
    for raw in args {
        let Some((name, value)) = raw.split_once('=') else {
            anyhow::bail!(
                "{}",
                Safe::literal("market query arguments must use --arg name=value")
            );
        };
        parsed.push((name.to_string(), value.to_string()));
    }
    market_query::validate_args(&parsed)?;
    Ok(parsed)
}

/// Discard typeahead before displaying a market send preview. Raw mode makes
/// even an unfinished line readable. Restore line input on failure as well.
fn drain_market_confirmation_input() -> std::io::Result<()> {
    use ratatui::crossterm::{event, terminal};
    terminal::enable_raw_mode()?;
    let drained = (|| {
        while event::poll(std::time::Duration::ZERO)? {
            event::read()?;
        }
        Ok(())
    })();
    let restored = terminal::disable_raw_mode();
    drained.and(restored)
}

/// CLI and TUI use the same move-only action capability. This presenter
/// neither builds transactions nor interprets a failed simulation as consent.
#[allow(clippy::too_many_arguments)]
fn market_action_pump(
    st: &Stellar<'_, RealExecutor>,
    out: &Path,
    manifest: &MarketManifest,
    role: &str,
    method: &str,
    args: &[(String, String)],
    signer: &str,
    execute: bool,
) -> anyhow::Result<i32> {
    use std::io::IsTerminal;
    anyhow::ensure!(
        !execute || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal()),
        "{}",
        Safe::literal("market send requires an interactive terminal for exact confirmation")
    );
    let prepared = market_explorer_call(market_call::prepare(
        st, out, manifest, role, method, args, signer,
    ))?;
    if execute {
        drain_market_confirmation_input().map_err(|_| {
            anyhow::anyhow!(
                "{}",
                Safe::literal("market action not sent: terminal input could not be cleared")
            )
        })?;
    }
    println!("{}", prepared.summary());
    if !execute {
        return Ok(0);
    }
    if !confirm_on_stdin()? {
        eprintln!("market action not sent");
        return Ok(1);
    }
    let token: [u8; 64] =
        prepared.confirmation().as_bytes().try_into().map_err(|_| {
            anyhow::anyhow!("{}", Safe::literal("market confirmation is malformed"))
        })?;
    let token = std::str::from_utf8(&token)
        .map_err(|_| anyhow::anyhow!("{}", Safe::literal("market confirmation is malformed")))?;
    let result = market_explorer_call(market_call::execute(st, out, manifest, prepared, token))?;
    println!("outcome: {}", result.outcome);
    if let Some(value) = result.value {
        let rendered =
            market_explorer_call(Safe::render_market_json(&value, MARKET_READ_RENDER_CAP))?;
        println!("value: {rendered}");
    }
    Ok(if result.accepted { 0 } else { 1 })
}

fn market_main(cli: &Cli, cmd: &MarketCmd) -> anyhow::Result<i32> {
    match cmd {
        MarketCmd::Deploy {
            market,
            admin_identity,
            admin_config_dir,
        } => {
            let out = market_out(cli)?;
            let _lock = market_drive_lock(&out)?;
            // Fresh-directory rule: no market or pair markers may exist
            // before the first executor call.
            if let Some(marker) = marker_conflict(&out, &MARKET_RUN_MARKERS) {
                anyhow::bail!(
                    "run dir {} already holds market state ({}); market deploy needs a fresh directory",
                    out.display(),
                    marker
                );
            }
            let cfg = MarketConfig::load(market)?;
            let ctx = market_context(cli, &out, admin_identity, admin_config_dir)?;
            ctx.assert_testnet()?;
            let journal = MarketJournal::open(out.as_path());
            let mut deployer = MarketDeployer::new(cfg, ctx, journal)?;
            market_pump(&mut deployer)
        }
        MarketCmd::Resume {
            market,
            admin_identity,
            admin_config_dir,
        } => {
            let out = market_out(cli)?;
            let _lock = market_drive_lock(&out)?;
            anyhow::ensure!(
                out.join("deployment-plan.toml").exists(),
                "no deployment plan in {}: market resume only reconciles a market deploy that started",
                out.display()
            );
            anyhow::ensure!(
                !out.join("market-manifest.toml").exists() || publication_is_unsettled(&out)?,
                "{} is already a completed market run; market resume needs a partial deployment",
                out.display()
            );
            let cfg = MarketConfig::load(market)?;
            let ctx = market_context(cli, &out, admin_identity, admin_config_dir)?;
            ctx.assert_testnet()?;
            let journal = MarketJournal::open(out.as_path());
            let mut deployer = MarketDeployer::new(cfg, ctx, journal)?;
            market_pump(&mut deployer)
        }
        MarketCmd::Tui {
            mode,
            market,
            admin_identity,
            admin_config_dir,
            signer,
            signer_config_dir,
        } => {
            let out = market_out(cli)?;
            match mode {
                MarketTuiMode::Watch => {
                    anyhow::ensure!(
                        market.is_none()
                            && admin_identity.is_none()
                            && admin_config_dir.is_none()
                            && signer.is_none()
                            && signer_config_dir.is_none(),
                        "market tui --mode watch takes no signing or deployment flags; it is read-only"
                    );
                    let manifest = MarketManifest::load(&out).context(
                        "no completed market manifest; watch needs a finished market run",
                    )?;
                    tui::main_market_watch(manifest)
                }
                MarketTuiMode::Explore => {
                    anyhow::ensure!(
                        market.is_none()
                            && admin_identity.is_none()
                            && admin_config_dir.is_none()
                            && signer.is_some() == signer_config_dir.is_some(),
                        "{}",
                        Safe::literal("market explore accepts either both --signer and --signer-config-dir or neither, and no deployment flags")
                    );
                    let (out, manifest, net, exec) = match signer_config_dir {
                        Some(dir) => market_action_context(cli, dir)?,
                        None => market_read_context(cli)?,
                    };
                    tui::market_explorer::main_market_explorer(
                        out,
                        manifest,
                        net,
                        exec,
                        signer.clone(),
                    )
                }
                MarketTuiMode::Deploy | MarketTuiMode::Resume => {
                    anyhow::ensure!(
                        signer.is_none() && signer_config_dir.is_none(),
                        "market deployment TUI uses --admin-identity and --admin-config-dir, not explorer signing flags"
                    );
                    let mode_is_deploy = *mode == MarketTuiMode::Deploy;
                    let market = market.clone().context("market tui requires --market")?;
                    let identity = admin_identity
                        .clone()
                        .context("market tui requires --admin-identity")?;
                    let config_dir = admin_config_dir
                        .clone()
                        .context("market tui requires --admin-config-dir")?;
                    if mode_is_deploy {
                        if let Some(marker) = marker_conflict(&out, &MARKET_RUN_MARKERS) {
                            anyhow::bail!(
                                "run dir {} already holds market state ({}); market tui --mode deploy needs a fresh directory",
                                out.display(),
                                marker
                            );
                        }
                    } else {
                        anyhow::ensure!(
                            out.join("deployment-plan.toml").exists(),
                            "no deployment plan in {}; market tui --mode resume only reconciles a market deploy that started",
                            out.display()
                        );
                        anyhow::ensure!(
                            !out.join("market-manifest.toml").exists()
                                || publication_is_unsettled(&out)?,
                            "{} is already a completed market run; market tui --mode resume needs a partial deployment",
                            out.display()
                        );
                    }
                    let _lock = market_drive_lock(&out)?;
                    let cfg = MarketConfig::load(&market)?;
                    let ctx = market_context(cli, &out, &identity, &config_dir)?;
                    ctx.assert_testnet()?;
                    let journal = MarketJournal::open(out.as_path());
                    let deployer = MarketDeployer::new(cfg, ctx, journal)?;
                    tui::main_market_deploy(deployer)
                }
            }
        }
        MarketCmd::Play {
            market,
            admin_identity,
            admin_config_dir,
            snapshot,
            cont,
            halt,
            step,
        } => {
            let chosen = cli
                .out
                .clone()
                .context("market play requires --out naming the completed market run")?;
            reject_pair_markers(&chosen)?;
            let out = chosen
                .canonicalize()
                .with_context(|| format!("cannot open run dir {}", chosen.display()))?;
            reject_pair_markers(&out)?;
            let _lock = market_drive_lock(&out)?;
            let manifest = MarketManifest::load(&out)
                .context("market play needs a completed market run directory")?;
            let acts_chosen = [*snapshot, *cont, *halt, step.is_some()]
                .into_iter()
                .filter(|x| *x)
                .count();
            anyhow::ensure!(
                acts_chosen == 1,
                "market play takes exactly one action: --snapshot, --continue, --halt, or --step <index>"
            );
            MarketConfig::load(market)?;
            let ctx = market_context(cli, &out, admin_identity, admin_config_dir)?;
            ctx.assert_testnet()?;
            let journal = MarketJournal::open(out.as_path());
            let mut runner = MarketRunner::new(manifest, ctx, journal)?;
            let act = if *snapshot {
                MarketAct::Snapshot
            } else if *cont {
                MarketAct::Continue
            } else if *halt {
                MarketAct::Halt
            } else {
                MarketAct::Run((*step).context("internal: step index already validated")?)
            };
            let ev = runner.act(act).map_err(|e| {
                // Market action failures are surfaced sanitized: the chain
                // is composed under the market policy, never raw.
                anyhow::anyhow!("{}", stellar::Safe::error_chain(Policy::Market, &e))
            })?;
            match ev {
                MarketEvent::Phase { label } => println!("phase: {label}"),
                MarketEvent::Cli { argv } => println!("cli: {argv}"),
                MarketEvent::Step { label, ok } => {
                    println!("step {label}: {}", if ok { "ok" } else { "failed" });
                    if !ok {
                        return Ok(1);
                    }
                }
                MarketEvent::Halt { reason } => {
                    eprintln!("market runner halted: {reason}");
                    return Ok(1);
                }
                MarketEvent::Failed(reason) => {
                    eprintln!("market action failed: {reason}");
                    return Ok(1);
                }
            }
            Ok(0)
        }
        MarketCmd::Contracts => {
            let out = market_out(cli)?;
            let manifest = MarketManifest::load(&out)
                .context("market contracts needs a completed market run directory")?;
            ensure_market_manifest_testnet(&manifest)?;
            for (role, contract_id) in market_explorer::roles(&manifest) {
                println!("{role} {contract_id}");
            }
            Ok(0)
        }
        MarketCmd::Inspect { role, method } => {
            let out = market_out(cli)?;
            let manifest = MarketManifest::load(&out)
                .context("market inspect needs a completed market run directory")?;
            ensure_market_manifest_testnet(&manifest)?;
            let (net, exec) = market_explorer_net(cli, &out)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            let output = market_explorer_call(market_explorer::inspect(
                &st,
                &out,
                &manifest,
                role,
                method.as_deref(),
            ))?;
            println!("{output}");
            Ok(0)
        }
        MarketCmd::Attach { role, contract_id } => {
            let out = market_out(cli)?;
            let manifest = MarketManifest::load(&out)
                .context("market attach needs a completed market run directory")?;
            ensure_market_manifest_testnet(&manifest)?;
            let contract_id = ContractId::from_str(contract_id)
                .map_err(|_| anyhow::anyhow!("invalid contract id"))?;
            let (net, exec) = market_explorer_net(cli, &out)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            market_explorer_call(market_explorer::attach(
                &st,
                &out,
                &manifest,
                role,
                contract_id,
            ))?;
            println!("market contract attached");
            Ok(0)
        }
        MarketCmd::Query { role, method, args } => {
            let args = parse_market_query_args(args)?;
            let (out, manifest, net, exec) = market_read_context(cli)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            // The query service self-gates the exact testnet and the pinned
            // CLI before composing under the market policy.
            let value = market_explorer_call(market_query::query(
                &st, &out, &manifest, role, method, &args,
            ))?;
            let output =
                market_explorer_call(Safe::render_market_json(&value, MARKET_READ_RENDER_CAP))?;
            println!("{output}");
            Ok(0)
        }
        MarketCmd::Call {
            role,
            method,
            args,
            signer,
            signer_config_dir,
            execute,
        } => {
            let args = parse_market_query_args(args)?;
            let (out, manifest, net, exec) = market_action_context(cli, signer_config_dir)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            market_action_pump(&st, &out, &manifest, role, method, &args, signer, *execute)
        }
        MarketCmd::Guides => {
            for (category, action, role, method, help) in market_guides::catalog() {
                println!("{category} {action} [{role}.{method}]: {help}");
            }
            Ok(0)
        }
        MarketCmd::Guide {
            category,
            action,
            asset,
            source,
            args,
            signer,
            signer_config_dir,
            execute,
        } => {
            let args = parse_market_query_args(args)?;
            let (out, manifest, net, exec) = market_action_context(cli, signer_config_dir)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            let (role, method, args) = market_explorer_call(market_guides::guide(
                &st,
                &out,
                &manifest,
                category,
                action,
                asset.as_deref(),
                *source,
                &args,
            ))?;
            market_action_pump(
                &st, &out, &manifest, &role, &method, &args, signer, *execute,
            )
        }
        MarketCmd::Status => {
            let (out, manifest, net, exec) = market_read_context(cli)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            market_explorer_call(st.assert_market_cli_version().map(|_| ()))?;
            let (report, ready) =
                market_explorer_call(market_status::status(&st, &out, &manifest))?;
            println!("{report}");
            anyhow::ensure!(ready, "{}", Safe::literal("market prices are not ready"));
            Ok(0)
        }
        MarketCmd::Settle {
            coordinate,
            consumed_by,
        } => {
            let (out, _manifest, net, exec) = market_read_context(cli)?;
            let _lock = market_drive_lock(&out)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            let summary =
                market_explorer_call(market_history::settle(&st, &out, coordinate, consumed_by))?;
            println!("{summary}");
            Ok(0)
        }
        MarketCmd::Events {
            role,
            cursor,
            start_ledger,
            count,
        } => {
            // Exactly one continuation position, and every argument rule is
            // settled before any run directory, executor, or network call.
            anyhow::ensure!(
                cursor.is_some() ^ start_ledger.is_some(),
                "{}",
                Safe::literal("market events requires exactly one of --cursor or --start-ledger")
            );
            let count = count.unwrap_or(MARKET_EVENTS_MAX_COUNT);
            anyhow::ensure!(
                (1..=MARKET_EVENTS_MAX_COUNT).contains(&count),
                "{}",
                Safe::literal("market events --count must be between 1 and 100")
            );
            // A continuation handle is checked against the RPC event-id
            // grammar here, so a malformed cursor never reaches the role
            // lookup or the pinned CLI.
            if let Some(cursor) = cursor.as_deref() {
                market_explorer_call(crate::stellar::validate_event_cursor(cursor))?;
            }
            let (out, manifest, net, exec) = market_read_context(cli)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            // The attachment role is validated against the live spec digest
            // before any history read is composed.
            let target = market_explorer_call(market_explorer::validated_target(
                &st, &out, &manifest, role,
            ))?;
            let events = match (cursor.as_deref(), start_ledger) {
                (Some(cursor), None) => {
                    market_explorer_call(market_history::events_after(&st, &target, cursor, count))?
                }
                (None, Some(start_ledger)) => market_explorer_call(market_history::events(
                    &st,
                    &target,
                    *start_ledger,
                    count,
                ))?,
                _ => anyhow::bail!(
                    "{}",
                    Safe::literal(
                        "market events requires exactly one of --cursor or --start-ledger"
                    )
                ),
            };
            println!("{events}");
            Ok(0)
        }
        MarketCmd::Receipt { hash } => {
            let (_out, _manifest, net, exec) = market_read_context(cli)?;
            let st = Stellar::new(&net, &exec, Policy::Market);
            // The receipt service statically validates the hash before
            // composing; a rejected input is never echoed.
            let receipt = market_explorer_call(market_history::receipt(&st, hash))?;
            println!("{receipt}");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("abtest-market-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("harness directory");
        dir
    }

    fn cli(bin: &str, out: Option<PathBuf>) -> Cli {
        Cli {
            stellar_bin: PathBuf::from(bin),
            rpc_url: "https://soroban-testnet.stellar.org".into(),
            passphrase: MARKET_TESTNET_PASSPHRASE.into(),
            out,
            cmd: Cmd::Market {
                cmd: MarketCmd::Status,
            },
        }
    }

    /// A planted run-directory `.env` is refused before any manifest read
    /// or executor exists, and the refusal never echoes what was planted.
    #[test]
    fn a_run_directory_carrying_dotenv_is_refused_before_any_read() {
        let run = harness("dotenv");
        std::fs::write(
            run.join(".env"),
            "STELLAR_CONTRACT_ID=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAD2KM\n",
        )
        .expect("planted dotenv");
        let err = market_read_context(&cli("stellar", Some(run.clone())))
            .err()
            .expect("a planted dotenv cannot steer a market read");
        assert_eq!(err.to_string(), MARKET_CWD_UNSAFE);
        assert!(
            !err.to_string().contains("STELLAR_CONTRACT_ID") && !err.to_string().contains("CAAAAA"),
            "the refusal never echoes the planted bytes: {err}"
        );
        let _ = std::fs::remove_dir_all(&run);
    }

    #[test]
    fn an_unsafe_cli_path_is_refused_without_echoing_it() {
        for requested in ["", "/", "./absent-market-stellar/stellar"] {
            let err = market_explorer_bin(Path::new(requested))
                .expect_err("a missing path or a directory cannot be the market CLI");
            assert_eq!(err.to_string(), MARKET_CLI_UNSAFE);
            assert!(
                !err.to_string().contains("absent-market-stellar"),
                "the refusal never echoes the caller's path: {err}"
            );
        }
    }
}
