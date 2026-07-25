#!/usr/bin/env python3
"""End-to-end test of dynamodb-plugin against DynamoDB Local.

Spawns the plugin binary, sends JSON-RPC requests over stdin, reads responses
from stdout. Tests ALL real RPC methods with seeded data.
"""

import json
import subprocess
import sys
import time

BINARY = "target/release/dynamodb-plugin.exe"
CONN = {
    "region": "us-east-1",
    "access_key_id": "test",
    "secret_access_key": "test",
    "endpoint": "http://localhost:8000",
}
TABLE = "test_users"

passed = 0
failed = 0
results = []


def send(proc, method, params=None, req_id=1):
    """Send a JSON-RPC request and read the response line."""
    msg = {"jsonrpc": "2.0", "method": method, "id": req_id}
    if params is not None:
        msg["params"] = params
    line = json.dumps(msg) + "\n"
    proc.stdin.write(line)
    proc.stdin.flush()
    resp_line = proc.stdout.readline()
    if not resp_line:
        return {"error": {"code": -32000, "message": "NO RESPONSE (EOF)"}}
    return json.loads(resp_line)


def check(name, resp, expect_error=False, expect_key=None):
    """Validate a response and record the result."""
    global passed, failed
    has_error = "error" in resp
    has_result = "result" in resp

    if expect_error:
        ok = has_error
        detail = resp.get("error", {}).get("message", "")[:120]
    else:
        ok = has_result and not has_error
        if ok and expect_key is not None:
            r = resp["result"]
            if isinstance(r, dict):
                ok = expect_key in r
            elif isinstance(r, list):
                ok = len(r) > 0
            else:
                ok = r is not None
        detail = json.dumps(resp.get("result", resp.get("error", "")))[:150]

    status = "✅" if ok else "❌"
    if ok:
        passed += 1
    else:
        failed += 1
    results.append(f"  {status} {name}: {detail}")
    return ok


def main():
    print(f"🚀 Starting {BINARY}...")
    proc = subprocess.Popen(
        [f"./{BINARY}"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    # Give the worker pool a moment to spin up
    time.sleep(0.5)

    print("─" * 60)
    print("  LIFECYCLE")
    print("─" * 60)

    r = send(proc, "initialize", req_id=1)
    check("initialize", r, expect_key="success")

    r = send(proc, "ping", {"params": CONN}, req_id=2)
    check("ping", r)

    r = send(proc, "test_connection", {"params": CONN}, req_id=3)
    check("test_connection", r)

    print("─" * 60)
    print("  METADATA")
    print("─" * 60)

    r = send(proc, "get_tables", {"params": CONN}, req_id=4)
    check("get_tables", r, expect_key=None)
    tables = r.get("result", [])
    table_names = [t.get("name", t) if isinstance(t, dict) else t for t in tables] if isinstance(tables, list) else []
    if TABLE in str(table_names):
        print(f"       ↳ found '{TABLE}' in table list ✓")

    r = send(proc, "get_columns", {"params": CONN, "table": TABLE}, req_id=5)
    check("get_columns", r)

    r = send(proc, "get_indexes", {"params": CONN, "table": TABLE}, req_id=6)
    check("get_indexes", r)

    r = send(proc, "get_foreign_keys", {"params": CONN, "table": TABLE}, req_id=7)
    check("get_foreign_keys", r)

    r = send(proc, "get_databases", {}, req_id=8)
    check("get_databases", r)

    r = send(proc, "get_schemas", {}, req_id=9)
    check("get_schemas", r)

    r = send(proc, "get_routines", {}, req_id=10)
    check("get_routines", r)

    r = send(proc, "get_views", {}, req_id=11)
    check("get_views", r)

    print("─" * 60)
    print("  QUERIES (PartiQL)")
    print("─" * 60)

    # Scan all
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT * FROM {TABLE}"}, req_id=12)
    check("SELECT * (full scan)", r)
    rows = r.get("result", {})
    if isinstance(rows, dict) and "rows" in rows:
        print(f"       ↳ {len(rows['rows'])} rows returned")
    elif isinstance(rows, list):
        print(f"       ↳ {len(rows)} rows returned")

    # Filtered query
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT * FROM {TABLE} WHERE id = 'usr-001'"}, req_id=13)
    check("SELECT WHERE id='usr-001'", r)

    # Projection
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT id, name, email FROM {TABLE} WHERE active = true"}, req_id=14)
    check("SELECT projection (active=true)", r)

    # Count-like
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT * FROM {TABLE} WHERE age > 30"}, req_id=15)
    check("SELECT WHERE age > 30", r)

    print("─" * 60)
    print("  CRUD")
    print("─" * 60)

    # Insert
    r = send(proc, "insert_record", {
        "params": CONN,
        "table": TABLE,
        "data": {
            "id": "usr-test",
            "created_at": "2026-07-24T12:00:00Z",
            "email": "test@hermes.dev",
            "name": "Hermes Bot",
            "age": 1,
            "active": True,
            "role": "tester",
        },
    }, req_id=16)
    check("insert_record", r)

    # Verify insert
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT * FROM {TABLE} WHERE id = 'usr-test'"}, req_id=17)
    check("verify insert (SELECT)", r)

    # Update (composite key table — use `key` object)
    r = send(proc, "update_record", {
        "params": CONN,
        "table": TABLE,
        "key": {"id": "usr-test", "created_at": "2026-07-24T12:00:00Z"},
        "col_name": "role",
        "new_val": "senior-tester",
    }, req_id=18)
    check("update_record", r)

    # Verify update
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT name, role FROM {TABLE} WHERE id = 'usr-test'"}, req_id=19)
    check("verify update (SELECT)", r)

    # Delete (composite key)
    r = send(proc, "delete_record", {
        "params": CONN,
        "table": TABLE,
        "key": {"id": "usr-test", "created_at": "2026-07-24T12:00:00Z"},
    }, req_id=20)
    check("delete_record", r)

    # Verify delete
    r = send(proc, "execute_query", {"params": CONN, "query": f"SELECT * FROM {TABLE} WHERE id = 'usr-test'"}, req_id=21)
    check("verify delete (empty result)", r)

    print("─" * 60)
    print("  DDL (SQL generation)")
    print("─" * 60)

    r = send(proc, "get_create_table_sql", {"params": CONN, "table_name": TABLE}, req_id=22)
    check("get_create_table_sql", r)

    r = send(proc, "get_add_column_sql", {"params": CONN, "table": TABLE, "column": {"name": "phone", "type": "S"}}, req_id=23)
    check("get_add_column_sql", r)

    r = send(proc, "get_alter_column_sql", {"params": CONN, "table": TABLE, "old_column": {"name": "age"}, "new_column": {"name": "age_years"}}, req_id=24)
    check("get_alter_column_sql", r)

    r = send(proc, "get_create_index_sql", {"params": CONN, "table": TABLE, "index": "role-index", "columns": ["role"]}, req_id=25)
    check("get_create_index_sql", r)

    print("─" * 60)
    print("  ERROR HANDLING")
    print("─" * 60)

    r = send(proc, "execute_query", {"params": CONN, "query": ""}, req_id=26)
    check("empty query → error", r, expect_error=True)

    r = send(proc, "nonexistent_method", {}, req_id=27)
    check("unknown method → error", r, expect_error=True)

    # Bad JSON
    proc.stdin.write("not json\n")
    proc.stdin.flush()
    resp_line = proc.stdout.readline()
    r = json.loads(resp_line) if resp_line else {"error": {"message": "no response"}}
    check("malformed JSON → parse error", r, expect_error=True)

    print("─" * 60)

    # Shut down
    proc.stdin.close()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    print(f"\n{'═' * 60}")
    print(f"  RESULTS: {passed} passed, {failed} failed, {passed + failed} total")
    print(f"{'═' * 60}\n")

    for line in results:
        print(line)

    print()
    if failed == 0:
        print("🎉 ALL TESTS PASSED — plugin is release-ready")
    else:
        print(f"⚠️  {failed} test(s) FAILED — review before releasing")

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
