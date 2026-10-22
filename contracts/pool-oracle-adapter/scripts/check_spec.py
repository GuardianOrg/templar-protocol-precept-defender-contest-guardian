#!/usr/bin/env python3
"""Assert the optimized pool-oracle-adapter WASM exports exactly the frozen ABI.

Runs `stellar contract info interface --wasm <optimized> --output json` and checks:
  * the exported function set is exactly the 12 frozen functions,
  * every argument/return shape matches (SEP-40 seven included),
  * the four referenced UDTs (PoolOracleConfig, Asset, PriceData, AdapterError)
    are present with their frozen field/case shapes.

The Stellar CLI walks up parent directories loading a `.env` (and binds
STELLAR_CONTRACT_ID/STELLAR_WASM_HASH env vars), which would collide with --wasm;
so the subprocess runs from a fresh temp dir with all STELLAR_* vars stripped,
and the --wasm path is absolute. CLI info lines go to stderr; we parse stdout.
"""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

DEFAULT_WASM = "target/wasm32v1-none/release-soroban-checked/pool_oracle_adapter.optimized.wasm"

# Type helpers mirroring the ScSpecTypeDef JSON encoding.
ADDRESS = "address"
U32 = "u32"
U64 = "u64"
VOID = []
def udt(name):
    return {"udt": {"name": name}}
def opt(inner):
    return {"option": {"value_type": inner}}
def vec(inner):
    return {"vec": {"element_type": inner}}

ASSET = udt("Asset")
PRICE_DATA = udt("PriceData")
CONFIG = udt("PoolOracleConfig")

def fn(name, inputs, outputs):
    return {"name": name, "inputs": [{"name": n, "type_": t} for n, t in inputs], "outputs": outputs}

EXPECTED = {
    f["name"]: f
    for f in [
        fn("__constructor",
           [("deployment_owner", ADDRESS), ("parent_oracle", ADDRESS),
            ("decimals", U32), ("resolution", U32), ("base", ASSET)],
           [{"result": {"ok_type": "void", "error_type": udt("AdapterError")}}]),
        fn("config", [], [CONFIG]),
        fn("interface_version", [], [U32]),
        fn("storage_version", [], [U32]),
        fn("extend_ttl", [], VOID),
        fn("base", [], [ASSET]),
        fn("assets", [], [vec(ASSET)]),
        fn("decimals", [], [U32]),
        fn("resolution", [], [U32]),
        fn("price", [("asset", ASSET), ("timestamp", U64)], [opt(PRICE_DATA)]),
        fn("prices", [("asset", ASSET), ("records", U32)], [opt(vec(PRICE_DATA))]),
        fn("lastprice", [("asset", ASSET)], [opt(PRICE_DATA)]),
    ]
}

# Frozen UDT wire shapes, asserted from the interface JSON itself (observed
# canonical order: the pinned CLI's spec encoder sorts struct fields; union
# cases and error cases keep declaration/value order). Field lists are matched
# exactly with order, so renames, additions, removals, type changes, and
# error-discriminant drift all fail.
EXPECTED_UDTS = {
    "PoolOracleConfig": ("udt_struct_v0", [
        ("base", ASSET), ("decimals", U32), ("deployment_owner", ADDRESS),
        ("parent_oracle", ADDRESS), ("resolution", U32)]),
    "PriceData": ("udt_struct_v0", [("price", "i128"), ("timestamp", U64)]),
    "Asset": ("udt_union_v0", [
        {"tuple_v0": {"name": "Stellar", "type_": [ADDRESS]}},
        {"tuple_v0": {"name": "Other", "type_": ["symbol"]}}]),
    "AdapterError": ("udt_error_enum_v0", [
        ("AlreadyInitialized", 1), ("InvalidConfig", 2),
        ("ParentUnavailable", 3), ("ParentMismatch", 4)]),
}

# UDTs the proxy-oracle dependency carries into the spec without being part of
# the adapter's interface. Names only: listed so any further exported type fails.
TRANSITIVE_UDTS = {
    "CircuitBreakerConfig", "ContractError", "CumulativeChangeConfig", "MonotonicRunConfig",
    "NormalizedPrice", "ProxyConfig", "RearmConfig", "SetEnforcedConfig", "SorobanDecimal",
    "SourceConfig", "StepwiseChangeConfig", "WindowedChangeDeltaConfig",
}

failures = []
def check(ok, msg):
    if not ok:
        failures.append(msg)


def strip_doc(node):
    # CLI 26 attaches "doc" to every input/type entry; it is documentation,
    # not contract. Strip it recursively before shape comparison so expected
    # literals stay shape-only while every structural key is still compared.
    if isinstance(node, dict):
        return {k: strip_doc(v) for k, v in node.items() if k != "doc"}
    if isinstance(node, list):
        return [strip_doc(v) for v in node]
    return node


def load_spec(stellar, wasm):
    env = {k: v for k, v in os.environ.items() if not k.startswith("STELLAR_")}
    with tempfile.TemporaryDirectory() as tmp:
        # Same wrapper discipline as the justfile: the CLI inherits the
        # qualified toolchain env instead of the ambient shell's.
        try:
            proc = subprocess.run(
                ["pool-adapter-env", stellar, "contract", "info", "interface",
                 "--wasm", str(wasm), "--output", "json"],
                cwd=tmp, env=env, capture_output=True, text=True,
            )
        except FileNotFoundError:
            sys.exit("FAIL: pool-adapter-env not found on PATH (devenv provides it)")
    if proc.returncode != 0:
        sys.exit(f"FAIL: stellar CLI exited {proc.returncode}: {proc.stderr.strip()}")
    try:
        spec = json.loads(proc.stdout)
    except json.JSONDecodeError as e:
        sys.exit(f"FAIL: interface output is not JSON: {e}\nstdout: {proc.stdout[:500]}")
    if isinstance(spec, list):
        return spec
    if isinstance(spec, dict):
        for key in ("functions", "entries"):
            if isinstance(spec.get(key), list):
                return spec[key]
        sys.exit(f"FAIL: unexpected interface JSON shape; observed keys: {sorted(spec)}")
    sys.exit(f"FAIL: unexpected interface JSON type: {type(spec).__name__}")


def main():
    stellar = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("STELLAR_BIN", "stellar")
    wasm = Path(sys.argv[2] if len(sys.argv) > 2 else DEFAULT_WASM).resolve()
    if not wasm.is_file():
        sys.exit(f"FAIL: optimized WASM not found: {wasm} (run `just optimize` first)")

    entries = load_spec(stellar, wasm)
    functions = []
    udts = {}
    for i, entry in enumerate(entries):
        if not isinstance(entry, dict) or len(entry) != 1:
            check(False, f"entry {i} is not a single-key ScSpecEntry object: {entry}")
            continue
        kind, payload = next(iter(entry.items()))
        payload = strip_doc(payload)
        if kind == "function_v0":
            functions.append(payload)
        elif kind in ("udt_struct_v0", "udt_union_v0", "udt_error_enum_v0"):
            name = payload.get("name")
            check(name not in udts, f"duplicate UDT spec entry: {name}")
            udts[name] = (kind, payload)
        else:
            failures.append(f"unexpected spec entry kind: {kind}")

    names = [f.get("name") for f in functions]
    dupes = sorted({n for n in names if names.count(n) > 1})
    check(not dupes, f"duplicate exported functions: {dupes}")

    actual = {f.get("name"): f for f in functions}
    for name in sorted(set(names) - set(EXPECTED)):
        failures.append(f"unexpected exported function: {name}")
    for name in sorted(set(EXPECTED) - set(names)):
        failures.append(f"missing exported function: {name}")
    for name, want in EXPECTED.items():
        got = actual.get(name)
        if got is None:
            continue
        check(got.get("inputs") == want["inputs"],
              f"{name}: inputs mismatch\n  expected {json.dumps(want['inputs'])}\n  actual   {json.dumps(got.get('inputs'))}")
        check(got.get("outputs") == want["outputs"],
              f"{name}: outputs mismatch\n  expected {json.dumps(want['outputs'])}\n  actual   {json.dumps(got.get('outputs'))}")

    for name in sorted(set(udts) - set(EXPECTED_UDTS) - TRANSITIVE_UDTS):
        failures.append(f"unexpected UDT: {name}")

    for name, (kind, members) in EXPECTED_UDTS.items():
        got = udts.get(name)
        if got is None:
            failures.append(f"referenced UDT missing from spec: {name}")
            continue
        got_kind, payload = got
        check(got_kind == kind, f"{name}: expected spec entry kind {kind}, got {got_kind}")
        if got_kind != kind:
            continue
        if kind == "udt_error_enum_v0":
            actual_members = [(c.get("name"), c.get("value")) for c in payload.get("cases", [])]
        elif kind == "udt_union_v0":
            actual_members = payload.get("cases", [])
        else:
            actual_members = [(m.get("name"), m.get("type_")) for m in payload.get("fields", [])]
        check(actual_members == members,
              f"{name}: {kind} members mismatch\n  expected {json.dumps(members)}\n  actual   {json.dumps(actual_members)}")

    if failures:
        print(f"SPEC CHECK FAILED ({len(failures)} violation(s)):")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print(f"SPEC CHECK PASSED: exactly {len(EXPECTED)} functions and {len(EXPECTED_UDTS)} UDTs with frozen shapes")


if __name__ == "__main__":
    main()
