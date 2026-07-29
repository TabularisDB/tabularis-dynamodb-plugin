#!/usr/bin/env python3
"""Live JSON-RPC test for Group B query fixes (#20 LIMIT, #26 scan/query/get, #27 pagination)."""
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
    proc = subprocess.run([PLUGIN], input=json.dumps(req) + "\n",
                          capture_output=True, text=True, timeout=30)
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


# Seed 5 rows with same partition key for query/pagination tests.
print("== seed 5 rows ==")
for i in range(5):
    rpc("insert_record", {**PARAMS, "table": TABLE, "data": {
        "id": "grp-b", "created_at": f"2026-02-0{i}", "idx": i}})

print("== #20: PartiQL LIMIT stripped and applied client-side ==")
resp = rpc("execute_query", {**PARAMS, "query": f"SELECT * FROM \"{TABLE}\" WHERE id = 'grp-b' LIMIT 2"})
rows = resp.get("result", {}).get("rows", [])
check("LIMIT 2 returns exactly 2 rows (no Unsupported clause error)",
      "result" in resp and len(rows) == 2, err_msg(resp) or f"got {len(rows)}")
check("truncated flag set when limit applied", resp.get("result", {}).get("truncated") is True, str(resp.get("result", {})))

print("== #26: native #!scan ==")
resp = rpc("execute_query", {**PARAMS, "query": f"#!scan\nTableName: {TABLE}\nLimit: 3"})
check("#!scan returns rows", "result" in resp and len(resp["result"]["rows"]) > 0, err_msg(resp))
scan_token = resp.get("result", {}).get("pagination", {}).get("next_token") if resp.get("result") else None
check("#!scan returns a pagination token when Limit<total", scan_token is not None, str(resp.get("result", {}).get("pagination")))

print("== #26: native #!query ==")
resp = rpc("execute_query", {**PARAMS, "query": f"#!query\nTableName: {TABLE}\nPartitionKey: id\nPartitionValue: grp-b"})
qrows = resp.get("result", {}).get("rows", [])
check("#!query returns the 5 grp-b rows", "result" in resp and len(qrows) == 5, err_msg(resp) or f"got {len(qrows)}")

print("== #27: pagination — feed scan token back ==")
if scan_token:
    resp2 = rpc("execute_query", {**PARAMS, "query": f"#!scan\nTableName: {TABLE}\nLimit: 3", "next_token": scan_token})
    check("resuming scan with next_token succeeds", "result" in resp2, err_msg(resp2))
else:
    check("resuming scan with next_token succeeds", False, "no token from first scan")

print("== #26: native #!get ==")
resp = rpc("execute_query", {**PARAMS, "query": f"#!get\nTableName: {TABLE}\nKey:\n  id: grp-b\n  created_at: 2026-02-00"})
grows = resp.get("result", {}).get("rows", [])
check("#!get returns the single item", "result" in resp and len(grows) == 1, err_msg(resp) or f"got {len(grows)}")

# Cleanup seed rows
for i in range(5):
    rpc("delete_record", {**PARAMS, "table": TABLE, "key": {"id": "grp-b", "created_at": f"2026-02-0{i}"}})

print(f"\n=== GROUP B: {passed} passed, {failed} failed ===")
sys.exit(1 if failed else 0)
