#!/usr/bin/env python3
"""Live JSON-RPC test for #17: ExecuteTransaction multi-statement PartiQL."""
import json
import subprocess
import uuid

PLUGIN = "./target/release/dynamodb-plugin.exe"
PARAMS = {
    "params": {
        "region": "us-east-1",
        "access_key_id": "local",
        "secret_access_key": "local",
        "endpoint": "http://localhost:8000",
    }
}
TABLE = "test_users"  # HASH id (S), RANGE created_at (S)

passed = failed = 0


def rpc(method, params):
    req = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    proc = subprocess.run(
        [PLUGIN], input=json.dumps(req) + "\n",
        capture_output=True, text=True, timeout=30,
    )
    out = proc.stdout.strip().splitlines()
    if not out:
        raise RuntimeError(f"no output; stderr={proc.stderr[:300]}")
    return json.loads(out[-1])


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"  PASS  {name}")
    else:
        failed += 1
        print(f"  FAIL  {name}  {detail}")


def cleanup(ids):
    for cid in ids:
        rpc("delete_record", {**PARAMS, "table": TABLE,
                              "key": {"id": cid, "created_at": "2026-03-01"}})


# Clean prior state.
cleanup(["tx-1", "tx-2"])

print("== #17: multi-statement transaction (2 INSERTs) ==")
txn_sql = (
    f"INSERT INTO \"{TABLE}\" VALUE {{'id': 'tx-1', 'created_at': '2026-03-01', 'name': 'T1'}}; "
    f"INSERT INTO \"{TABLE}\" VALUE {{'id': 'tx-2', 'created_at': '2026-03-01', 'name': 'T2'}}"
)
r = rpc("execute_query", {**PARAMS, "query": txn_sql})
check("transaction succeeds (no error)", "error" not in r, json.dumps(r)[:200])
check("affected_rows == 2", r.get("result", {}).get("affected_rows") == 2,
      f"got {r.get('result', {}).get('affected_rows')}")

# Both rows should exist.
rb = rpc("execute_query", {**PARAMS, "query": f'SELECT id FROM "{TABLE}" WHERE id = \'tx-1\''})
check("tx-1 committed", len(rb.get("result", {}).get("rows", [])) == 1)
rb2 = rpc("execute_query", {**PARAMS, "query": f'SELECT id FROM "{TABLE}" WHERE id = \'tx-2\''})
check("tx-2 committed", len(rb2.get("result", {}).get("rows", [])) == 1)

print("== #17: atomicity — one bad statement rolls back all ==")
cleanup(["tx-3"])
atomic_sql = (
    f"INSERT INTO \"{TABLE}\" VALUE {{'id': 'tx-3', 'created_at': '2026-03-01', 'name': 'OK'}}; "
    f"INSERT INTO \"no_such_table_xyz\" VALUE {{'id': 'x'}}"
)
ra = rpc("execute_query", {**PARAMS, "query": atomic_sql})
check("mixed transaction rejected", "error" in ra, f"got: {json.dumps(ra)[:200]}")
# tx-3 must NOT have been committed (atomic rollback).
rb3 = rpc("execute_query", {**PARAMS, "query": f'SELECT id FROM "{TABLE}" WHERE id = \'tx-3\''})
check("tx-3 rolled back (not present)", len(rb3.get("result", {}).get("rows", [])) == 0,
      f"rows={rb3.get('result', {}).get('rows')}")

print("== #17: single statement still uses ExecuteStatement path ==")
single = rpc("execute_query", {**PARAMS, "query": f'SELECT id FROM "{TABLE}" WHERE id = \'tx-1\''})
check("single statement works", "error" not in single and len(single.get("result", {}).get("rows", [])) == 1)

print("== #17: empty transaction rejected ==")
re = rpc("execute_query", {**PARAMS, "query": "   ;  ;  "})
# split_statements yields 0 statements -> falls through to ExecuteStatement of empty body -> error
check("empty/whitespace-only statements error", "error" in re or re.get("result", {}).get("affected_rows", 1) != 0,
      f"got: {json.dumps(re)[:200]}")

# cleanup
cleanup(["tx-1", "tx-2", "tx-3"])

print(f"\n{'='*40}\n  {passed} passed, {failed} failed")
raise SystemExit(1 if failed else 0)
