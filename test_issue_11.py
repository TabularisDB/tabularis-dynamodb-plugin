#!/usr/bin/env python3
"""Live JSON-RPC test for #11: condition expressions / idempotent inserts."""
import json
import subprocess

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
        [PLUGIN],
        input=json.dumps(req) + "\n",
        capture_output=True,
        text=True,
        timeout=30,
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


def err_msg(resp):
    return resp.get("error", {}).get("message", "")


# Clean any prior run state.
for ts in ["2026-01-01", "2026-01-02"]:
    rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "idem-1", "created_at": ts}})
rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "idem-2", "created_at": "2026-01-01"}})

print("== #11: idempotent insert (default attribute_not_exists(pk)) ==")
r1 = rpc("insert_record", {**PARAMS, "table": TABLE, "idempotent": True, "data": {
    "id": "idem-1", "created_at": "2026-01-01", "name": "Alice", "age": 30,
}})
check("first idempotent insert succeeds", "result" in r1 and "error" not in r1, err_msg(r1))

# Same composite key again with idempotent=true must be rejected.
r2 = rpc("insert_record", {**PARAMS, "table": TABLE, "idempotent": True, "data": {
    "id": "idem-1", "created_at": "2026-01-01", "name": "Bob", "age": 99,
}})
check("duplicate idempotent insert is rejected", "error" in r2, f"got: {json.dumps(r2)[:200]}")
check("rejection mentions condition", "condition" in err_msg(r2).lower(), err_msg(r2))

print("== #11: explicit condition_expression ==")
r3 = rpc("insert_record", {**PARAMS, "table": TABLE,
                           "condition_expression": "attribute_not_exists(id)", "data": {
                               "id": "idem-2", "created_at": "2026-01-01", "name": "Carol", "age": 25,
                           }})
check("explicit condition insert succeeds", "result" in r3 and "error" not in r3, err_msg(r3))

r4 = rpc("insert_record", {**PARAMS, "table": TABLE,
                           "condition_expression": "attribute_not_exists(id)", "data": {
                               "id": "idem-2", "created_at": "2026-01-01", "name": "Dup", "age": 1,
                           }})
check("unsatisfiable explicit condition rejected", "error" in r4, f"got: {json.dumps(r4)[:200]}")

print("== #11: non-idempotent insert still overwrites (backward compat) ==")
r5 = rpc("insert_record", {**PARAMS, "table": TABLE, "data": {
    "id": "idem-1", "created_at": "2026-01-01", "name": "Overwritten", "age": 100,
}})
check("plain insert overwrites (no condition)", "result" in r5 and "error" not in r5, err_msg(r5))

# Readback via PartiQL SELECT (param key is `query`, table quoted).
r6 = rpc("execute_query", {**PARAMS,
                           "query": f'SELECT name FROM "{TABLE}" WHERE id = \'idem-1\''})
rows = r6.get("result", {}).get("rows", [])
cols = r6.get("result", {}).get("columns", [])
row = dict(zip(cols, rows[0])) if rows else {}
check("overwrite reflected in read", row.get("name") == "Overwritten", f"cols={cols} rows={rows}")

# cleanup
for ts in ["2026-01-01", "2026-01-02"]:
    rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "idem-1", "created_at": ts}})
rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "idem-2", "created_at": "2026-01-01"}})

print(f"\n{'='*40}\n  {passed} passed, {failed} failed")
raise SystemExit(1 if failed else 0)
