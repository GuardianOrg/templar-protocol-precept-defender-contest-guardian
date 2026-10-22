#!/usr/bin/env python3
"""Offline boundary checks for the dedicated original-market price keeper."""
from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

SOURCE = Path(__file__).with_name("refresh_spoon_prices.py")
SPEC = importlib.util.spec_from_file_location("refresh_spoon_prices", SOURCE)
assert SPEC is not None and SPEC.loader is not None
keeper = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(keeper)

TOKENS = {name: "C" + chr(ord("A") + index) * 55 for index, name in enumerate(keeper.ASSETS)}
SOURCE_ID = "C" + "Z" * 55


def expect_failure(action, stage: str) -> keeper.RefreshFailure:
    try:
        action()
    except keeper.RefreshFailure as error:
        assert error.stage == stage
        return error
    raise AssertionError(f"expected safe refusal at {stage}")

def expect_value_error(action) -> None:
    try:
        action()
    except ValueError:
        return
    raise AssertionError("expected malformed action arguments to fail closed")


def test_main_help_exits_before_refresh_cycle() -> None:
    with patch.object(sys, "argv", [str(SOURCE), "--help"]), patch.object(
        keeper, "refresh_cycle", side_effect=AssertionError("help must not refresh")
    ) as refresh, patch("sys.stdout"):
        try:
            keeper.main()
        except SystemExit as error:
            assert error.code == 0
        else:
            raise AssertionError("--help must exit 0 before entering the cycle")
        finally:
            refresh.assert_not_called()


def test_main_dry_run_is_rejected_before_refresh_cycle() -> None:
    with patch.object(sys, "argv", [str(SOURCE), "--dry-run"]), patch.object(
        keeper, "refresh_cycle", side_effect=AssertionError("unsupported flag must not refresh")
    ) as refresh, patch("sys.stderr"), patch("sys.stdout"):
        try:
            keeper.main()
        except SystemExit as error:
            assert error.code == 2
        else:
            raise AssertionError("--dry-run must exit 2 before entering the cycle")
        finally:
            refresh.assert_not_called()


def test_market_flag_rebinds_every_pin_before_the_cycle() -> None:
    seen = {}

    def capture():
        seen.update(run=keeper.RUN, sha=keeper.MANIFEST_SHA256, ledger=keeper.DEPLOY_LEDGER)

    live, invariants = keeper.MARKETS["live"], keeper.MARKETS["invariants"]
    assert live != invariants and len({m[1] for m in keeper.MARKETS.values()}) == len(keeper.MARKETS)
    try:
        with patch.object(sys, "argv", [str(SOURCE), "--market", "invariants"]), patch.object(
            keeper, "refresh_cycle", side_effect=capture
        ), patch("sys.stdout"):
            assert keeper.main() == 0
        assert (seen["run"], seen["sha"], seen["ledger"]) == invariants
        with patch.object(sys, "argv", [str(SOURCE)]), patch.object(
            keeper, "refresh_cycle", side_effect=capture
        ), patch("sys.stdout"):
            assert keeper.main() == 0
        assert (seen["run"], seen["sha"], seen["ledger"]) == live
        with patch.object(sys, "argv", [str(SOURCE), "--market", "mainnet"]), patch.object(
            keeper, "refresh_cycle", side_effect=AssertionError("unknown market must not refresh")
        ) as refresh, patch("sys.stderr"), patch("sys.stdout"):
            try:
                keeper.main()
            except SystemExit as error:
                assert error.code == 2
            else:
                raise AssertionError("unknown market must exit 2 before entering the cycle")
            refresh.assert_not_called()
    finally:
        keeper.select_market("live")


def test_quote_and_registry_boundaries() -> None:
    assert keeper._parse_quote({"price": "20000000000", "timestamp": 1790778675}) == (20_000_000_000, 1_790_778_675)
    for quote in (
        {"price": "0", "timestamp": 1},
        {"price": "01", "timestamp": 1},
        {"price": "1.0", "timestamp": 1},
        {"price": str(1 << 127), "timestamp": 1},
        {"price": "1", "timestamp": True},
        {"price": "1", "timestamp": "1"},
        {"price": "1", "timestamp": 0},
        {"price": "1", "timestamp": 1, "foreign": "x"},
    ):
        try:
            keeper._parse_quote(quote)
        except ValueError:
            pass
        else:
            raise AssertionError("malformed source price passed its boundary")

    registry = [{"Stellar": TOKENS[name]} for name in keeper.ASSETS]
    keeper._parse_registered_assets(registry, TOKENS)
    for wrong in (registry[:-1], registry + [{"Stellar": SOURCE_ID}], [*registry[:-1], registry[0]]):
        try:
            keeper._parse_registered_assets(wrong, TOKENS)
        except ValueError:
            pass
        else:
            raise AssertionError("non-exact registered assets passed their boundary")


def test_invalid_native_actions_cannot_be_built() -> None:
    for method, price, timestamp in (
        ("transfer", None, None),
        ("set_data", "0", 1),
        ("set_data", str(1 << 127), 1),
        ("set_data", "1", True),
        ("refresh", "1", 1),
    ):
        expect_value_error(
            lambda: keeper._native_argv(SOURCE_ID, method, TOKENS["weth"], price, timestamp)
        )
    expect_value_error(lambda: keeper._native_argv(SOURCE_ID, "refresh", "blnd"))


def test_pending_marker_blocks_replay_after_ambiguous_send() -> None:
    with tempfile.TemporaryDirectory() as directory:
        run_dir = Path(directory)
        payload = keeper._pending_marker("weth", 0, 20_000_000_000, 1_790_778_676)
        keeper._write_pending(payload, run_dir)
        expect_failure(lambda: keeper._ensure_no_pending(run_dir), "unresolved-marker")
        expect_failure(lambda: keeper._clear_pending(payload + b"x", run_dir), "marker-clear")
        assert (run_dir / keeper.PENDING_NAME).read_bytes() == payload
        keeper._clear_pending(payload, run_dir)
        keeper._ensure_no_pending(run_dir)

        counter = run_dir / "invocations"
        fake = run_dir / "fake.py"
        fake.write_text(
            "import pathlib, sys\n"
            "path = pathlib.Path(sys.argv[1])\n"
            "with path.open('a', encoding='ascii') as out: out.write('called\\n')\n"
            "sys.stderr.write('private-looking-diagnostic-must-not-escape')\n"
            "raise SystemExit(9)\n",
            encoding="utf-8",
        )
        keeper._write_pending(payload, run_dir)
        error = expect_failure(lambda: keeper._send_once([sys.executable, str(fake), str(counter)]), "native-send")
        assert "private-looking-diagnostic" not in str(error)
        assert counter.read_text(encoding="ascii").splitlines() == ["called"]
        assert (run_dir / keeper.PENDING_NAME).read_bytes() == payload


def test_existing_uncertainty_blocks_the_entire_next_cycle() -> None:
    with tempfile.TemporaryDirectory() as directory:
        run_dir = Path(directory)
        payload = keeper._pending_marker("weth", 0, 10, 2)
        keeper._write_pending(payload, run_dir)

        def forbid_cycle():
            raise AssertionError("cycle continued past an unresolved send")

        with patch.multiple(
            keeper,
            RUN=run_dir,
            _driver_lock=lambda: os.open(run_dir / "lock", os.O_CREAT | os.O_RDWR, 0o600),
            _load_manifest=forbid_cycle,
        ):
            expect_failure(keeper.refresh_cycle, "unresolved-marker")
        assert (run_dir / keeper.PENDING_NAME).read_bytes() == payload


def test_concurrent_repricing_is_not_overwritten() -> None:
    reads = 0

    def read_source(*_):
        nonlocal reads
        reads += 1
        return (10 if reads <= 12 else 11, 1)

    def forbid_write(*_):
        raise AssertionError("changed source must abort before sending or writing a marker")

    with tempfile.TemporaryDirectory() as directory:
        run_dir = Path(directory)
        with patch.multiple(
            keeper,
            _driver_lock=lambda: os.open(run_dir / "lock", os.O_CREAT | os.O_RDWR, 0o600),
            _ensure_no_pending=lambda _run: None,
            _load_manifest=lambda: {"tokens": TOKENS},
            _verify_network_and_signer=lambda: None,
            _market_json=lambda *_: [{"Stellar": TOKENS[asset]} for asset in keeper.ASSETS],
            _read_source=read_source,
            _ledger_seconds=lambda: 2,
            _write_pending=forbid_write,
            _send_once=forbid_write,
        ):
            expect_failure(keeper.refresh_cycle, "publish-source-changed")




def verify_adapter(
    adapter_quote, runtime_timestamp: int, publish_times: list[int], now: int,
    *, resolution: int = 1, max_age: int = 3600, max_drift: int = 60,
) -> None:
    with patch.object(keeper, "_read_adapter", return_value=adapter_quote), patch.object(
        keeper, "_read_runtime_timestamp", return_value=runtime_timestamp
    ), patch.object(keeper, "_ledger_seconds", return_value=now):
        keeper._verify_adapter(
            "weth", TOKENS["weth"], 10, publish_times, max_age, max_drift,
            resolution=resolution,
        )


def test_runtime_timestamp_parser_rejects_malformed_normalized_prices() -> None:
    timestamp = 1_800_000_020
    normalized = {"mantissa": 10, "expo": -7, "timestamp": timestamp}
    with patch.object(keeper, "_market_json", return_value=normalized):
        assert keeper._read_runtime_timestamp("weth", TOKENS["weth"]) == timestamp

    for malformed in (
        None,
        {},
        {"mantissa": 10, "timestamp": timestamp},
        {**normalized, "unexpected": 1},
        {**normalized, "mantissa": True},
        {**normalized, "mantissa": 10.5},
        {**normalized, "mantissa": 1 << 63},
        {**normalized, "mantissa": -(1 << 63) - 1},
        {**normalized, "expo": False},
        {**normalized, "expo": -7.5},
        {**normalized, "expo": 1 << 31},
        {**normalized, "expo": -(1 << 31) - 1},
        {**normalized, "timestamp": True},
        {**normalized, "timestamp": timestamp + 0.5},
        {**normalized, "timestamp": 0},
        {**normalized, "timestamp": 1 << 64},
    ):
        with patch.object(keeper, "_market_json", return_value=malformed):
            expect_failure(
                lambda: keeper._read_runtime_timestamp("weth", TOKENS["weth"]),
                "runtime-quote",
            )
def test_adapter_freshness_uses_runtime_age_and_configured_future_drift() -> None:
    for observed, runtime_timestamp, now, publish_times, max_age in (
        ((11, 2), 2, 3, [2, 2, 2], 3600),
        ((10, 2), 2, 3603, [2, 2, 2], 3600),
        ((10, 64), 64, 3, [64, 64, 64], 3600),
    ):
        expect_failure(
            lambda: verify_adapter(
                observed, runtime_timestamp, publish_times, now, max_age=max_age
            ),
            "adapter-readback",
        )
    verify_adapter((10, 63), 63, [63, 63, 63], 3)


def test_adapter_uses_300_second_bucket_and_raw_current_cycle_timestamp() -> None:
    publish_times = [1_800_000_010, 1_800_000_020, 1_800_000_030]
    # MedianLow returns one selected source timestamp; all three same-price
    # publications project to the same adapter bucket.
    for runtime_timestamp in publish_times:
        verify_adapter(
            (10, 1_800_000_000), runtime_timestamp, publish_times, 1_800_000_030,
            resolution=300,
        )

    expect_failure(
        lambda: verify_adapter(
            (10, 1_800_000_001), 1_800_000_020, publish_times, 1_800_000_030,
            resolution=300,
        ),
        "adapter-readback",
    )

    # Neither pre-cycle nor unrelated in-bucket cache timestamps prove this
    # refresh: MedianLow's aggregate timestamp must come from a source publish.
    for cached_timestamp in (1_800_000_009, 1_800_000_015):
        expect_failure(
            lambda: verify_adapter(
                (10, 1_800_000_000), cached_timestamp, publish_times, 1_800_000_030,
                resolution=300,
            ),
            "adapter-readback",
        )

def test_adapter_rejects_nonpositive_or_boolean_resolution() -> None:
    for resolution in (0, -300, True):
        expect_failure(
            lambda: verify_adapter(
                (10, 0), 1, [1, 1, 1], 1, resolution=resolution,
            ),
            "adapter-readback",
        )

def test_ledger_clock_uses_rpc_time_and_rejects_boolean_integers() -> None:
    latest = {"id": "a" * 64, "protocolVersion": 23, "sequence": 321}
    fetched = {
        "latestLedger": 322,
        "latestLedgerCloseTime": 1_800_000_000,
        "ledgers": [{"sequence": 321, "ledgerCloseTime": "1700000000", "headerXdr": "x", "metadataXdr": "x"}],
    }
    results = [(0, json.dumps(latest).encode(), b""), (0, json.dumps(fetched).encode(), b"")]

    def fake_run(argv, timeout=keeper.CHILD_TIMEOUT, *, output_limit=keeper.OUTPUT_LIMIT):
        return results.pop(0)

    with patch.object(keeper, "_run_bounded", side_effect=fake_run), patch.object(
        keeper.time, "time", return_value=1
    ):
        assert keeper._ledger_seconds() == 1_800_000_000

    expect_value_error(lambda: keeper._parse_latest_sequence({"sequence": True}))
    expect_value_error(lambda: keeper._parse_latest_sequence({"sequence": keeper.MAX_U32 + 1}))
    bad_close = dict(fetched, latestLedgerCloseTime=True)
    expect_value_error(lambda: keeper._parse_latest_close_time(bad_close, 321))
    bad_latest = dict(fetched, latestLedger=True)
    expect_value_error(lambda: keeper._parse_latest_close_time(bad_latest, 321))


def test_dotenv_metadata_refuses_spawn_and_children_are_isolated() -> None:
    real_lstat = keeper.Path.lstat

    def root_env_present(path):
        if path == Path("/.env"):
            return object()
        return real_lstat(path)

    with patch.object(keeper.Path, "lstat", root_env_present), patch.object(
        keeper.subprocess, "Popen", side_effect=AssertionError("dotenv must refuse before spawn")
    ):
        expect_failure(lambda: keeper._run_bounded([sys.executable, "-c", "pass"]), "child-dotenv")

    def root_env_inaccessible(path):
        if path == Path("/.env"):
            raise PermissionError("metadata denied")
        return real_lstat(path)

    with patch.object(keeper.Path, "lstat", root_env_inaccessible), patch.object(
        keeper.subprocess, "Popen", side_effect=AssertionError("metadata failure must refuse before spawn")
    ):
        expect_failure(lambda: keeper._run_bounded([sys.executable, "-c", "pass"]), "child-dotenv")

    with tempfile.TemporaryDirectory() as directory:
        fake_crate = Path(directory) / "crate"
        fake_crate.mkdir()
        (fake_crate / ".env").write_text("STELLAR_RPC_URL=dotenv-hijack\n", encoding="ascii")
        child = (
            "import json, os, pathlib, sys\n"
            "if pathlib.Path.cwd() == pathlib.Path(sys.argv[1]):\n"
            "  for line in pathlib.Path('.env').read_text(encoding='ascii').splitlines():\n"
            "    key, value = line.split('=', 1); os.environ[key] = value\n"
            "print(json.dumps({'cwd': os.getcwd(), 'rpc': os.environ.get('STELLAR_RPC_URL'), "
            "'secret': os.environ.get('STELLAR_SECRET'), 'pythonpath': 'PYTHONPATH' in os.environ, "
            "'rustlog': 'RUST_LOG' in os.environ}))\n"
        )
        def root_env_missing(path):
            if path == Path("/.env"):
                raise FileNotFoundError
            return real_lstat(path)

        with patch.object(keeper, "CRATE", fake_crate), patch.object(
            keeper.Path, "lstat", root_env_missing
        ), patch.dict(
            os.environ,
            {
                "STELLAR_RPC_URL": "environment-hijack",
                "STELLAR_SECRET": "fixture-secret",
                "PYTHONPATH": "/poison",
                "RUST_LOG": "trace",
            },
        ):
            status, stdout, _ = keeper._run_bounded([sys.executable, "-c", child, str(fake_crate)])
        result = json.loads(stdout)
        assert status == 0
        assert result == {"cwd": "/", "rpc": None, "secret": None, "pythonpath": False, "rustlog": False}


def test_source_change_before_refresh_is_detected_even_if_median_is_same() -> None:
    prices = {source: {asset: (5, 10, 20)[source] for asset in keeper.ASSETS} for source in range(3)}
    reads: dict[tuple[str, int], int] = {}
    published: dict[tuple[str, int], tuple[int, int]] = {}
    markers = []
    sends = []
    clock = 1
    source_ids = ["C" + letter * 55 for letter in "EFG"]

    def read_source(source, asset, _contract):
        key = (asset, source)
        reads[key] = reads.get(key, 0) + 1
        count = reads[key]
        initial = (prices[source][asset], 1)
        if count <= 2:
            return initial
        if count == 3:
            return published[key]
        price, timestamp = published[key]
        if asset == "weth" and source == 2:
            return price + 1, timestamp
        return price, timestamp

    def pending(asset, source, price, timestamp):
        if source is not None:
            published[(asset, source)] = (price, timestamp)
        markers.append((asset, source))
        return b"pending"

    def ledger_seconds():
        nonlocal clock
        clock += 1
        return clock

    with tempfile.TemporaryDirectory() as directory:
        run_dir = Path(directory)
        manifest = {
            "tokens": TOKENS,
            "oracle": {
                "sources": source_ids,
                "runtime": "C" + "H" * 55,
                "max_age_secs": 3600,
                "max_clock_drift_secs": 60,
            },
        }
        with patch.multiple(
            keeper,
            _driver_lock=lambda: os.open(run_dir / "lock", os.O_CREAT | os.O_RDWR, 0o600),
            _ensure_no_pending=lambda _run: None,
            _load_manifest=lambda: manifest,
            _verify_network_and_signer=lambda: None,
            _market_json=lambda *_: [{"Stellar": TOKENS[asset]} for asset in keeper.ASSETS],
            _read_source=read_source,
            _ledger_seconds=ledger_seconds,
            _pending_marker=pending,
            _write_pending=lambda _payload, _run: None,
            _clear_pending=lambda _payload, _run: None,
            _send_once=lambda _argv: sends.append("sent"),
        ):
            error = expect_failure(keeper.refresh_cycle, "refresh-source-changed")
        assert error.asset == "weth" and error.source == 2
        assert len(sends) == 12
        assert len(markers) == 12 and all(source is not None for _, source in markers)


def test_full_cycle_verifies_adapter_at_adapter_resolution() -> None:
    # The deployed pool adapter buckets timestamps to its own configured resolution
    # (deploy.rs builds it at 1); keeper verification must expect ADAPTER_RESOLUTION,
    # not the oracle's source-side 300-second bucket.
    prices = {source: {asset: (5, 10, 20)[source] for asset in keeper.ASSETS} for source in range(3)}
    reads: dict[tuple[str, int], int] = {}
    published: dict[tuple[str, int], tuple[int, int]] = {}
    markers: list[tuple[str, int | None]] = []
    resolutions: list[int] = []
    # Ordered log of every durable-marker write, native send and marker clear.
    events: list[tuple[str, object]] = []
    open_marker: list[bytes] = []
    clock = 1
    source_ids = ["C" + letter * 55 for letter in "EFG"]

    def read_source(source, asset, _contract):
        key = (asset, source)
        reads[key] = reads.get(key, 0) + 1
        if reads[key] <= 2:
            return prices[source][asset], 1
        return published[key]

    def pending(asset, source, price, timestamp):
        if source is not None:
            published[(asset, source)] = (price, timestamp)
        markers.append((asset, source))
        # A distinct payload per operation, so the ordering assertion below can
        # tell the marker bracketing a send from any other operation's marker.
        return f"{asset}|{source}|{price}|{timestamp}".encode()

    def ledger_seconds():
        nonlocal clock
        clock += 1
        return clock

    def write_pending(payload, _run):
        assert not open_marker, "durable marker written while one is still pending"
        open_marker.append(payload)
        events.append(("write", payload))

    def send_once(argv):
        assert open_marker, "native send issued with no durable pending marker"
        events.append(("send", tuple(argv)))

    def clear_pending(payload, _run):
        assert open_marker == [payload], "pending marker cleared for another operation"
        open_marker.pop()
        events.append(("clear", payload))

    def record_verify(_asset, _contract, _price, _publish_times, _max_age, _max_drift, *, resolution):
        resolutions.append(resolution)

    manifest = {
        "tokens": TOKENS,
        "oracle": {
            "sources": source_ids,
            "runtime": "C" + "H" * 55,
            "max_age_secs": 3600,
            "max_clock_drift_secs": 60,
            "resolution": 300,
        },
    }
    with tempfile.TemporaryDirectory() as directory:
        run_dir = Path(directory)
        with patch.multiple(
            keeper,
            _driver_lock=lambda: os.open(run_dir / "lock", os.O_CREAT | os.O_RDWR, 0o600),
            _ensure_no_pending=lambda _run: None,
            _load_manifest=lambda: manifest,
            _verify_network_and_signer=lambda: None,
            _market_json=lambda *_: [{"Stellar": TOKENS[asset]} for asset in keeper.ASSETS],
            _read_source=read_source,
            _ledger_seconds=ledger_seconds,
            _pending_marker=pending,
            _write_pending=write_pending,
            _clear_pending=clear_pending,
            _send_once=send_once,
            _verify_adapter=record_verify,
        ):
            keeper.refresh_cycle()
        assert len(markers) == 12 + len(keeper.ASSETS)
        assert resolutions == [1] * (2 * len(keeper.ASSETS))
        assert not open_marker, "cycle finished with an unresolved pending marker"
        # Every native send -- 12 publishes then 4 refreshes -- must sit inside a
        # durable-marker write/send/clear triple for that same operation: no send
        # outside a marker, no marker spanning two operations.
        assert [kind for kind, _ in events] == ["write", "send", "clear"] * len(markers)
        runtime = manifest["oracle"]["runtime"]
        for index, (asset, source) in enumerate(markers):
            (_, payload), (_, argv), (_, cleared) = events[3 * index : 3 * index + 3]
            assert payload == cleared, f"{asset} send cleared a different operation's marker"
            fields = payload.decode().split("|")
            assert fields[:2] == [asset, str(source)]
            assert argv[argv.index("--") + 1] == (
                "refresh" if source is None else "set_data"
            )
            assert argv[argv.index("--id") + 1] == (
                runtime if source is None else source_ids[source]
            )
            assert any(TOKENS[asset] in part for part in argv)
            if source is not None:
                price, timestamp = published[(asset, source)]
                assert fields[2:] == [str(price), str(timestamp)]
                assert argv[argv.index("--price") + 1] == str(price)
                assert argv[argv.index("--timestamp") + 1] == str(timestamp)


def main() -> None:
    test_main_help_exits_before_refresh_cycle()
    test_main_dry_run_is_rejected_before_refresh_cycle()
    test_quote_and_registry_boundaries()
    test_invalid_native_actions_cannot_be_built()
    test_pending_marker_blocks_replay_after_ambiguous_send()
    test_existing_uncertainty_blocks_the_entire_next_cycle()
    test_dotenv_metadata_refuses_spawn_and_children_are_isolated()
    test_runtime_timestamp_parser_rejects_malformed_normalized_prices()
    test_ledger_clock_uses_rpc_time_and_rejects_boolean_integers()
    test_concurrent_repricing_is_not_overwritten()
    test_source_change_before_refresh_is_detected_even_if_median_is_same()
    test_adapter_freshness_uses_runtime_age_and_configured_future_drift()
    test_adapter_uses_300_second_bucket_and_raw_current_cycle_timestamp()
    test_adapter_rejects_nonpositive_or_boolean_resolution()
    test_full_cycle_verifies_adapter_at_adapter_resolution()
    print("refresh_spoon_prices: offline boundary checks passed")


if __name__ == "__main__":
    main()
