#!/usr/bin/env python3
"""Refresh the four oracle assets of one pinned testnet market (`--market`)."""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import selectors
import signal
import subprocess
import time
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
CRATE = ROOT / "precept/tools/abtest"
EVIDENCE = ROOT / "precept-abtest-evidence"
# Pinned markets: run directory, sha256 of its market-manifest.toml, deploy ledger.
# Both share the admin, signer, assets and network below.
MARKETS: dict[str, tuple[Path, str, int]] = {
    "live": (
        EVIDENCE / "market-live-20260930",
        "b02ef2a25cf0169d612d997c7e7e6307e715836688445b2149001181243b1b3f",
        4_951_021,
    ),
    "invariants": (
        EVIDENCE / "market-invariants-20261005T104817Z",
        "7a0cf13965bf6b5d02aec2d3e1c839d0a440f3730b35e7903cf9267d9e6e5660",
        5_034_961,
    ),
}
RUN, MANIFEST_SHA256, DEPLOY_LEDGER = MARKETS["live"]
STELLAR = EVIDENCE / "recovery-20260930-market-smoke-2/tooling/stellar"
CLI = CRATE / "target/release/precept-abtest"
SIGNER_CONFIG = ROOT / ".stellar"
RPC_URL = "https://soroban-testnet.stellar.org"
PASSPHRASE = "Test SDF Network ; September 2015"
ADMIN = "GCNQ4ZLK45KOYNIQLIYI7GT6TYWGK2OZEVZEY7A3W5L7QSPBNCBWSJAQ"
SIGNER_ALIAS = "ralplan-check"
ASSETS = ("weth", "usdc", "xlm", "stable")
ADAPTER_RESOLUTION = 1  # deploy.rs builds the pool adapter at resolution 1; the oracle's 300 s bucket is source-side only
SOURCE_COUNT = 3
CHILD_TIMEOUT = 240
OUTPUT_LIMIT = 128 * 1024
MAX_I128 = (1 << 127) - 1
MAX_U64 = (1 << 64) - 1
MAX_U32 = (1 << 32) - 1
LEDGER_FETCH_OUTPUT_LIMIT = 8 * 1024 * 1024
PRICE_RE = re.compile(r"[1-9][0-9]*\Z")
LOCK_NAME = ".price-refresh.lock"
PENDING_NAME = ".spoon-price-keeper-unresolved"


class RefreshFailure(Exception):
    def __init__(self, stage: str, asset: str | None = None, source: int | None = None):
        self.stage = stage
        self.asset = asset
        self.source = source


def _log(
    stage: str,
    status: str,
    asset: str | None = None,
    source: int | None = None,
    price: str | None = None,
    timestamp: int | None = None,
) -> None:
    event: dict[str, Any] = {"stage": stage, "status": status}
    if asset is not None:
        event["asset"] = asset
    if source is not None:
        event["source"] = source
    if price is not None:
        event["price"] = price
    if timestamp is not None:
        event["timestamp"] = timestamp
    print(json.dumps(event, separators=(",", ":")), flush=True)


def _strict_json(data: bytes) -> Any:
    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key")
            result[key] = value
        return result

    def reject_constant(_: str) -> None:
        raise ValueError("non-finite JSON number")

    return json.loads(data.decode("utf-8"), object_pairs_hook=unique_object, parse_constant=reject_constant)


def _child_env() -> dict[str, str]:
    return {
        key: value for key, value in os.environ.items()
        if not key.startswith("STELLAR_") and key not in {"PYTHONPATH", "RUST_LOG", "RUST_LOG_STYLE"}
    }


def _kill_group(pid: int) -> None:
    try:
        os.killpg(pid, signal.SIGKILL)
    except OSError:
        pass


def _run_bounded(
    argv: list[str], timeout: int = CHILD_TIMEOUT, *, output_limit: int = OUTPUT_LIMIT,
) -> tuple[int, bytes, bytes]:
    """Run exactly once from checked dotenv-free root with bounded output."""
    try:
        Path("/.env").lstat()
    except FileNotFoundError:
        pass
    except OSError:
        raise RefreshFailure("child-dotenv") from None
    else:
        raise RefreshFailure("child-dotenv")
    if type(output_limit) is not int or output_limit <= 0:
        raise RefreshFailure("child-output-limit")
    if not argv or not Path(argv[0]).is_absolute():
        raise RefreshFailure("child-path")
    proc: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    completed = False
    try:
        proc = subprocess.Popen(
            argv,
            cwd="/",
            env=_child_env(),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
            close_fds=True,
        )
        streams = ((proc.stdout, "stdout"), (proc.stderr, "stderr"))
        for stream, name in streams:
            assert stream is not None
            os.set_blocking(stream.fileno(), False)
            selector.register(stream, selectors.EVENT_READ, name)
        deadline = time.monotonic() + timeout
        while selector.get_map() or proc.poll() is None:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RefreshFailure("child-timeout")
            for key, _ in selector.select(min(remaining, 0.1)):
                stream = key.fileobj
                name = key.data
                try:
                    chunk = os.read(stream.fileno(), 4096)
                except OSError:
                    raise RefreshFailure("child-io") from None
                if not chunk:
                    selector.unregister(stream)
                    stream.close()
                    continue
                target = buffers[name]
                if sum(map(len, buffers.values())) + len(chunk) > output_limit:
                    raise RefreshFailure("child-output-limit")
                target.extend(chunk)
        returncode = proc.wait()
        completed = True
        return returncode, bytes(buffers["stdout"]), bytes(buffers["stderr"])
    except OSError:
        raise RefreshFailure("child-start") from None
    finally:
        selector.close()
        if proc is not None:
            if not completed:
                _kill_group(proc.pid)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                _kill_group(proc.pid)
                proc.kill()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            for stream in (proc.stdout, proc.stderr):
                if stream is not None and not stream.closed:
                    stream.close()


def _global_argv(*command: str) -> list[str]:
    return [
        str(CLI), "--out", str(RUN), "--stellar-bin", str(STELLAR),
        "--rpc-url", RPC_URL, "--passphrase", PASSPHRASE, *command,
    ]


def _market_json(*command: str) -> Any:
    try:
        status, stdout, _ = _run_bounded(_global_argv("market", "query", *command))
        if status != 0:
            raise RefreshFailure("market-query")
        return _strict_json(stdout)
    except RefreshFailure:
        raise
    except (UnicodeError, ValueError, TypeError):
        raise RefreshFailure("market-query-json") from None


def _native_argv(
    target: str,
    method: str,
    asset: str,
    price: str | None = None,
    timestamp: int | None = None,
) -> list[str]:
    if method not in {"set_data", "refresh"} or not _contract_id(asset) or not _contract_id(target):
        raise ValueError("unsupported keeper action")
    argv = [
        str(STELLAR), "contract", "invoke", "--id", target,
        "--source", ADMIN, "--sign-with-key", SIGNER_ALIAS,
        "--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE,
        "--config-dir", str(SIGNER_CONFIG), "--send", "yes",
        "--inclusion-fee", "10000", "--", method,
        "--asset", _asset_value(asset),
    ]
    if method == "set_data":
        if (
            type(price) is not str or PRICE_RE.fullmatch(price) is None or len(price) > 39
            or int(price) > MAX_I128 or type(timestamp) is not int or not 0 < timestamp <= MAX_U64
        ):
            raise ValueError("publication requires a positive i128 price and u64 timestamp")
        argv.extend(("--price", price, "--timestamp", str(timestamp)))
    elif price is not None or timestamp is not None:
        raise ValueError("refresh takes only the asset")
    return argv


def _network_argv() -> list[str]:
    return [
        str(STELLAR), "network", "info", "--output", "json", "--rpc-url", RPC_URL,
        "--network-passphrase", PASSPHRASE, "--config-dir", str(SIGNER_CONFIG),
    ]


def _key_argv() -> list[str]:
    return [str(STELLAR), "keys", "public-key", SIGNER_ALIAS, "--config-dir", str(SIGNER_CONFIG)]


def _verify_network_and_signer() -> None:
    try:
        code, stdout, _ = _run_bounded(_network_argv())
        info = _strict_json(stdout) if code == 0 else None
        if type(info) is not dict or info.get("passphrase") != PASSPHRASE:
            raise RefreshFailure("network-preflight")
        code, stdout, _ = _run_bounded(_key_argv())
        if code != 0 or stdout.decode("utf-8").strip() != ADMIN:
            raise RefreshFailure("signer-preflight")
    except RefreshFailure:
        raise
    except (UnicodeError, ValueError, TypeError):
        raise RefreshFailure("native-preflight") from None


def _load_manifest() -> dict[str, Any]:
    try:
        data = (RUN / "market-manifest.toml").read_bytes()
        if hashlib.sha256(data).hexdigest() != MANIFEST_SHA256:
            raise RefreshFailure("manifest-pin")
        manifest = tomllib.loads(data.decode("utf-8"))
    except RefreshFailure:
        raise
    except (OSError, UnicodeError, tomllib.TOMLDecodeError):
        raise RefreshFailure("manifest-read") from None
    try:
        tokens = manifest["tokens"]
        oracle = manifest["oracle"]
        sources = oracle["sources"]
        if (
            manifest["admin"] != ADMIN
            or manifest["network"] != {"label": "testnet", "passphrase": PASSPHRASE}
            or type(manifest["deploy_ledger"]) is not int
            or manifest["deploy_ledger"] != DEPLOY_LEDGER
            or not isinstance(tokens, dict)
            or not set(ASSETS).issubset(tokens)
            or not isinstance(sources, list)
            or len(sources) != SOURCE_COUNT
            or len(set(sources)) != SOURCE_COUNT
            or not all(_contract_id(source) for source in sources)
            or not _contract_id(oracle["runtime"])
            or type(oracle["resolution"]) is not int
            or oracle["resolution"] <= 0
            or type(oracle["max_age_secs"]) is not int
            or oracle["max_age_secs"] <= 0
            or type(oracle["max_clock_drift_secs"]) is not int
            or oracle["max_clock_drift_secs"] < 0
        ):
            raise ValueError("manifest shape mismatch")
        if not all(_contract_id(tokens[asset]) for asset in ASSETS):
            raise ValueError("manifest token mismatch")
    except (KeyError, TypeError, ValueError):
        raise RefreshFailure("manifest-shape") from None
    return manifest


def _contract_id(value: Any) -> bool:
    return type(value) is str and len(value) == 56 and value.startswith("C") and value.isascii()


def _parse_quote(value: Any) -> tuple[int, int]:
    """Parse the observed native `lastprice` object without numeric coercion."""
    if type(value) is not dict or set(value) != {"price", "timestamp"}:
        raise ValueError("missing or unexpected price fields")
    price_text = value["price"]
    timestamp = value["timestamp"]
    if type(price_text) is not str or PRICE_RE.fullmatch(price_text) is None:
        raise ValueError("price must be a positive decimal i128 string")
    if type(timestamp) is not int or not 0 < timestamp <= MAX_U64:
        raise ValueError("timestamp must be a positive u64 number")
    price = int(price_text)
    if price > MAX_I128:
        raise ValueError("price exceeds i128")
    return price, timestamp


def _parse_registered_assets(value: Any, expected_ids: dict[str, str]) -> None:
    if type(value) is not list or len(value) != len(ASSETS):
        raise ValueError("registered asset list mismatch")
    expected = set(expected_ids.values())
    found: list[str] = []
    for entry in value:
        if type(entry) is not dict or set(entry) != {"Stellar"} or not _contract_id(entry["Stellar"]):
            raise ValueError("registered asset entry is malformed")
        found.append(entry["Stellar"])
    if len(set(found)) != len(ASSETS) or set(found) != expected:
        raise ValueError("registered asset list mismatch")


def _asset_value(asset: str) -> str:
    return json.dumps({"Stellar": asset}, separators=(",", ":"))


def _ledger_flags() -> list[str]:
    return [
        "--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE,
        "--config-dir", str(SIGNER_CONFIG),
    ]


def _ledger_latest_argv() -> list[str]:
    return [str(STELLAR), "ledger", "latest", "--output", "json", *_ledger_flags()]


def _ledger_fetch_argv(sequence: int) -> list[str]:
    return [
        str(STELLAR), "ledger", "fetch", str(sequence), "--limit", "1",
        "--xdr-format", "xdr", "--output", "json", *_ledger_flags(),
    ]


def _parse_latest_sequence(value: Any) -> int:
    if type(value) is not dict:
        raise ValueError("invalid latest ledger")
    sequence = value.get("sequence")
    if type(sequence) is not int or not 0 < sequence <= MAX_U32:
        raise ValueError("invalid latest sequence")
    return sequence


def _parse_latest_close_time(value: Any, requested_sequence: int) -> int:
    if type(value) is not dict:
        raise ValueError("invalid fetched ledger")
    latest = value.get("latestLedger")
    close_time = value.get("latestLedgerCloseTime")
    ledgers = value.get("ledgers")
    if (
        type(latest) is not int or not requested_sequence <= latest <= MAX_U32
        or type(close_time) is not int or not 0 < close_time <= MAX_U64
        or type(ledgers) is not list or len(ledgers) != 1
        or type(ledgers[0]) is not dict
        or type(ledgers[0].get("sequence")) is not int
        or ledgers[0]["sequence"] != requested_sequence
    ):
        raise ValueError("invalid ledger close time")
    return close_time


def _ledger_seconds() -> int:
    try:
        status, stdout, _ = _run_bounded(_ledger_latest_argv())
        if status != 0:
            raise ValueError("ledger latest failed")
        sequence = _parse_latest_sequence(_strict_json(stdout))
        status, stdout, _ = _run_bounded(
            _ledger_fetch_argv(sequence), output_limit=LEDGER_FETCH_OUTPUT_LIMIT,
        )
        if status != 0:
            raise ValueError("ledger fetch failed")
        return _parse_latest_close_time(_strict_json(stdout), sequence)
    except RefreshFailure:
        raise RefreshFailure("ledger-clock") from None
    except (UnicodeError, ValueError, TypeError):
        raise RefreshFailure("ledger-clock") from None


def _read_source(source: int, asset: str, contract: str) -> tuple[int, int]:
    try:
        value = _market_json(f"source.{source}", "lastprice", "--arg", f"asset={_asset_value(contract)}")
        return _parse_quote(value)
    except RefreshFailure:
        raise
    except (ValueError, TypeError):
        raise RefreshFailure("source-quote", asset, source) from None


def _read_adapter(asset: str, contract: str) -> tuple[int, int]:
    try:
        return _parse_quote(_market_json("adapter", "lastprice", "--arg", f"asset={_asset_value(contract)}"))
    except RefreshFailure:
        raise
    except (ValueError, TypeError):
        raise RefreshFailure("adapter-quote", asset) from None


def _read_runtime_timestamp(asset: str, contract: str) -> int:
    try:
        value = _market_json(
            "proxy-runtime", "aggregated_latest", "--arg", f"asset={_asset_value(contract)}",
        )
        if type(value) is not dict or set(value) != {"mantissa", "expo", "timestamp"}:
            raise ValueError("invalid normalized runtime price")
        mantissa = value["mantissa"]
        exponent = value["expo"]
        timestamp = value["timestamp"]
        if (
            type(mantissa) is not int or not -(1 << 63) <= mantissa < (1 << 63)
            or type(exponent) is not int or not -(1 << 31) <= exponent < (1 << 31)
            or type(timestamp) is not int or not 0 < timestamp <= MAX_U64
        ):
            raise ValueError("invalid normalized runtime price")
        return timestamp
    except RefreshFailure:
        raise
    except (TypeError, ValueError):
        raise RefreshFailure("runtime-quote", asset) from None


def _pending_marker(asset: str, source: int | None, price: int, timestamp: int | None) -> bytes:
    record = {"schema": "spoon-price-keeper/pending/v1", "asset": asset, "price": str(price)}
    if source is not None:
        record["source"] = source
        record["timestamp"] = timestamp
        record["action"] = "set_data"
    else:
        record["action"] = "refresh"
    return (json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n").encode("ascii")


def _ensure_no_pending(run_dir: Path) -> None:
    try:
        (run_dir / PENDING_NAME).lstat()
    except FileNotFoundError:
        return
    except OSError:
        raise RefreshFailure("unresolved-marker") from None
    raise RefreshFailure("unresolved-marker")


def _write_pending(payload: bytes, run_dir: Path) -> None:
    path = run_dir / PENDING_NAME
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            view = memoryview(payload)
            while view:
                written = os.write(fd, view)
                if written <= 0:
                    raise OSError("short marker write")
                view = view[written:]
            os.fsync(fd)
        finally:
            os.close(fd)
        dirfd = os.open(run_dir, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
    except OSError:
        raise RefreshFailure("marker-write") from None


def _clear_pending(payload: bytes, run_dir: Path) -> None:
    name = PENDING_NAME
    try:
        dirfd = os.open(run_dir, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            before = os.stat(name, dir_fd=dirfd, follow_symlinks=False)
            if before.st_mode & 0o170000 != 0o100000:
                raise OSError("marker is not a regular file")
            fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dirfd)
            try:
                opened = os.fstat(fd)
                data = os.read(fd, len(payload) + 1)
                if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino) or data != payload:
                    raise OSError("marker changed")
            finally:
                os.close(fd)
            os.unlink(name, dir_fd=dirfd)
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
    except OSError:
        raise RefreshFailure("marker-clear") from None


def _driver_lock() -> int:
    fd: int | None = None
    try:
        info = RUN.lstat()
        if RUN.is_symlink() or not RUN.is_dir() or info.st_mode & 0o170000 != 0o040000:
            raise OSError("unsafe run directory")
        fd = os.open(RUN / LOCK_NAME, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fd
    except OSError:
        if fd is not None:
            try:
                os.close(fd)
            except OSError:
                pass
        raise RefreshFailure("driver-lock") from None


def _verify_adapter(
    asset: str, contract: str, price: int, publish_times: list[int],
    max_age: int, max_drift: int, *, resolution: int,
) -> None:
    if type(resolution) is not int or resolution <= 0:
        raise RefreshFailure("adapter-readback", asset)
    observed_price, timestamp = _read_adapter(asset, contract)
    runtime_timestamp = _read_runtime_timestamp(asset, contract)
    now = _ledger_seconds()
    # MedianLow preserves its selected source timestamp; see the pinned
    # contracts/contract/proxy-oracle/kernel/src/proxy/aggregator/method/median/specific_price.rs.
    if (
        observed_price != price
        or runtime_timestamp not in publish_times
        or timestamp != runtime_timestamp - runtime_timestamp % resolution
        or runtime_timestamp > now + max_drift
        or now - runtime_timestamp > max_age
    ):
        raise RefreshFailure("adapter-readback", asset)


def _send_once(argv: list[str]) -> None:
    code, _, _ = _run_bounded(argv)
    if code != 0:
        raise RefreshFailure("native-send")


def refresh_cycle() -> None:
    lock_fd = _driver_lock()
    try:
        _ensure_no_pending(RUN)
        _log("cycle", "start")
        manifest = _load_manifest()
        token_ids = {asset: manifest["tokens"][asset] for asset in ASSETS}
        try:
            _verify_network_and_signer()
            registry = _market_json("proxy-runtime", "registered_assets")
            _parse_registered_assets(registry, token_ids)
        except RefreshFailure:
            raise RefreshFailure("registration-preflight") from None
        except (ValueError, TypeError):
            raise RefreshFailure("registration-preflight") from None

        quotes: dict[str, dict[int, tuple[int, int]]] = {asset: {} for asset in ASSETS}
        for source in range(SOURCE_COUNT):
            for asset in ASSETS:
                _log("source-read", "start", asset, source)
                try:
                    quotes[asset][source] = _read_source(source, asset, token_ids[asset])
                except RefreshFailure:
                    raise RefreshFailure("source-read", asset, source) from None
                _log("source-read", "done", asset, source)

        quote_check_time = _ledger_seconds()
        for asset in ASSETS:
            for source, (_, old_timestamp) in quotes[asset].items():
                if old_timestamp >= quote_check_time:
                    raise RefreshFailure("source-timestamp", asset, source)
        # MedianLow over the pinned SOURCE_COUNT quotes, matching the runtime aggregator.
        median = {asset: sorted(quotes[asset][s][0] for s in range(SOURCE_COUNT))[(SOURCE_COUNT - 1) // 2] for asset in ASSETS}
        publish_times: dict[str, list[int]] = {asset: [] for asset in ASSETS}

        for source in range(SOURCE_COUNT):
            for asset in ASSETS:
                price, snapshot_timestamp = quotes[asset][source]
                _log("publish", "start", asset, source, str(price))
                try:
                    _verify_network_and_signer()
                    current = _read_source(source, asset, token_ids[asset])
                    if current != (price, snapshot_timestamp):
                        raise RefreshFailure("source-changed", asset, source)
                    timestamp = _ledger_seconds()
                    if timestamp <= current[1]:
                        raise RefreshFailure("publish-timestamp", asset, source)
                    argv = _native_argv(
                        manifest["oracle"]["sources"][source], "set_data", token_ids[asset],
                        str(price), timestamp,
                    )
                    marker = _pending_marker(asset, source, price, timestamp)
                    _write_pending(marker, RUN)
                    _send_once(argv)
                    published = _read_source(source, asset, token_ids[asset])
                    if published != (price, timestamp):
                        raise RefreshFailure("source-readback", asset, source)
                    _clear_pending(marker, RUN)
                    publish_times[asset].append(timestamp)
                except RefreshFailure as failure:
                    raise RefreshFailure(f"publish-{failure.stage}", asset, source) from None
                _log("publish", "done", asset, source, str(price), timestamp)

        for asset in ASSETS:
            price = median[asset]
            _log("refresh", "start", asset, price=str(price))
            try:
                _verify_network_and_signer()
                current_sources = []
                for source in range(SOURCE_COUNT):
                    _log("refresh-source-check", "start", asset, source)
                    current_sources.append(_read_source(source, asset, token_ids[asset]))
                    _log("refresh-source-check", "done", asset, source)
                for source, current in enumerate(current_sources):
                    expected = (quotes[asset][source][0], publish_times[asset][source])
                    if current != expected:
                        raise RefreshFailure("source-changed", asset, source)
                argv = _native_argv(manifest["oracle"]["runtime"], "refresh", token_ids[asset])
                marker = _pending_marker(asset, None, price, None)
                _write_pending(marker, RUN)
                _send_once(argv)
                _verify_adapter(
                    asset, token_ids[asset], price, publish_times[asset],
                    manifest["oracle"]["max_age_secs"], manifest["oracle"]["max_clock_drift_secs"],
                    resolution=ADAPTER_RESOLUTION,
                )
                _clear_pending(marker, RUN)
            except RefreshFailure as failure:
                raise RefreshFailure(f"refresh-{failure.stage}", asset, failure.source) from None
            _log("refresh", "done", asset, price=str(price))

        for asset in ASSETS:
            _log("adapter-readback", "start", asset, price=str(median[asset]))
            try:
                _verify_adapter(
                    asset, token_ids[asset], median[asset], publish_times[asset],
                    manifest["oracle"]["max_age_secs"], manifest["oracle"]["max_clock_drift_secs"],
                    resolution=ADAPTER_RESOLUTION,
                )
            except RefreshFailure:
                raise RefreshFailure("adapter-readback", asset) from None
            _log("adapter-readback", "done", asset, price=str(median[asset]))
        _log("cycle", "done")
    finally:
        os.close(lock_fd)


def select_market(name: str) -> None:
    """Rebind the per-market pins; every later read goes through the module globals."""
    global RUN, MANIFEST_SHA256, DEPLOY_LEDGER
    RUN, MANIFEST_SHA256, DEPLOY_LEDGER = MARKETS[name]
    _log("market", name)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--market", choices=sorted(MARKETS), default="live")
    select_market(parser.parse_args().market)
    try:
        refresh_cycle()
        return 0
    except RefreshFailure as failure:
        _log(failure.stage, "failed", failure.asset, failure.source)
        return 1
    except KeyboardInterrupt:
        _log("interrupted", "failed")
        return 1
    except Exception:
        _log("internal", "failed")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
