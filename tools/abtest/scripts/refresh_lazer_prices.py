#!/usr/bin/env python3
"""Refresh one disposable TESTNET receiver with official signed Pyth Lazer updates.

Parsed API fields are expectations only: the immutable receiver authenticates the
signed payload through the official verifier. An unresolved send requires manual
reconciliation; this keeper never replays it automatically.
"""
from __future__ import annotations

import argparse
import base64
import binascii
import fcntl
import hashlib
import http.client
import json
import os
import re
import selectors
import signal
import stat
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

RPC_URL = "https://soroban-testnet.stellar.org"
PASSPHRASE = "Test SDF Network ; September 2015"
LAZER_URL = "https://pyth-lazer-0.dourolabs.app/v1/latest_price"
VERIFIER = "CAYFT5JE3UQTKT4Q6ZOZK4FXVYVT6RE3MFC7STA4UB6WAEGBT65MRU52"
TOKEN_CONTRACTS = {
    "xlm": "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
    "usdc": "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA",
}
FEED_IDS = {"xlm": 23, "usdc": 7}
ASSET_ORDER = ("xlm", "usdc")
ASSET_CONTRACTS = REQUIRED_ARTIFACTS = ("pool", "oracle", "verifier")
ARTIFACT_FILES = {"pool": "pool", "oracle": "pyth-lazer-oracle", "verifier": "pyth-verifier"}
MAX_I128 = (1 << 127) - 1
MAX_U64 = (1 << 64) - 1
MAX_QUOTE_AGE = 60
MAX_FUTURE_AGE = 15
HTTP_TIMEOUT = 10
CHILD_TIMEOUT = 240
OUTPUT_LIMIT = 128 * 1024
HTTP_LIMIT = 65536
PRICE_RE = re.compile(r"[1-9][0-9]*\Z", re.ASCII)
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
CLI_VERSION_RE = re.compile(r"\Astellar (\d+)\.(\d+)\.(\d+) ", re.ASCII)
LOCK_NAME = ".price-refresh.lock"
PENDING_NAME = ".lazer-price-keeper-unresolved"
RESULT_NAME = PENDING_NAME + ".result"


class KeeperFailure(Exception):
    def __init__(self, stage: str, asset: str | None = None):
        super().__init__(stage)
        self.stage = stage
        self.asset = asset


def _log(stage: str, status: str, asset: str | None = None,
         price: str | None = None, timestamp: int | None = None) -> None:
    event: dict[str, Any] = {"stage": stage, "status": status}
    if asset is not None:
        event["asset"] = asset
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

    return json.loads(data.decode("utf-8"), object_pairs_hook=unique_object,
                      parse_constant=reject_constant, parse_float=reject_constant)


def _child_env() -> dict[str, str]:
    # Do not inherit API keys, proxy credentials, Stellar overrides, or dotenv hooks.
    allowed = {"PATH", "HOME", "LANG", "LC_ALL", "SSL_CERT_FILE", "SSL_CERT_DIR",
               "NIX_SSL_CERT_FILE", "TMPDIR"}
    return {key: value for key, value in os.environ.items() if key in allowed}


def _kill_group(pid: int) -> None:
    try:
        os.killpg(pid, signal.SIGKILL)
    except OSError:
        pass


def _run_bounded(argv: list[str], timeout: int = CHILD_TIMEOUT, *,
                 output_limit: int = OUTPUT_LIMIT,
                 receipt: Path | None = None) -> tuple[int, bytes, bytes]:
    """Run exactly once from a dotenv-free root with bounded output."""
    try:
        Path("/.env").lstat()
    except FileNotFoundError:
        pass
    except OSError:
        raise KeeperFailure("child-dotenv") from None
    else:
        raise KeeperFailure("child-dotenv")
    if type(output_limit) is not int or output_limit <= 0:
        raise KeeperFailure("child-output-limit")
    if not argv or not Path(argv[0]).is_absolute():
        raise KeeperFailure("child-path")
    proc: subprocess.Popen[bytes] | None = None
    selector = selectors.DefaultSelector()
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    completed = False
    failure = None
    try:
        proc = subprocess.Popen(
            argv, cwd="/", env=_child_env(), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True, close_fds=True,
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
                raise KeeperFailure("child-timeout")
            for key, _ in selector.select(min(remaining, 0.1)):
                stream = key.fileobj
                name = key.data
                try:
                    chunk = os.read(stream.fileno(), 4096)
                except OSError:
                    raise KeeperFailure("child-io") from None
                if not chunk:
                    selector.unregister(stream)
                    stream.close()
                    continue
                target = buffers[name]
                available = output_limit - sum(map(len, buffers.values()))
                if len(chunk) > available:
                    target.extend(chunk[:available])
                    raise KeeperFailure("child-output-limit")
                target.extend(chunk)
        returncode = proc.wait()
        completed = True
        return returncode, bytes(buffers["stdout"]), bytes(buffers["stderr"])
    except KeeperFailure as exc:
        failure = exc.stage
        raise
    except OSError:
        failure = "child-start"
        raise KeeperFailure(failure) from None
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
            for stream, name in ((proc.stdout, "stdout"), (proc.stderr, "stderr")):
                if stream is not None and not stream.closed:
                    # SIGTERM/interrupt can fire while readable output sits
                    # unread in the pipe: drain only what is already available,
                    # into the same buffers, capped by the same combined limit.
                    target = buffers[name]
                    try:
                        while True:
                            room = output_limit - sum(map(len, buffers.values()))
                            if room <= 0:
                                break
                            chunk = os.read(stream.fileno(), min(4096, room))
                            if not chunk:
                                break
                            target.extend(chunk)
                    except OSError:  # BlockingIOError: none buffered; dead pipe: closed
                        pass
                    stream.close()
        if receipt is not None:
            _write_pending(_result_record(
                proc.returncode if proc is not None else None,
                bytes(buffers["stdout"]), bytes(buffers["stderr"]), failure,
            ), receipt, filename=RESULT_NAME)


def load_deployment(runpath: Path) -> dict[str, Any]:
    """Parse rc-deployment.json once, testnet-only, before any side effect."""
    try:
        path = runpath / "rc-deployment.json"
        data = path.read_bytes()
        deployment = _strict_json(data)
    except (OSError, UnicodeError, ValueError):
        raise KeeperFailure("deployment-read") from None
    try:
        if type(deployment) is not dict:
            raise ValueError("deployment must be an object")
        if (deployment.get("network_label") != "testnet"
                or deployment.get("passphrase") != PASSPHRASE
                or deployment.get("rpc_url") != RPC_URL):
            raise ValueError("deployment network mismatch")
        contracts = deployment["contracts"]
        if contracts.get("verifier") != VERIFIER:
            raise ValueError("official testnet verifier required")
        tokens = deployment["tokens"]
        signer = deployment["signer"]
        keeper = deployment["keeper_signer"]
        artifacts = deployment["artifact_sha256"]
        admin = deployment["admin"]
        if type(admin) is not str or len(admin) != 56 or not admin.startswith("G"):
            raise ValueError("admin must be a G-address")
        for name, table in (("contracts", contracts), ("tokens", tokens),
                            ("artifact_sha256", artifacts)):
            if type(table) is not dict:
                raise ValueError(f"{name} must be an object")
        for name in ASSET_CONTRACTS:
            if not _contract_id(contracts[name]):
                raise ValueError("contract id malformed")
        for asset, contract in TOKEN_CONTRACTS.items():
            if tokens[asset] != contract:
                raise ValueError("token must be canonical testnet XLM or USDC")
        for name in REQUIRED_ARTIFACTS:
            value = artifacts[name]
            if type(value) is not str or SHA256_RE.fullmatch(value) is None:
                raise ValueError("artifact pin malformed")
            local = runpath / "artifacts" / f"{ARTIFACT_FILES[name]}.wasm"
            try:
                if hashlib.sha256(local.read_bytes()).hexdigest() != value:
                    raise ValueError("local artifact does not match its pin")
            except OSError:
                raise ValueError("local artifact missing") from None
        if (type(signer) is not dict or type(signer.get("alias")) is not str
                or not signer["alias"] or signer["alias"].startswith("-")
                or type(signer.get("config_dir")) is not str
                or not signer["config_dir"].startswith("/")):
            raise ValueError("admin signer misconfigured")
        if (type(keeper) is not dict or type(keeper.get("alias")) is not str
                or not keeper["alias"] or keeper["alias"].startswith("-")
                or type(keeper.get("config_dir")) is not str
                or not keeper["config_dir"].startswith("/")
                or keeper["alias"] == signer["alias"]
                or keeper["config_dir"] == signer["config_dir"]):
            raise ValueError("keeper signer must be an explicit separate key")
    except (KeyError, TypeError, ValueError):
        raise KeeperFailure("deployment-shape") from None
    return deployment


def _contract_id(value: Any) -> bool:
    return type(value) is str and len(value) == 56 and value.startswith("C") and value.isascii()


def resolve_runtime(deployment: dict[str, Any], stellar_bin: str) -> None:
    """Resolve and pin the CLI binary and keeper public key into the deployment."""
    try:
        code, stdout, _ = _run_bounded([stellar_bin, "--version"], timeout=60)
        text = stdout.decode("ascii", "strict").splitlines()[0]
    except (IndexError, UnicodeError):
        raise KeeperFailure("cli-version") from None
    match = CLI_VERSION_RE.match(text) if code == 0 else None
    if code != 0 or match is None:
        raise KeeperFailure("cli-version")
    if (int(match.group(1)), int(match.group(2))) < (26, 1):
        raise KeeperFailure("cli-version")
    deployment["_stellar_bin"] = stellar_bin
    keeper = deployment["keeper_signer"]
    argv = [
        stellar_bin, "keys", "public-key", keeper["alias"],
        "--config-dir", keeper["config_dir"],
    ]
    try:
        code, stdout, _ = _run_bounded(argv)
        if code != 0:
            raise KeeperFailure("keeper-key")
        key = stdout.decode("ascii", "strict").strip()
    except (UnicodeError, ValueError):
        raise KeeperFailure("keeper-key") from None
    if len(key) != 56 or not key.startswith("G"):
        raise KeeperFailure("keeper-key")
    deployment["keeper_public_key"] = key


def _verify_network_identity(deployment: dict[str, Any]) -> None:
    """Pin the RPC endpoint to the exact testnet passphrase (native SAC is not enough)."""
    argv = [str(deployment["_stellar_bin"]), "network", "info",
            "--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE,
            "--config-dir", deployment["keeper_signer"]["config_dir"],
            "--output", "json"]
    try:
        code, stdout, _ = _run_bounded(argv, timeout=60)
        if code != 0:
            raise KeeperFailure("rpc-passphrase")
        value = _strict_json(stdout)
    except (UnicodeError, ValueError, TypeError):
        raise KeeperFailure("rpc-passphrase") from None
    if type(value) is not dict or value.get("passphrase") != PASSPHRASE:
        raise KeeperFailure("rpc-passphrase")


def _verify_chain_code_hashes(deployment: dict[str, Any]) -> None:
    """Bind pool/oracle/verifier on-chain code hashes to the artifact pins."""
    for role in REQUIRED_ARTIFACTS:
        argv = [str(deployment["_stellar_bin"]), "contract", "info", "hash",
                "--contract-id", deployment["contracts"][role],
                "--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE,
                "--config-dir", deployment["keeper_signer"]["config_dir"]]
        try:
            code, stdout, _ = _run_bounded(argv, timeout=60)
            if code != 0:
                raise KeeperFailure("chain-hash", role)
            value = stdout.decode("ascii", "strict").strip()
        except (UnicodeError, ValueError):
            raise KeeperFailure("chain-hash", role) from None
        if SHA256_RE.fullmatch(value) is None or value != deployment["artifact_sha256"][role]:
            raise KeeperFailure("chain-hash", role)


def _host_clock() -> int:
    return time.time_ns() // 1000


def _read_credential(credential_file: Path) -> str:
    # File credential only (systemd LoadCredential); the secret never appears
    # in argv, the child env, or any log line.
    try:
        info = os.lstat(credential_file)
        if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
            raise KeeperFailure("credential-file")
        raw = credential_file.read_bytes()
    except OSError:
        raise KeeperFailure("credential-file") from None
    if len(raw) > 512:
        raise KeeperFailure("credential-file")
    try:
        key = raw.decode("ascii", "strict").strip()
    except UnicodeError:
        raise KeeperFailure("credential-file") from None
    if not key or any(c.isspace() for c in key):
        raise KeeperFailure("credential-file")
    return key


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _fetch_signed(credential_file: Path) -> bytes:
    """POST the fixed request body; return bounded raw response bytes."""
    key = _read_credential(credential_file)
    body = json.dumps({
        "priceFeedIds": [FEED_IDS["xlm"], FEED_IDS["usdc"]],
        "properties": ["price", "exponent", "feedUpdateTimestamp"],
        "formats": ["leEcdsa"],
        "channel": "fixed_rate@200ms",
        "jsonBinaryEncoding": "base64",
        "parsed": True,
    }, separators=(",", ":")).encode("ascii")
    request = urllib.request.Request(
        LAZER_URL, data=body,
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        method="POST",
    )
    opener = urllib.request.build_opener(_NoRedirect)
    try:
        with opener.open(request, timeout=HTTP_TIMEOUT) as response:
            if getattr(response, "status", 200) != 200:
                raise KeeperFailure("lazer-fetch")
            payload = response.read(HTTP_LIMIT + 1)
    except urllib.error.HTTPError as error:
        # Close the error response before its finalizer can log a server-
        # supplied reason (which may echo a credential).
        error.close()
        raise KeeperFailure("lazer-fetch") from None
    except (urllib.error.URLError, OSError, ValueError, http.client.HTTPException):
        # Stage only: never the response body, URL, or credential.
        raise KeeperFailure("lazer-fetch") from None
    if len(payload) > HTTP_LIMIT:
        raise KeeperFailure("lazer-fetch")
    return payload


def parse_signed_response(response: bytes, now_us: int) -> tuple[bytes, int, dict[str, tuple[int, int]]]:
    """Strictly validate the API response; parsed fields are expectations only.

    Returns (raw signed payload bytes, header timestamp us,
    {asset: (usd7 floor, feed_update_timestamp us)}).
    """
    if type(now_us) is not int or not 0 < now_us <= MAX_U64:
        raise KeeperFailure("quote-clock")
    try:
        body = _strict_json(response)
        if type(body) is not dict or set(body) != {"parsed", "leEcdsa"}:
            raise ValueError("lazer envelope malformed")
        parsed, signed = body["parsed"], body["leEcdsa"]
        if type(parsed) is not dict or set(parsed) != {"timestampUs", "priceFeeds"}:
            raise ValueError("lazer parsed malformed")
        if type(signed) is not dict or set(signed) != {"encoding", "data"}:
            raise ValueError("lazer signed envelope malformed")
        if signed["encoding"] != "base64" or type(signed["data"]) is not str:
            raise ValueError("lazer signed encoding mismatch")
        header_us = _strict_u64(parsed["timestampUs"], str_int=True)
        payload = base64.b64decode(signed["data"], validate=True)
        if len(payload) < 64:
            raise ValueError("raw update too small")
        rows = parsed["priceFeeds"]
        if type(rows) is not list or len(rows) != len(ASSET_ORDER):
            raise ValueError("lazer feed rows malformed")
        quotes: dict[str, tuple[int, int]] = {}
        for row in rows:
            if type(row) is not dict or set(row) != {
                "priceFeedId", "price", "exponent", "feedUpdateTimestamp",
            }:
                raise ValueError("lazer feed row malformed")
            asset = next((a for a, fid in FEED_IDS.items() if fid == row["priceFeedId"]), None)
            if asset is None or asset in quotes:
                raise ValueError("lazer feed set mismatch")
            if row["exponent"] != -8:
                raise ValueError("lazer feed exponent mismatch")
            feed_us = _strict_u64(row["feedUpdateTimestamp"])
            if feed_us > header_us:
                raise ValueError("lazer feed newer than header")
            _check_age(feed_us, now_us)
            value = _floor_usd7(row["price"])
            quotes[asset] = (value, feed_us)
        if set(quotes) != set(ASSET_ORDER):
            raise ValueError("lazer missing a feed")
        _check_age(header_us, now_us)
        return payload, header_us, quotes
    except (ValueError, TypeError, binascii.Error):
        raise KeeperFailure("lazer-parse") from None


def _strict_u64(value: Any, *, str_int: bool = False) -> int:
    if str_int:
        if type(value) is not str or PRICE_RE.fullmatch(value) is None:
            raise ValueError("u64 string malformed")
        value = int(value)
    if type(value) is not int or not 0 < value <= MAX_U64:
        raise ValueError("u64 must be a positive integer")
    return value


def _check_age(value_us: int, now_us: int) -> None:
    age = now_us - value_us
    if age > MAX_QUOTE_AGE * 1_000_000 or -age > MAX_FUTURE_AGE * 1_000_000:
        raise ValueError("update stale or future-dated")


def _floor_usd7(price: Any) -> int:
    # Signed expo is fixed -8; positive raw i64. price * 10**-8 USD floored
    # into USD-7 units is integer floor division by 10. No floats.
    if type(price) is not str or PRICE_RE.fullmatch(price) is None:
        raise ValueError("lazer price must be a positive integer string")
    raw = int(price)
    usd7 = raw // 10
    if usd7 <= 0 or usd7 > MAX_I128:
        raise ValueError("price out of i128 range")
    return usd7


def _seconds(us: int) -> int:
    return us // 1_000_000


def _verify_verifier_binding(deployment: dict[str, Any]) -> None:
    """The receiver's own verifier() must name the fixed official verifier."""
    argv = _invoke_argv(deployment, deployment["contracts"]["oracle"], "verifier", send=False)
    try:
        code, stdout, _ = _run_bounded(argv)
        if code != 0:
            raise KeeperFailure("verifier-binding")
        value = _strict_json(stdout)
    except (UnicodeError, ValueError, TypeError):
        raise KeeperFailure("verifier-binding") from None
    if value != VERIFIER:
        raise KeeperFailure("verifier-binding")


def _invoke_argv(deployment: dict[str, Any], target: str, method: str, *,
                 asset: str | None = None, payload: bytes | None = None,
                 send: bool) -> list[str]:
    argv = [str(deployment["_stellar_bin"]), "contract", "invoke", "--id", target]
    if send:
        argv += ["--sign-with-key", deployment["keeper_signer"]["alias"]]
    argv += [
        "--source", deployment["keeper_public_key"],
        "--rpc-url", RPC_URL, "--network-passphrase", PASSPHRASE,
        "--config-dir", deployment["keeper_signer"]["config_dir"],
    ]
    if send:
        argv += ["--send", "yes", "--inclusion-fee", "10000"]
    else:
        argv += ["--send", "no"]
    argv += ["--", method]
    if asset is not None:
        argv += ["--asset", json.dumps({"Stellar": asset}, separators=(",", ":"))]
    if payload is not None:
        # Signed payload is public; it is never a secret.
        argv += ["--payload", payload.hex()]
    return argv


def _read_lastprice(deployment: dict[str, Any], asset: str) -> tuple[int, int] | None:
    argv = _invoke_argv(
        deployment, deployment["contracts"]["oracle"], "lastprice",
        asset=asset, send=False,
    )
    try:
        code, stdout, _ = _run_bounded(argv)
        if code != 0:
            raise ValueError("oracle read failed")
        value = _strict_json(stdout) if stdout.strip() else None
    except (UnicodeError, ValueError, TypeError):
        raise ValueError("oracle read malformed") from None
    if value is None:
        return None
    if type(value) is not dict or set(value) != {"price", "timestamp"}:
        raise ValueError("oracle quote shape malformed")
    price_text, timestamp = value["price"], value["timestamp"]
    if (type(price_text) is not str or PRICE_RE.fullmatch(price_text) is None
            or type(timestamp) is not int or not 0 <= timestamp <= MAX_U64):
        raise ValueError("oracle quote malformed")
    price = int(price_text)
    if price > MAX_I128:
        raise ValueError("oracle quote malformed")
    return price, timestamp


def _result_record(code: int | None, stdout: bytes, stderr: bytes,
                   failure: str | None = None) -> bytes:
    return (json.dumps({
        "schema": "lazer-price-keeper/result/v1",
        "exit": code, "failure": failure,
        "stdout_base64": base64.b64encode(stdout).decode("ascii"),
        "stderr_base64": base64.b64encode(stderr).decode("ascii"),
    }, sort_keys=True, separators=(",", ":")) + "\n").encode("ascii")


def _pending_marker(payload: bytes, header_us: int, argv: list[str]) -> bytes:
    record = {
        "schema": "lazer-price-keeper/pending/v1",
        "payload_sha256": hashlib.sha256(payload).hexdigest(),
        "header_timestamp_us": header_us,
        "argv": argv,
    }
    return (json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n").encode("ascii")


def _ensure_no_pending(run_dir: Path) -> None:
    """Any pending object — regular or not — blocks the cycle; content is irrelevant."""
    for filename in (PENDING_NAME, RESULT_NAME):
        try:
            os.lstat(run_dir / filename)
        except FileNotFoundError:
            continue
        except OSError:
            pass
        raise KeeperFailure("unresolved-marker")


def _write_pending(payload: bytes, run_dir: Path, *, filename: str = PENDING_NAME) -> None:
    path = run_dir / filename
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
        raise KeeperFailure("marker-write") from None


def _clear_pending(payload: bytes, run_dir: Path, *, filename: str = PENDING_NAME) -> None:
    try:
        dirfd = os.open(run_dir, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            before = os.stat(filename, dir_fd=dirfd, follow_symlinks=False)
            if not stat.S_ISREG(before.st_mode):
                raise OSError("marker is not a regular file")
            fd = os.open(filename, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=dirfd)
            try:
                opened = os.fstat(fd)
                data = os.read(fd, len(payload) + 1)
                if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino) or data != payload:
                    raise OSError("marker changed")
            finally:
                os.close(fd)
            os.unlink(filename, dir_fd=dirfd)
            os.fsync(dirfd)
        finally:
            os.close(dirfd)
    except OSError:
        raise KeeperFailure("marker-clear") from None


def _driver_lock(runpath: Path) -> int:
    fd: int | None = None
    try:
        info = runpath.lstat()
        if runpath.is_symlink() or not runpath.is_dir() or not stat.S_ISDIR(info.st_mode):
            raise OSError("unsafe run directory")
        fd = os.open(runpath / LOCK_NAME, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fd
    except OSError:
        if fd is not None:
            try:
                os.close(fd)
            except OSError:
                pass
        raise KeeperFailure("driver-lock") from None


def check_quotes(runpath: Path, stellar_bin: str, credential_file: Path) -> dict[str, dict[str, Any]]:
    """Fetch, parse, and report quotes; never locks, journals, or mutates."""
    deployment = load_deployment(runpath)
    resolve_runtime(deployment, stellar_bin)
    _verify_network_identity(deployment)
    _verify_chain_code_hashes(deployment)
    _verify_verifier_binding(deployment)
    payload, header_us, quotes = parse_signed_response(_fetch_signed(credential_file), _host_clock())
    report = {
        asset: {"price": str(quotes[asset][0]), "timestamp": _seconds(quotes[asset][1])}
        for asset in ASSET_ORDER
    }
    _log("check", "done", price=json.dumps(report, separators=(",", ":")), timestamp=header_us)
    return report


def _terminated(signum, frame):
    # systemd TimeoutStartSec SIGTERMs us before the per-child deadline and
    # the default disposition would skip the receipt. Raising a fixed stage
    # (no signal or exception content is ever logged) lets _run_bounded's
    # finally kill/reap the child and write the receipt first.
    signal.signal(signal.SIGTERM, signal.SIG_DFL)
    raise KeeperFailure("terminated")


def run_cycle(runpath: Path, stellar_bin: str, credential_file: Path) -> dict[str, str]:
    runpath = Path(runpath)
    deployment = load_deployment(runpath)
    lock_fd = _driver_lock(runpath)
    try:
        _ensure_no_pending(runpath)
        resolve_runtime(deployment, stellar_bin)
        _verify_network_identity(deployment)
        _verify_chain_code_hashes(deployment)
        _verify_verifier_binding(deployment)
        payload, header_us, quotes = parse_signed_response(_fetch_signed(credential_file), _host_clock())
        result: dict[str, str] = {asset: "unchanged" for asset in ASSET_ORDER}
        stale_or_conflict: list[str] = []
        for asset in ASSET_ORDER:
            usd7, feed_us = quotes[asset]
            try:
                current = _read_lastprice(deployment, deployment["tokens"][asset])
            except ValueError:
                raise KeeperFailure("oracle-read", asset) from None
            if current is None or current != (usd7, _seconds(feed_us)):
                result[asset] = "update"
            if current is not None and current[1] > _seconds(feed_us):
                stale_or_conflict.append(asset)
        if stale_or_conflict:
            # On-chain monotonicity rejects the whole update too; fail closed
            # before spending anything.
            raise KeeperFailure("quote-not-newer", stale_or_conflict[0])
        if all(value == "unchanged" for value in result.values()):
            _log("cycle", "done", price=json.dumps(result, separators=(",", ":")))
            return result
        # Recheck ages right before the single send: reads may have aged out
        # the update in the meantime.
        now = _host_clock()
        try:
            for asset in ASSET_ORDER:
                _check_age(quotes[asset][1], now)
            _check_age(header_us, now)
        except ValueError:
            raise KeeperFailure("quote-age") from None
        argv = _invoke_argv(
            deployment, deployment["contracts"]["oracle"], "update",
            payload=payload, send=True,
        )
        marker = _pending_marker(payload, header_us, argv)
        _write_pending(marker, runpath)
        code, stdout, stderr = _run_bounded(argv, receipt=runpath)
        if code != 0:
            raise KeeperFailure("update-send")
        for asset in ASSET_ORDER:
            usd7, feed_us = quotes[asset]
            published = _read_lastprice(deployment, deployment["tokens"][asset])
            if published != (usd7, _seconds(feed_us)):
                # Includes the permissionless-newer-publisher case: keep the
                # marker and fail closed; never replay automatically.
                raise KeeperFailure("oracle-readback", asset)
            _log("update", "done", asset, str(usd7), _seconds(feed_us))
        _clear_pending(_result_record(code, stdout, stderr), runpath, filename=RESULT_NAME)
        _clear_pending(marker, runpath)
        _log("cycle", "done", price=json.dumps(result, separators=(",", ":")))
        return result
    finally:
        os.close(lock_fd)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True, help="run directory holding rc-deployment.json")
    parser.add_argument("--stellar-bin", required=True, help="absolute path to a >= 26.1.0 stellar CLI")
    parser.add_argument("--credential-file", required=True, type=Path,
                        help="file holding the Pyth Lazer API key (systemd LoadCredential)")
    parser.add_argument("--check", action="store_true",
                        help="fetch, parse, and report quotes without journaling or chain writes")
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, _terminated)
    try:
        if args.check:
            check_quotes(args.run, args.stellar_bin, args.credential_file)
        else:
            run_cycle(args.run, args.stellar_bin, args.credential_file)
        return 0
    except KeeperFailure as failure:
        _log(failure.stage, "failed", failure.asset)
        return 1
    except KeyboardInterrupt:
        _log("interrupted", "failed")
        return 1
    except Exception:
        _log("internal", "failed")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
