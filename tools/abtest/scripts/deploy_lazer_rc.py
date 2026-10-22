#!/usr/bin/env python3
"""Replace an empty backend-price testnet RC with a signed-Lazer RC.

Deploy only the immutable oracle and pool. Reuse the pinned factory/backstop,
activation tokens and existing admin LP balance. Never change the original pool.
Every send is journaled; unresolved outcomes require manual reconciliation.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
import re
import sys
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "lazer_keeper", Path(__file__).resolve().with_name("refresh_lazer_prices.py")
)
keeper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(keeper)
ADMIN = "GCNQ4ZLK45KOYNIQLIYI7GT6TYWGK2OZEVZEY7A3W5L7QSPBNCBWSJAQ"
VERIFIER = "CAYFT5JE3UQTKT4Q6ZOZK4FXVYVT6RE3MFC7STA4UB6WAEGBT65MRU52"
ONE = 10_000_000
ROLES = ("pool", "backstop", "factory", "token", "comet", "emitter", "oracle", "verifier")


def durable_json(path: Path, value) -> None:
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w") as file:
        json.dump(value, file, sort_keys=True, indent=2)
        file.write("\n")
        file.flush()
        os.fsync(file.fileno())
    temporary.replace(path)
    fd = os.open(path.parent, os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def reserve_config(index: int) -> dict:
    return dict(index=index, decimals=7, c_factor=7_500_000 if index == 0 else 9_000_000,
                l_factor=8_000_000 if index == 0 else 9_500_000,
                util=5_000_000 if index == 0 else 8_500_000, max_util=9_500_000,
                supply_cap=str((1 << 127) - 1), r_base=100_000, r_one=500_000,
                r_two=5_000_000, r_three=15_000_000, reactivity=20, enabled=True)


def run(args) -> dict:
    if args.passphrase != keeper.PASSPHRASE or args.rpc_url != keeper.RPC_URL:
        raise ValueError("only the exact official Stellar testnet is permitted")
    if not Path(args.stellar_bin).is_absolute():
        raise ValueError("--stellar-bin must be absolute")
    base_run = Path(args.base_run).resolve(strict=True)
    out = Path(args.out).resolve()
    if out == base_run:
        raise ValueError("the replacement needs a new run directory")
    base = json.loads((base_run / "rc-deployment.json").read_text())
    if (base["passphrase"] != keeper.PASSPHRASE or base["rpc_url"] != keeper.RPC_URL
            or base["network_label"] != "testnet" or base["admin"] != ADMIN):
        raise ValueError("base deployment is not the agreed testnet RC")
    for name, token in keeper.TOKEN_CONTRACTS.items():
        if base["tokens"][name] != token:
            raise ValueError("base reserve token mismatch")
    out.mkdir(mode=0o700, parents=True, exist_ok=True)
    journal = out / "rc-journal"
    journal.mkdir(exist_ok=True)
    lock = (out / ".rc-deploy.lock").open("a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    if list(journal.glob("*.pending.json")):
        raise ValueError("reconcile unfinished deployment send before resuming")
    pins = json.loads((out / "artifact-pins.json").read_text())
    if set(pins) != set(ROLES):
        raise ValueError("artifact role set mismatch")
    for role, pin in pins.items():
        if role in keeper.ARTIFACT_FILES and Path(pin["path"]).resolve() != (
                out / "artifacts" / f"{keeper.ARTIFACT_FILES[role]}.wasm").resolve():
            raise ValueError("keeper artifact must be staged under --out/artifacts: " + role)
        data = Path(pin["path"]).read_bytes()
        if hashlib.sha256(data).hexdigest() != pin["sha256"] or len(data) > 131072:
            raise ValueError("artifact hash/size mismatch: " + role)
        if role not in ("oracle", "verifier") and pin["sha256"] != base["artifact_sha256"][role]:
            raise ValueError("reused artifact differs from base: " + role)
    cli = str(Path(args.stellar_bin).resolve(strict=True))
    admin_config = str(Path(args.admin_config_dir).resolve(strict=True))
    network = ["--rpc-url", keeper.RPC_URL, "--network-passphrase", keeper.PASSPHRASE]

    def command(arguments: list[str]) -> str:
        code, stdout, stderr = keeper._run_bounded([cli, *arguments])
        if code:
            raise RuntimeError("stellar read failed: " + stderr.decode(errors="replace")[:1200])
        return stdout.decode().strip()

    def send(label: str, arguments: list[str]) -> str:
        record = dict(argv=[cli, *arguments])
        settled = journal / (label + ".settled.json")
        if settled.exists():
            saved = json.loads(settled.read_text())
            if saved["argv"] != record["argv"]:
                raise ValueError("settled command changed: " + label)
            return saved["stdout"].strip()
        pending = journal / (label + ".pending.json")
        durable_json(pending, record)
        print(json.dumps(dict(stage=label, status="sending")), flush=True)
        code, stdout, stderr = keeper._run_bounded(record["argv"])
        record.update(exit=code, stdout=stdout.decode(errors="replace"),
                      stderr=stderr.decode(errors="replace"))
        durable_json(journal / (label + ".evidence.json"), record)
        if code:
            raise RuntimeError("send requires reconciliation: " + label)
        durable_json(settled, record)
        pending.unlink()
        fd = os.open(journal, os.O_DIRECTORY)
        os.fsync(fd)
        os.close(fd)
        print(json.dumps(dict(stage=label, status="settled")), flush=True)
        return record["stdout"].strip()

    def parameters(values: dict) -> list[str]:
        return [part for key, value in values.items() for part in
                ("--" + key, json.dumps(value, separators=(",", ":"))
                 if isinstance(value, (dict, list)) else str(value))]

    def view(contract: str, method: str, **values):
        return json.loads(command(["contract", "invoke", "--id", contract, "--source", ADMIN,
                                   *network, "--config-dir", admin_config, "--send", "no",
                                   "--", method, *parameters(values)]))

    def invoke(label: str, contract: str, method: str, **values):
        text = send(label, ["contract", "invoke", "--id", contract,
                           "--source", ADMIN, "--sign-with-key", args.admin_alias,
                           *network, "--config-dir", admin_config, "--send", "yes",
                           "--inclusion-fee", "10000", "--", method, *parameters(values)])
        return json.loads(text) if text else None

    version = command(["--version"])
    match = re.match(r"^stellar (\d+)\.(\d+)\.(\d+) ", version)
    if not match or tuple(map(int, match.groups())) < (26, 1, 0):
        raise ValueError("stable Stellar CLI >=26.1.0 required")
    if json.loads(command(["network", "info", *network, "--output", "json"]))["passphrase"] != keeper.PASSPHRASE:
        raise ValueError("RPC identity is not testnet")
    if command(["keys", "public-key", args.admin_alias, "--config-dir", admin_config]) != ADMIN:
        raise ValueError("admin alias must resolve to the agreed admin")
    signer = base["keeper_signer"]
    owner = command(["keys", "public-key", signer["alias"], "--config-dir", signer["config_dir"]])
    if owner == ADMIN or signer["config_dir"] == admin_config:
        raise ValueError("keeper must use a separate signer")
    for role in ("pool", "factory", "backstop", "comet", "emitter"):
        digest = command(["contract", "info", "hash", "--contract-id", base["contracts"][role], *network])
        if digest != pins[role]["sha256"]:
            raise ValueError("base on-chain code hash mismatch: " + role)
    if command(["contract", "info", "hash", "--contract-id", VERIFIER, *network]) != pins["verifier"]["sha256"]:
        raise ValueError("official verifier code hash mismatch")
    for token in keeper.TOKEN_CONTRACTS.values():
        if view(token, "decimals") != 7:
            raise ValueError("canonical token decimals mismatch")
        data = view(base["contracts"]["pool"], "get_reserve", asset=token)["data"]
        if int(data["b_supply"]) or int(data["d_supply"]):
            raise ValueError("old RC is not empty; preserve its keeper and resolve migration first")
    if not (journal / "backstop-deposit.settled.json").exists():
        if int(view(base["contracts"]["comet"], "balance", id=ADMIN)) < 50_000 * ONE:
            raise ValueError("insufficient existing admin activation LP balance")
    keeper.parse_signed_response(keeper._fetch_signed(Path(args.credential_file)), keeper._host_clock())
    if not args.yes:
        print(json.dumps(dict(network="testnet", admin=ADMIN, keeper=owner,
                              verifier=VERIFIER, reused_pool=base["contracts"]["pool"],
                              action="deploy replacement pool and signed oracle only")))
        return {}

    salt = hashlib.sha256((out.name + ":pyth-lazer-oracle").encode()).hexdigest()
    plan = dict(base_pool=base["contracts"]["pool"], artifacts=pins, keeper=signer,
                admin=ADMIN, verifier=VERIFIER, salt=salt, reserves=[reserve_config(0), reserve_config(1)])
    frozen = out / "rc-deployment-plan.json"
    if frozen.exists() and json.loads(frozen.read_text()) != plan:
        raise ValueError("deployment inputs differ from frozen plan")
    durable_json(frozen, plan)
    digest = send("upload-oracle", ["contract", "upload", "--wasm", pins["oracle"]["path"],
                  "--source", ADMIN, "--sign-with-key", args.admin_alias, *network,
                  "--config-dir", admin_config])
    if digest != pins["oracle"]["sha256"]:
        raise ValueError("uploaded oracle hash mismatch")
    oracle = send("deploy-oracle", ["contract", "deploy", "--wasm-hash", digest,
                  "--salt", salt, "--source", ADMIN, "--sign-with-key", args.admin_alias,
                  *network, "--config-dir", admin_config])
    predicted = command(["contract", "id", "wasm", "--source", ADMIN, "--salt", salt, *network])
    if oracle != predicted or view(oracle, "verifier") != VERIFIER:
        raise ValueError("oracle identity or verifier binding mismatch")
    contracts = {role: base["contracts"][role] for role in ("factory", "backstop", "comet", "emitter")}
    pool = invoke("factory-deploy-pool", contracts["factory"], "deploy", admin=ADMIN,
                  name="RCXlmUsdcPyth", salt=hashlib.sha256((out.name + ":pool").encode()).hexdigest(),
                  oracle=oracle, backstop_take_rate=0, max_positions=6, min_collateral=ONE)
    if view(contracts["factory"], "is_pool", pool_address=pool) is not True or view(pool, "get_admin") != ADMIN:
        raise ValueError("new pool factory/admin mismatch")
    contracts.update(pool=pool, oracle=oracle, verifier=VERIFIER)
    for index, name in enumerate(("xlm", "usdc")):
        token = keeper.TOKEN_CONTRACTS[name]
        config = reserve_config(index)
        invoke("queue-reserve-" + name, pool, "queue_set_reserve", asset=token, metadata=config)
        if invoke("set-reserve-" + name, pool, "set_reserve", asset=token) != index:
            raise ValueError("reserve index mismatch")
        if view(pool, "get_reserve", asset=token)["config"] != config:
            raise ValueError("reserve configuration mismatch")
    invoke("backstop-deposit", contracts["backstop"], "deposit",
           **{"from": ADMIN, "pool_address": pool, "amount": 50_000 * ONE})
    if int(view(contracts["backstop"], "pool_data", pool=pool)["tokens"]) != 50_000 * ONE:
        raise ValueError("new pool activation credit mismatch")
    deployment = dict(network_label="testnet", rpc_url=keeper.RPC_URL, passphrase=keeper.PASSPHRASE,
                      admin=ADMIN, contracts=contracts, tokens=base["tokens"],
                      signer=dict(alias=args.admin_alias, config_dir=admin_config), keeper_signer=signer,
                      artifact_sha256={role: pin["sha256"] for role, pin in pins.items()},
                      oracle=dict(min_sources=1, max_age_secs=300, max_clock_drift_secs=15,
                                  decimals=7, resolution=1, base="USD"),
                      oracle_trust="on-chain Pyth Lazer signatures; official testnet verifier; permissionless delivery",
                      fork_commit=base["fork_commit"], replaces_pool=base["contracts"]["pool"])
    durable_json(out / "rc-deployment.json", deployment)
    # Both RCs temporarily use the same fee account. Serialize with the
    # still-running base keeper until the replacement has proved healthy.
    base_lock = keeper._driver_lock(base_run)
    try:
        try:
            (base_run / ".backend-price-keeper-unresolved").lstat()
        except FileNotFoundError:
            pass
        else:
            raise ValueError("reconcile the base keeper's pending send before sharing its signer")
        keeper.run_cycle(out, cli, Path(args.credential_file))
    finally:
        os.close(base_lock)
    if view(pool, "get_config")["status"] == 6:
        invoke("leave-setup", pool, "set_status", pool_status=3)
    if view(pool, "get_config")["status"] == 3:
        if invoke("activate", pool, "update_status") != 1:
            raise ValueError("activation refused")
    config = view(pool, "get_config")
    if config["status"] != 1 or config["bstop_rate"] != 0 or config["oracle"] != oracle:
        raise ValueError("active pool configuration mismatch")
    if view(contracts["backstop"], "reward_zone") != []:
        raise ValueError("rewards must remain disabled")
    actors = json.loads((base_run / "qa-accounts.json").read_text())
    durable_json(out / "qa-accounts.json", actors)
    ledger = json.loads(command(["ledger", "latest", *network, "--output", "json"]))["sequence"]
    lines = [f"admin = {json.dumps(ADMIN)}", 'rpc_host = "soroban-testnet.stellar.org"',
             f"deploy_ledger = {ledger}", *[f"{role} = {json.dumps(contracts[role])}"
                for role in ("pool", "backstop", "factory", "comet", "emitter")],
             "[network]", 'label = "testnet"', f"passphrase = {json.dumps(keeper.PASSPHRASE)}",
             "[oracle]", f"runtime = {json.dumps(oracle)}", f"adapter = {json.dumps(oracle)}",
             f"sources = [{json.dumps(VERIFIER)}]", "min_sources = 1", "max_age_secs = 300",
             "max_clock_drift_secs = 15", "decimals = 7", "resolution = 1", 'base = "USD"',
             "[tokens]", *[f"{name} = {json.dumps(base['tokens'][name])}" for name in ("xlm", "usdc", "blnd")],
             f"stable = {json.dumps(base['tokens']['synthetic_usdc'])}",
             "[reserves.xlm]", "index = 0", "decimals = 7", "[reserves.usdc]", "index = 1", "decimals = 7",
             "[identities]", *[f"{who} = {json.dumps(actors['rc-' + who])}" for who in ("frodo", "sam", "user")]]
    (out / "market-manifest.toml").write_text("\n".join(lines) + "\n")
    print(json.dumps(dict(status="active", pool=pool, oracle=oracle, verifier=VERIFIER)), flush=True)
    return deployment


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-run", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--stellar-bin", required=True)
    parser.add_argument("--credential-file", required=True)
    parser.add_argument("--admin-config-dir", required=True)
    parser.add_argument("--admin-alias", default="ralplan-check")
    parser.add_argument("--rpc-url", default=keeper.RPC_URL)
    parser.add_argument("--passphrase", default=keeper.PASSPHRASE)
    parser.add_argument("--yes", action="store_true")
    run(parser.parse_args())


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
