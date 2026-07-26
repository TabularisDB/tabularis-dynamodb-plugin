#!/usr/bin/env python3
"""Live JSON-RPC test for #8: ConsumedCapacity reporting + destructive guard."""
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


# Ensure a row exists to read.
rpc("insert_record", {**PARAMS, "table": TABLE, "data": {
    "id": "cap-1", "created_at": "2026-02-01", "name": "Cap", "age": 7,
}})

print("== #8: consumed_capacity reported on native reads ==")
# Scan
r_scan = rpc("execute_query", {**PARAMS, "query": f"#!scan\nTableName: {TABLE}"})
cc = r_scan.get("result", {}).get("consumed_capacity")
check("scan reports consumed_capacity", isinstance(cc, (int, float)) and cc > 0, f"got {cc}")

# Query
r_query = rpc("execute_query", {**PARAMS, "query":
    f"#!query\nTableName: {TABLE}\nPartitionKey: id\nPartitionValue: cap-1"})
cc2 = r_query.get("result", {}).get("consumed_capacity")
check("query reports consumed_capacity", isinstance(cc2, (int, float)) and cc2 > 0, f"got {cc2}")

# Get
r_get = rpc("execute_query", {**PARAMS, "query":
    f"#!get\nTableName: {TABLE}\nKey:\n  id: cap-1\n  created_at: 2026-02-01"})
cc3 = r_get.get("result", {}).get("consumed_capacity")
check("get reports consumed_capacity", isinstance(cc3, (int, float)) and cc3 > 0, f"got {cc3}")

# PartiQL SELECT — DynamoDB Local does NOT return ConsumedCapacity for
# ExecuteStatement (a local-emulator limitation; production DynamoDB does).
# We accept either a positive number or None here; the code requests it either way.
r_sel = rpc("execute_query", {**PARAMS, "query": f'SELECT name FROM "{TABLE}" WHERE id = \'cap-1\''})
cc4 = r_sel.get("result", {}).get("consumed_capacity")
check("partiql SELECT capacity (local may be None)", cc4 is None or (isinstance(cc4, (int, float)) and cc4 > 0), f"got {cc4}")

print("== #8: scan limit + pagination path ==")
# Insert a second row so a Limit:1 scan pages.
rpc("insert_record", {**PARAMS, "table": TABLE, "data": {
    "id": "cap-2", "created_at": "2026-02-01", "name": "Cap2", "age": 8,
}})
r_scan_limited = rpc("execute_query", {**PARAMS, "query": f"#!scan\nTableName: {TABLE}\nLimit: 1"})
res = r_scan_limited.get("result", {})
check("scan Limit:1 returns exactly 1 row", len(res.get("rows", [])) == 1, f"got {len(res.get('rows', []))}")
check("scan Limit:1 signals has_more/pagination", res.get("has_more") is True or res.get("pagination") is not None,
      f"has_more={res.get('has_more')} pagination={res.get('pagination')}")

print("== #8: destructive guard ==")
# DROP TABLE must be blocked without confirmation.
r_drop = rpc("execute_query", {**PARAMS, "query": f'DROP TABLE "{TABLE}"'})
warn = r_drop.get("result", {}).get("warning")
check("DROP TABLE blocked with warning", warn is not None and "confirm" in warn.lower(),
      f"got: {json.dumps(r_drop)[:200]}")
# Verify table still exists (guard actually prevented execution).
r_tbls = rpc("get_tables", PARAMS)
names = [t.get("name") for t in r_tbls.get("result", [])] if isinstance(r_tbls.get("result"), list) else []
check("table survived blocked DROP", TABLE in names, f"tables={names}")

# WHERE-less DELETE must be blocked.
r_del = rpc("execute_query", {**PARAMS, "query": f'DELETE FROM "{TABLE}"'})
warn2 = r_del.get("result", {}).get("warning")
check("WHERE-less DELETE blocked with warning", warn2 is not None and "confirm" in warn2.lower(),
      f"got: {json.dumps(r_del)[:200]}")

# Guarded DELETE (with WHERE) is allowed and executes (removes our test row).
r_gdel = rpc("execute_query", {**PARAMS, "query": f'DELETE FROM "{TABLE}" WHERE id = \'cap-1\' AND created_at = \'2026-02-01\''})
check("guarded DELETE allowed (no warning)", r_gdel.get("result", {}).get("warning") is None,
      f"got: {json.dumps(r_gdel)[:200]}")

# cleanup any leftover
for cid in ["cap-1", "cap-2"]:
    rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": cid, "created_at": "2026-02-01"}})

print(f"\n{'='*40}\n  {passed} passed, {failed} failed")
raise SystemExit(1 if failed else 0)
