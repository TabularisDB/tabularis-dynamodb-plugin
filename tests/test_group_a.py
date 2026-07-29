#!/usr/bin/env python3
"""Live JSON-RPC test for Group A CRUD fixes (#19/#28 PutItem, #23 key validation, #21 nested types)."""
import json
import subprocess
import sys

PLUGIN = "../target/release/dynamodb-plugin.exe"
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
        [PLUGIN],
        input=json.dumps(req) + "\n",
        capture_output=True,
        text=True,
        timeout=30,
    )
    out = proc.stdout.strip().splitlines()
    if not out:
        raise RuntimeError(f"no output from plugin; stderr={proc.stderr[:300]}")
    return json.loads(out[-1])


def check(name, cond, detail=""):
    global passed, failed
    if cond:
        passed += 1
        print(f"  PASS  {name}")
    else:
        failed += 1
        print(f"  FAIL  {name}  {detail}")


def err_msg(resp):
    return resp.get("error", {}).get("message", "")


print("== #19/#28: insert_record via PutItem (large value + nested types) ==")
# Build an item >8KB to prove the 8KB ExecuteStatement limit is gone.
big = "x" * 12000
nested = {"a": {"b": {"c": 1}}, "list": [1, 2, {"d": True}]}
resp = rpc("insert_record", {**PARAMS, "table": TABLE, "data": {
    "id": "grp-a-1", "created_at": "2026-01-01",
    "big_field": big, "nested": nested, "num": 42, "flag": True,
}})
check("large+nested insert succeeds", "result" in resp and "error" not in resp, err_msg(resp))

# Read it back and confirm nested map/list round-trip as JSON objects, not strings (#21).
resp = rpc("execute_query", {**PARAMS, "query": f'SELECT * FROM "{TABLE}" WHERE id = \'grp-a-1\''})
rows = resp.get("result", {}).get("rows", [])
cols = resp.get("result", {}).get("columns", [])
check("readback returns the item", len(rows) == 1, f"cols={cols} rows={rows}")
if rows:
    row = dict(zip(cols, rows[0]))
    check("#21 nested map is an object (not string)",
          isinstance(row.get("nested"), dict), f"got {type(row.get('nested'))}: {row.get('nested')!r}")
    check("nested deep value intact", isinstance(row.get("nested"), dict) and row["nested"].get("a", {}).get("b", {}).get("c") in (1, "1"), str(row.get("nested")))
    check("nested list is an array", isinstance(row.get("nested"), dict) and isinstance(row["nested"].get("list"), list), str(row.get("nested")))

print("== #23: update/delete reject non-key columns in key object ==")
resp = rpc("update_record", {**PARAMS, "table": TABLE, "key": {"id": "grp-a-1", "created_at": "2026-01-01", "email": "x@y.z"}, "col_name": "big_field", "new_val": "y"})
check("update with non-key column rejected", "error" in resp and "not a key attribute" in err_msg(resp), err_msg(resp))
resp = rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"email": "x@y.z"}})
check("delete with non-key column rejected", "error" in resp and "not a key attribute" in err_msg(resp), err_msg(resp))

print("== #23: full-key requirement (missing sort key) ==")
resp = rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "grp-a-1"}})
check("delete missing sort key rejected", "error" in resp and "missing required key" in err_msg(resp), err_msg(resp))

print("== #23/#28: valid update + delete via native APIs ==")
resp = rpc("update_record", {**PARAMS, "table": TABLE, "key": {"id": "grp-a-1", "created_at": "2026-01-01"}, "col_name": "big_field", "new_val": "updated"})
check("valid update succeeds", "result" in resp and "error" not in resp, err_msg(resp))
resp = rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "grp-a-1", "created_at": "2026-01-01"}})
check("valid delete succeeds", "result" in resp and "error" not in resp, err_msg(resp))

print(f"\n=== GROUP A: {passed} passed, {failed} failed ===")
sys.exit(1 if failed else 0)
