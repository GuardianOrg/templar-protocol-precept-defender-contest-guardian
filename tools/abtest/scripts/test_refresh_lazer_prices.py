#!/usr/bin/env python3
"""Boundary tests for the signed Lazer keeper; the real crypto lives on-chain."""
from __future__ import annotations

import base64
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location(
    "refresh_lazer_prices", Path(__file__).with_name("refresh_lazer_prices.py")
)
keeper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(keeper)

RAW = base64.b64decode(
    "5L1HTRQLg7jSqZxjz717QuX7RY01SuA5W1dR2sptOrK3f4q/EsmZjUxDI/oLueLOXElxZMnk"
    "KRie7iY52/N2Tcop0jgBRAB108eTwI/upmtdBgADAhcAAAADAOceJgEAAAAABPj/DAHAj+6m"
    "a10GAAcAAAADAIjL9QUAAAAABPj/DAHAj+6ma10GAA==",
    validate=True,
)
HEADER_US = 1791566803800000
BODY = json.dumps({
    "parsed": {"timestampUs": str(HEADER_US), "priceFeeds": [
        {"priceFeedId": 23, "price": "19275495", "exponent": -8,
         "feedUpdateTimestamp": HEADER_US},
        {"priceFeedId": 7, "price": "99994504", "exponent": -8,
         "feedUpdateTimestamp": HEADER_US},
    ]},
    "leEcdsa": {"encoding": "base64", "data": base64.b64encode(RAW).decode()},
}, separators=(",", ":")).encode()


def api_response(payload: bytes | None = None, float_feed: bool = False) -> bytes:
    body = json.loads(BODY)
    parsed, signed = body["parsed"], body["leEcdsa"]
    if payload is not None:
        signed["data"] = base64.b64encode(payload).decode()
    if float_feed:
        parsed["priceFeeds"][0]["feedUpdateTimestamp"] = float(HEADER_US)
    return json.dumps(body, separators=(",", ":")).encode()


def assert_failure(test: unittest.TestCase, stage: str, call) -> None:
    with test.assertRaises(keeper.KeeperFailure) as caught:
        call()
    test.assertEqual(caught.exception.stage, stage)


class ParseTests(unittest.TestCase):
    def parse(self, payload: bytes):
        return keeper.parse_signed_response(payload, HEADER_US)

    def test_valid_response_yields_payload_usd7_and_microseconds(self) -> None:
        payload, header_us, quotes = self.parse(BODY)
        self.assertEqual(payload, RAW)
        self.assertEqual(len(RAW), 139)
        self.assertEqual(header_us, HEADER_US)
        self.assertEqual(quotes["xlm"], (1927549, HEADER_US))
        self.assertEqual(quotes["usdc"], (9999450, HEADER_US))
        self.assertEqual(keeper._seconds(HEADER_US), 1791566803)

    def test_usd7_floor_uses_integer_division(self) -> None:
        self.assertEqual(keeper._floor_usd7("19275495"), 1927549)
        self.assertEqual(keeper._floor_usd7("19"), 1)

    def test_float_in_integer_field_is_rejected(self) -> None:
        assert_failure(self, "lazer-parse",
                       lambda: self.parse(api_response(float_feed=True)))

    def test_payload_too_small_is_rejected(self) -> None:
        assert_failure(self, "lazer-parse",
                       lambda: self.parse(api_response(payload=b"x" * 63)))

    def test_feed_set_exponent_and_duplicate_rules(self) -> None:
        missing = json.loads(BODY)
        missing["parsed"]["priceFeeds"] = missing["parsed"]["priceFeeds"][:1]
        dup = json.loads(BODY)
        dup["parsed"]["priceFeeds"][1]["priceFeedId"] = 23
        expo = json.loads(BODY)
        expo["parsed"]["priceFeeds"][0]["exponent"] = -7
        for value in (missing, dup, expo):
            payload = json.dumps(value, separators=(",", ":")).encode()
            assert_failure(self, "lazer-parse", lambda p=payload: self.parse(p))

    def test_feed_newer_than_header_is_rejected(self) -> None:
        future = json.loads(BODY)
        future["parsed"]["priceFeeds"][0]["feedUpdateTimestamp"] = HEADER_US + 1
        assert_failure(self, "lazer-parse", lambda: self.parse(
            json.dumps(future, separators=(",", ":")).encode()))

    def test_stale_and_future_dated_windows(self) -> None:
        assert_failure(self, "lazer-parse", lambda: keeper.parse_signed_response(
            BODY, HEADER_US + 61_000_000))
        assert_failure(self, "lazer-parse", lambda: keeper.parse_signed_response(
            BODY, HEADER_US - 16_000_000))


class HttpCustodyTests(unittest.TestCase):
    def test_redirects_are_refused(self) -> None:
        self.assertIsNone(keeper._NoRedirect().redirect_request(
            None, None, None, None, None))

    def test_credential_file_only_and_never_echoed(self) -> None:
        key = "sekret-token-do-not-leak"
        with tempfile.TemporaryDirectory() as td:
            credential = Path(td) / "key"
            credential.write_text(key)
            os.chmod(credential, 0o600)
            self.assertEqual(keeper._read_credential(credential), key)
            os.chmod(credential, 0o644)
            assert_failure(self, "credential-file",
                           lambda: keeper._read_credential(credential))
            os.chmod(credential, 0o600)
            with mock.patch.object(keeper.urllib.request, "build_opener") as opener:
                failure = urllib.error.HTTPError(
                    keeper.LAZER_URL, 500, f"echo {key}", {}, None)
                opener.return_value.open.side_effect = failure
                with self.assertRaises(keeper.KeeperFailure) as caught:
                    keeper._fetch_signed(credential)
                self.assertEqual(caught.exception.stage, "lazer-fetch")
                self.assertNotIn(key, str(caught.exception))
                self.assertTrue(failure.closed)

    def test_oversized_response_refused_and_closed(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            credential = Path(td) / "key"
            credential.write_text("test-credential")
            os.chmod(credential, 0o600)
            for size in (65536, 65537):
                response = io.BytesIO(b"x" * size)
                with mock.patch.object(keeper.urllib.request, "build_opener") as opener:
                    opener.return_value.open.return_value = response
                    if size == 65536:
                        self.assertEqual(len(keeper._fetch_signed(credential)), size)
                    else:
                        assert_failure(self, "lazer-fetch",
                                       lambda: keeper._fetch_signed(credential))
                self.assertTrue(response.closed)


class DeploymentTests(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.runpath = Path(self._td.name) / "run"
        self.runpath.mkdir()
        self.addCleanup(self._td.cleanup)
        deployment = {
            "network_label": "testnet",
            "rpc_url": keeper.RPC_URL,
            "passphrase": keeper.PASSPHRASE,
            "admin": "G" + "A" * 55,
            "signer": {"alias": "admin-key", "config_dir": "/tmp/admin-cfg"},
            "keeper_signer": {"alias": "keeper-key", "config_dir": "/tmp/keeper-cfg"},
            "tokens": dict(keeper.TOKEN_CONTRACTS),
            "contracts": {
                "pool": "C" + "P" * 55,
                "oracle": "C" + "O" * 55,
                "verifier": keeper.VERIFIER,
                "backstop": "C" + "B" * 55,
                "factory": "C" + "F" * 55,
                "comet": "C" + "E" * 55,
                "emitter": "C" + "M" * 55,
            },
            "artifact_sha256": {},
        }
        artifacts = self.runpath / "artifacts"
        artifacts.mkdir()
        for role, filename in keeper.ARTIFACT_FILES.items():
            blob = role.encode()
            (artifacts / f"{filename}.wasm").write_bytes(blob)
            deployment["artifact_sha256"][role] = hashlib.sha256(blob).hexdigest()
        self.deployment = deployment
        self.path = self.runpath / "rc-deployment.json"

    def write(self, deployment: dict) -> None:
        self.path.write_text(json.dumps(deployment))

    def test_non_testnet_and_asset_mismatches_refused(self) -> None:
        for patch in (
            {"network_label": "public"},
            {"passphrase": "Public Global Stellar Network ; September 2015"},
        ):
            bad = dict(self.deployment)
            bad.update(patch)
            self.write(bad)
            assert_failure(self, "deployment-shape",
                           lambda: keeper.load_deployment(self.runpath))
        bad = dict(self.deployment)
        bad["tokens"] = {"xlm": "C" + "X" * 55, "usdc": "C" + "U" * 55}
        self.write(bad)
        assert_failure(self, "deployment-shape",
                       lambda: keeper.load_deployment(self.runpath))

    def test_wrong_verifier_refused(self) -> None:
        bad = dict(self.deployment)
        bad["contracts"] = dict(bad["contracts"], verifier="C" + "V" * 55)
        self.write(bad)
        assert_failure(self, "deployment-shape",
                       lambda: keeper.load_deployment(self.runpath))

    def test_artifact_pins_and_files(self) -> None:
        bad = dict(self.deployment)
        bad["artifact_sha256"] = dict(bad["artifact_sha256"], pool="0" * 64)
        self.write(bad)
        assert_failure(self, "deployment-shape",
                       lambda: keeper.load_deployment(self.runpath))
        self.assertEqual(sorted(keeper.ARTIFACT_FILES), ["oracle", "pool", "verifier"])
        self.assertEqual(keeper.ARTIFACT_FILES["oracle"], "pyth-lazer-oracle")

    def test_keeper_signer_must_differ_from_admin(self) -> None:
        bad = dict(self.deployment)
        bad["keeper_signer"] = dict(bad["signer"])
        self.write(bad)
        assert_failure(self, "deployment-shape",
                       lambda: keeper.load_deployment(self.runpath))


class CycleTests(unittest.TestCase):
    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.runpath = Path(self._td.name)
        self.addCleanup(self._td.cleanup)
        self.calls: list[list[str]] = []
        deployment = {
            "tokens": dict(keeper.TOKEN_CONTRACTS),
            "contracts": {"oracle": "C" + "O" * 55},
            "_stellar_bin": "/bin/stellar",
            "keeper_public_key": "G" + "K" * 55,
            "keeper_signer": {"alias": "keeper-key", "config_dir": "/tmp/keeper-cfg"},
        }
        for name, value in (
            ("load_deployment", deployment),
            ("resolve_runtime", None),
            ("_verify_verifier_binding", None),
            ("_verify_network_identity", None),
            ("_verify_chain_code_hashes", None),
            ("_fetch_signed", BODY),
        ):
            patcher = mock.patch.object(keeper, name, return_value=value)
            setattr(self, name, patcher.start())
        self._host_clock = mock.patch.object(keeper, "_host_clock",
                                            return_value=HEADER_US).start()
        self._read_lastprice = mock.patch.object(keeper, "_read_lastprice").start()
        self._run_bounded = mock.patch.object(keeper, "_run_bounded").start()
        self.addCleanup(mock.patch.stopall)

    def stub_reads(self, values: list) -> None:
        self.reads = values
        self.read_index = 0

        def read(deployment, asset):
            value = self.reads[self.read_index]
            self.read_index += 1
            return value
        self._read_lastprice.side_effect = read

    def stub_send(self, code: int = 0) -> None:
        self.marker = None

        def run(argv, *args, receipt=None, **kwargs):
            self.calls.append(argv)
            # Mirror the real runner: pending custody and result evidence are
            # durable before this call returns, whatever the send outcome.
            if receipt is not None:
                self.marker = (receipt / keeper.PENDING_NAME).read_bytes()
                keeper._write_pending(keeper._result_record(code, b"", b""),
                                      receipt, filename=keeper.RESULT_NAME)
            return code, b"", b""
        self._run_bounded.side_effect = run

    def test_pending_marker_blocks_before_any_side_effect(self) -> None:
        (self.runpath / keeper.PENDING_NAME).write_text("stuck\n")
        assert_failure(self, "unresolved-marker",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.resolve_runtime.assert_not_called()
        self._fetch_signed.assert_not_called()
        self.assertEqual(self.calls, [])

    def test_unchanged_prices_send_nothing(self) -> None:
        _, _, quotes = keeper.parse_signed_response(BODY, HEADER_US)
        self.stub_reads([(quotes["xlm"][0], keeper._seconds(quotes["xlm"][1])),
                         (quotes["usdc"][0], keeper._seconds(quotes["usdc"][1]))])
        self.stub_send()
        result = keeper.run_cycle(self.runpath, "/bin/stellar", Path("/tmp/k"))
        self.assertEqual(result, {"xlm": "unchanged", "usdc": "unchanged"})
        self.assertEqual(self.calls, [])
        self.assertFalse((self.runpath / keeper.PENDING_NAME).exists())

    def test_single_update_then_readback_clears_marker(self) -> None:
        payload, _, quotes = keeper.parse_signed_response(BODY, HEADER_US)
        self.stub_reads([None, None,
                         (quotes["xlm"][0], keeper._seconds(quotes["xlm"][1])),
                         (quotes["usdc"][0], keeper._seconds(quotes["usdc"][1]))])
        self.stub_send()
        result = keeper.run_cycle(self.runpath, "/bin/stellar", Path("/tmp/k"))
        self.assertEqual(result, {"xlm": "update", "usdc": "update"})
        self.assertEqual(len(self.calls), 1)
        argv = self.calls[0]
        self.assertIn("update", argv)
        index = argv.index("--payload")
        self.assertEqual(bytes.fromhex(argv[index + 1]), payload)
        self.assertEqual(json.loads(self.marker)["argv"], argv)
        self.assertFalse((self.runpath / keeper.RESULT_NAME).exists())
        self.assertFalse((self.runpath / keeper.PENDING_NAME).exists())

    def test_age_check_after_reads_blocks_send(self) -> None:
        self.stub_reads([None, None])
        self.stub_send()
        # Real logic: clock advances past the 60s window between the
        # initial parse and the pre-send recheck.
        self._host_clock.side_effect = [HEADER_US, HEADER_US + 61_000_000]
        assert_failure(self, "quote-age",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.assertEqual(self.calls, [])
        # Nothing was submitted: the marker only guards the transaction window.
        self.assertFalse((self.runpath / keeper.PENDING_NAME).exists())

    def test_older_onchain_price_fails_closed(self) -> None:
        self.stub_reads([(1, HEADER_US // 1_000_000 + 10),
                         (1, HEADER_US // 1_000_000 + 10)])
        self.stub_send()
        assert_failure(self, "quote-not-newer",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.assertEqual(self.calls, [])

    def test_readback_mismatch_keeps_marker_and_no_replay(self) -> None:
        self.stub_reads([None, None, (999, 1), (999, 1)])
        self.stub_send()
        assert_failure(self, "oracle-readback",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.assertEqual(len(self.calls), 1)
        self.assertTrue((self.runpath / keeper.PENDING_NAME).exists())
        self.assertTrue((self.runpath / keeper.RESULT_NAME).exists())

    def test_send_failure_retains_receipt_and_blocks_next_send(self) -> None:
        self.stub_reads([None, None])
        self.stub_send(code=1)
        assert_failure(self, "update-send",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.assertTrue((self.runpath / keeper.PENDING_NAME).exists())
        record = json.loads((self.runpath / keeper.RESULT_NAME).read_bytes())
        self.assertEqual(record["exit"], 1)
        # No replay: retained evidence blocks the next cycle before any effect.
        assert_failure(self, "unresolved-marker",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.assertEqual(len(self.calls), 1)

    def test_orphan_result_sidecar_blocks_cycle(self) -> None:
        keeper._write_pending(keeper._result_record(0, b"", b""), self.runpath,
                              filename=keeper.RESULT_NAME)
        assert_failure(self, "unresolved-marker",
                       lambda: keeper.run_cycle(self.runpath, "/bin/stellar",
                                                Path("/tmp/k")))
        self.resolve_runtime.assert_not_called()


class BoundedEvidenceTests(unittest.TestCase):
    """Receipt evidence survives an aborted bounded child, and the child dies."""

    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.runpath = Path(self._td.name)
        self.addCleanup(self._td.cleanup)

    def test_timeout_receipt_retains_partial_output(self) -> None:
        pidfile = self.runpath / "child.pid"
        child = (
            "import os, sys, time;"
            "sys.stdout.buffer.write(b'partial-out');"
            "sys.stderr.buffer.write(b'partial-err');"
            "sys.stdout.flush(); sys.stderr.flush();"
            # Pidfile after flushes: its existence proves the bytes are already
            # in the pipe, so the receipt is deterministic even if the selector
            # has not consumed them yet (the runner drains before close).
            f"open({str(pidfile)!r}, 'wb').write(str(os.getpid()).encode());"
            "time.sleep(60)"
        )
        assert_failure(self, "child-timeout",
                       lambda: keeper._run_bounded(
                           [sys.executable, "-c", child], timeout=1,
                           receipt=self.runpath))
        record = json.loads((self.runpath / keeper.RESULT_NAME).read_bytes())
        self.assertLess(record["exit"], 0)  # killed by the runner
        self.assertEqual(record["failure"], "child-timeout")
        self.assertEqual(base64.b64decode(record["stdout_base64"]), b"partial-out")
        self.assertEqual(base64.b64decode(record["stderr_base64"]), b"partial-err")
        pid = int(pidfile.read_bytes())
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.05)
        else:
            self.fail("timed-out child was not cleaned up")

    def test_output_limit_receipt_retains_bounded_output(self) -> None:
        child = "import sys; sys.stdout.buffer.write(b'x' * 8192); sys.stdout.flush()"
        assert_failure(self, "child-output-limit",
                       lambda: keeper._run_bounded(
                           [sys.executable, "-c", child], timeout=30,
                           output_limit=64, receipt=self.runpath))
        record = json.loads((self.runpath / keeper.RESULT_NAME).read_bytes())
        self.assertEqual(record["failure"], "child-output-limit")
        self.assertEqual(base64.b64decode(record["stdout_base64"]), b"x" * 64)


class TerminatedSendTests(unittest.TestCase):
    """SIGTERM mid-send: custody and receipt survive, the child dies."""

    def setUp(self) -> None:
        self._td = tempfile.TemporaryDirectory()
        self.runpath = Path(self._td.name)
        self.addCleanup(self._td.cleanup)

    def wait_for(self, *paths: Path, timeout: float = 10) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if all(path.exists() for path in paths):
                return True
            time.sleep(0.05)
        return False

    def test_sigterm_during_send_retains_custody_and_reaps_child(self) -> None:
        pidfile = self.runpath / "child.pid"
        child_code = (
            "import os, sys, time;"
            "sys.stdout.buffer.write(b'partial-out');"
            "sys.stderr.buffer.write(b'partial-err');"
            "sys.stdout.flush(); sys.stderr.flush();"
            # Pidfile after flushes: the bytes are provably in the pipe before
            # the test may terminate, so SIGTERM cannot lose them.
            "open(%r, 'wb').write(str(os.getpid()).encode());"
            "time.sleep(60)"
        ) % str(pidfile)
        # Real keeper main() with real SIGTERM registration; only the cycle
        # callback is replaced, and only to launch a real bounded child.
        # No auth, no network, no fakes: the wrapper script is test scaffolding.
        wrapper = self.runpath / "keeper_main.py"
        wrapper.write_text((
            "import importlib.util, sys\n"
            "spec = importlib.util.spec_from_file_location('keeper', %r)\n"
            "keeper = importlib.util.module_from_spec(spec)\n"
            "spec.loader.exec_module(keeper)\n"
            "def fake_cycle(runpath, stellar_bin, credential_file):\n"
            "    keeper._write_pending(keeper._pending_marker(b'x', 1, ['fake']), runpath)\n"
            "    keeper._run_bounded([sys.executable, '-c', %r], receipt=runpath)\n"
            "    return {}\n"
            "keeper.run_cycle = fake_cycle\n"
            "sys.argv = ['keeper', '--run', %r, '--stellar-bin', '/bin/stellar',"
            " '--credential-file', %r]\n"
            "raise SystemExit(keeper.main())\n"
        ) % (str(Path(keeper.__file__)), child_code,
             str(self.runpath), str(self.runpath / "credential")))
        proc = subprocess.Popen([sys.executable, str(wrapper)],
                                stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL)
        self.addCleanup(proc.kill)
        self.addCleanup(proc.stdout.close)
        # The marker proves the handler is registered: the send window is open.
        self.assertTrue(self.wait_for(self.runpath / keeper.PENDING_NAME, pidfile),
                        "keeper never reached the send window")
        proc.terminate()  # systemd-style SIGTERM
        self.assertEqual(proc.wait(timeout=30), 1)
        # Original marker retained; receipt holds the bounded partial output.
        self.assertTrue((self.runpath / keeper.PENDING_NAME).exists())
        record = json.loads((self.runpath / keeper.RESULT_NAME).read_bytes())
        self.assertEqual(record["failure"], "terminated")
        self.assertLess(record["exit"], 0)  # child killed by the runner
        self.assertEqual(base64.b64decode(record["stdout_base64"]), b"partial-out")
        self.assertEqual(base64.b64decode(record["stderr_base64"]), b"partial-err")
        # The fixed stage is logged with no signal/exception/key content.
        self.assertEqual(json.loads(proc.stdout.read().decode())["stage"], "terminated")
        # The bounded child is reaped, not orphaned to systemd's SIGKILL.
        pid = int(pidfile.read_bytes())
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return
            time.sleep(0.05)
        self.fail("SIGTERM-ed child was not cleaned up")


class NetworkPinTests(unittest.TestCase):
    def deployment(self) -> dict:
        return {
            "_stellar_bin": "/bin/stellar",
            "keeper_signer": {"alias": "keeper-key", "config_dir": "/tmp/keeper-cfg"},
            "contracts": {"pool": "C" + "P" * 55, "oracle": "C" + "O" * 55,
                          "verifier": keeper.VERIFIER},
            "artifact_sha256": {
                "pool": "1" * 64, "oracle": "2" * 64, "verifier": "3" * 64,
            },
        }

    def test_correct_passphrase_accepted_wrong_refused(self) -> None:
        with mock.patch.object(keeper, "_run_bounded") as run:
            run.return_value = (0, json.dumps({"passphrase": keeper.PASSPHRASE}).encode(), b"")
            keeper._verify_network_identity(self.deployment())
            run.return_value = (
                0, b'{"passphrase": "Public Global Stellar Network ; September 2015"}', b"")
            assert_failure(self, "rpc-passphrase",
                           lambda: keeper._verify_network_identity(self.deployment()))
            run.return_value = (1, b"", b"rpc down")
            assert_failure(self, "rpc-passphrase",
                           lambda: keeper._verify_network_identity(self.deployment()))

    def test_chain_hash_pin_binds_every_role(self) -> None:
        deployment = self.deployment()
        pins = deployment["artifact_sha256"]
        with mock.patch.object(keeper, "_run_bounded") as run:
            def reply(argv, *args, **kwargs):
                role = next(r for r in ("pool", "oracle", "verifier")
                            if deployment["contracts"][r] in argv)
                return 0, pins[role].encode() + b"\n", b""
            run.side_effect = reply
            keeper._verify_chain_code_hashes(deployment)
            self.assertEqual(run.call_count, 3)
            run.side_effect = lambda argv, *a, **k: (0, b"9" * 64, b"")
            assert_failure(self, "chain-hash",
                           lambda: keeper._verify_chain_code_hashes(deployment))
            run.side_effect = lambda argv, *a, **k: (1, b"", b"missing")
            assert_failure(self, "chain-hash",
                           lambda: keeper._verify_chain_code_hashes(deployment))


if __name__ == "__main__":
    unittest.main()
