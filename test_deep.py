#!/usr/bin/env python3
"""Deep stress test of dynamodb-plugin — edge cases, type handling, protocol abuse.

Covers: nested types, unicode, special chars, sparse data, query mode prefixes,
pagination, error paths, concurrent requests, empty/null handling, DDL edge cases.
"""
import json
import subprocess
import sys
import time
import threading

BINARY = "target/release/dynamodb-plugin.exe"
CONN = {
    "region": "us-east-1",
    "access_key_id": "test",
    "secret_access_key": "test",
    "endpoint": "http://localhost:8000",
}
TABLE = "test_users"

passed = []
failed = []
bugs = []


def send(proc, method, params, req_id=1, raw=None, timeout=10):
    """Send a JSON-RPC request and read the response."""
    if raw is not None:
        msg = raw
    else:
        msg = json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": req_id})
    proc.stdin.write(msg + "\n")
    proc.stdin.flush()
    proc.stdout.flush()
    line = proc.stdout.readline()
    if not line:
        return {"error": {"code": -32000, "message": "no response (EOF)"}}
    return json.loads(line)


def check(name, resp, expect_ok=True, expect_error_contains=None):
    global passed, failed
    ok = "result" in resp and "error" not in resp
    err = "error" in resp

    if expect_ok and ok:
        passed.append((name, resp.get("result")))
        print(f"  ✅ {name}")
    elif not expect_ok and err:
        msg = resp["error"].get("message", "")
        if expect_error_contains and expect_error_contains.lower() not in msg.lower():
            failed.append((name, f"expected error containing '{expect_error_contains}', got: {msg}"))
            print(f"  ❌ {name}: wrong error: {msg}")
        else:
            passed.append((name, resp.get("error")))
            print(f"  ✅ {name} (error as expected: {msg[:80]})")
    elif expect_ok and err:
        failed.append((name, resp["error"]))
        print(f"  ❌ {name}: {resp['error'].get('message', 'unknown error')}")
    else:
        failed.append((name, "unexpected success"))
        print(f"  ❌ {name}: expected error but got success")


def note_bug(name, detail):
    bugs.append((name, detail))
    print(f"  🐛 BUG: {name}")
    print(f"       {detail}")


def ddb_call(target, payload_dict):
    """Direct DynamoDB Local API call for seeding/verification."""
    import urllib.request
    payload = json.dumps(payload_dict).encode()
    req = urllib.request.Request(
        "http://localhost:8000", data=payload,
        headers={
            "Content-Type": "application/x-amz-json-1.0",
            "X-Amz-Target": f"DynamoDB_20120810.{target}",
            "Authorization": "AWS4-HMAC-SHA256 Credential=test/20260724/us-east-1/dynamodb/aws4_request",
        },
    )
    return json.loads(urllib.request.urlopen(req).read())


def seed_edge_case_table():
    """Create a table with a simple key for edge case tests."""
    try:
        ddb_call("CreateTable", {
            "TableName": "edge_cases",
            "KeySchema": [{"AttributeName": "pk", "KeyType": "HASH"}],
            "AttributeDefinitions": [{"AttributeName": "pk", "AttributeType": "S"}],
            "BillingMode": "PAY_PER_REQUEST",
        })
    except Exception:
        pass  # already exists


def main():
    seed_edge_case_table()

    proc = subprocess.Popen(
        [f"./{BINARY}"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1,
    )
    time.sleep(0.5)

    # Initialize
    send(proc, "initialize", {"params": CONN}, req_id=0)

    print("=" * 70)
    print("  SECTION 1: TYPE HANDLING")
    print("=" * 70)

    # 1a. Insert with nested object value
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-nested", "created_at": "2026-01-01T00:00:00Z",
            "profile": {"bio": "hello", "level": 5},
            "name": "Nested User",
        },
    }, req_id=1)
    check("insert with nested object", r)

    # 1b. Insert with array value
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-array", "created_at": "2026-01-01T00:00:00Z",
            "tags": ["admin", "dev", "ops"],
            "name": "Array User",
        },
    }, req_id=2)
    check("insert with array value", r)

    # 1c. Insert with boolean values
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-bool", "created_at": "2026-01-01T00:00:00Z",
            "active": True, "verified": False,
            "name": "Bool User",
        },
    }, req_id=3)
    check("insert with boolean values", r)

    # 1d. Insert with null value
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-null", "created_at": "2026-01-01T00:00:00Z",
            "middle_name": None,
            "name": "Null User",
        },
    }, req_id=4)
    check("insert with null value", r)

    # 1e. Insert with unicode string
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-unicode", "created_at": "2026-01-01T00:00:00Z",
            "name": "日本語ユーザー 🚀", "city": "São Paulo",
        },
    }, req_id=5)
    check("insert with unicode string", r)

    # 1f. Insert with single quotes in string (SQL escaping)
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-quotes", "created_at": "2026-01-01T00:00:00Z",
            "name": "O'Brien's \"test\" value",
        },
    }, req_id=6)
    check("insert with quotes in string", r)

    # 1g. Insert with negative and zero numbers
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-numbers", "created_at": "2026-01-01T00:00:00Z",
            "score": -42, "balance": 0, "rate": 3.14159,
            "name": "Number User",
        },
    }, req_id=7)
    check("insert with negative/zero/float numbers", r)

    # 1h. Insert with empty string value
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-empty", "created_at": "2026-01-01T00:00:00Z",
            "nickname": "",
            "name": "Empty String User",
        },
    }, req_id=8)
    check("insert with empty string value", r)

    # 1i. Insert with very long string (10KB)
    long_str = "x" * 10240
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-long", "created_at": "2026-01-01T00:00:00Z",
            "payload": long_str,
            "name": "Long String User",
        },
    }, req_id=9)
    check("insert with 10KB string value", r)

    # 1j. Insert with deeply nested structure
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
        "data": {
            "id": "usr-deep", "created_at": "2026-01-01T00:00:00Z",
            "meta": {"a": {"b": {"c": {"d": [1, 2, {"e": "deep"}]}}}},
            "name": "Deep Nest User",
        },
    }, req_id=10)
    check("insert with deeply nested structure", r)

    # Verify the nested data round-trips correctly
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"SELECT * FROM {TABLE} WHERE id = 'usr-nested'",
    }, req_id=11)
    check("query nested object back", r)
    if "result" in r:
        rows = r["result"].get("rows", [])
        if rows:
            row = rows[0]
            cols = r["result"].get("columns", [])
            if "profile" in cols:
                idx = cols.index("profile")
                profile_val = row[idx]
                if isinstance(profile_val, dict) and profile_val.get("bio") == "hello":
                    passed.append(("nested object round-trip", profile_val))
                    print("  ✅ nested object round-trip verified")
                else:
                    note_bug("nested object round-trip", f"profile came back as: {profile_val}")
            else:
                note_bug("nested object round-trip", f"'profile' column missing from result. Columns: {cols}")

    print()
    print("=" * 70)
    print("  SECTION 2: QUERY MODE PREFIXES")
    print("=" * 70)

    # 2a. #!partiql prefix
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"#!partiql\nSELECT name FROM {TABLE} WHERE id = 'usr-001'",
    }, req_id=20)
    check("#!partiql prefix query", r)

    # 2b. #!scan prefix (should work as passthrough PartiQL)
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"#!scan\nSELECT name FROM {TABLE} WHERE id = 'usr-001'",
    }, req_id=21)
    check("#!scan prefix query", r)

    # 2c. #!query prefix
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"#!query\nSELECT name FROM {TABLE} WHERE id = 'usr-001'",
    }, req_id=22)
    check("#!query prefix query", r)

    # 2d. #!get prefix
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"#!get\nSELECT name FROM {TABLE} WHERE id = 'usr-001'",
    }, req_id=23)
    check("#!get prefix query", r)

    # 2e. #!scan with YAML-like body (NOT valid PartiQL — should fail gracefully)
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"#!scan\ntable: {TABLE}\nlimit: 5\nfilter: active = true",
    }, req_id=24)
    # This SHOULD fail because the body isn't valid PartiQL
    if "error" in r:
        passed.append(("#!scan YAML body fails gracefully", r["error"]))
        print(f"  ✅ #!scan YAML body fails gracefully: {r['error'].get('message', '')[:60]}")
        note_bug("#!scan mode not implemented", "YAML-like scan/query/get bodies are passed through as PartiQL and fail. The QueryMode::Scan/Query/Get arms just call execute_statement() with the raw body — no YAML parsing or SDK Scan/Query/GetItem calls.")
    else:
        note_bug("#!scan YAML body unexpected success", f"Got result: {str(r.get('result', ''))[:100]}")

    print()
    print("=" * 70)
    print("  SECTION 3: SPARSE DATA / HETEROGENEOUS ITEMS")
    print("=" * 70)

    # 3a. Query items with different schemas (sparse data)
    # Insert an item with only 2 attributes
    r = send(proc, "insert_record", {
        "params": CONN, "table": "edge_cases",
        "data": {"pk": "sparse-1", "only_field": "hello"},
    }, req_id=30)
    check("insert sparse item 1", r)

    r = send(proc, "insert_record", {
        "params": CONN, "table": "edge_cases",
        "data": {"pk": "sparse-2", "other_field": 42, "extra": True},
    }, req_id=31)
    check("insert sparse item 2", r)

    r = send(proc, "execute_query", {
        "params": CONN,
        "query": "SELECT * FROM edge_cases",
    }, req_id=32)
    check("query sparse items", r)
    if "result" in r:
        cols = r["result"].get("columns", [])
        rows = r["result"].get("rows", [])
        if len(rows) >= 2:
            # Check if all attributes from ALL items appear in columns
            all_keys = set()
            for row in rows:
                for i, v in enumerate(row):
                    if v is not None and i < len(cols):
                        all_keys.add(cols[i])
            # The columns are derived from the FIRST item only
            if len(cols) < 3:  # sparse-1 has pk+only_field, sparse-2 has pk+other_field+extra
                note_bug(
                    "sparse data column loss",
                    f"Columns derived from first item only: {cols}. "
                    f"Attributes from other items (e.g. 'extra', 'other_field') are silently dropped. "
                    f"Expected union of all keys across all items."
                )
            else:
                print(f"  ✅ sparse data columns: {cols}")

    print()
    print("=" * 70)
    print("  SECTION 4: ERROR PATHS & EDGE CASES")
    print("=" * 70)

    # 4a. Query non-existent table
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": "SELECT * FROM nonexistent_table_xyz",
    }, req_id=40)
    check("query non-existent table", r, expect_ok=False)

    # 4b. get_columns on non-existent table
    r = send(proc, "get_columns", {"params": CONN, "table": "nonexistent_table_xyz"}, req_id=41)
    check("get_columns non-existent table", r, expect_ok=False)

    # 4c. get_indexes on non-existent table
    r = send(proc, "get_indexes", {"params": CONN, "table": "nonexistent_table_xyz"}, req_id=42)
    check("get_indexes non-existent table", r, expect_ok=False)

    # 4d. insert_record with empty data object
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE, "data": {},
    }, req_id=43)
    if "error" in r:
        passed.append(("insert empty data → error", r["error"]))
        print(f"  ✅ insert empty data → error: {r['error'].get('message', '')[:60]}")
    else:
        note_bug("insert empty data accepted", f"Empty data object {{}} was accepted and sent to DynamoDB. Result: {r.get('result')}")

    # 4e. insert_record with missing data field entirely
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE,
    }, req_id=44)
    check("insert missing data field", r, expect_ok=False)

    # 4f. insert_record with data as non-object (string)
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE, "data": "not-an-object",
    }, req_id=45)
    check("insert data as string", r, expect_ok=False)

    # 4g. insert_record with data as array
    r = send(proc, "insert_record", {
        "params": CONN, "table": TABLE, "data": [1, 2, 3],
    }, req_id=46)
    check("insert data as array", r, expect_ok=False)

    # 4h. update_record with key containing non-key columns
    r = send(proc, "update_record", {
        "params": CONN, "table": TABLE,
        "key": {"id": "usr-001", "created_at": "2026-01-15T10:30:00Z", "name": "Alice"},
        "col_name": "role", "new_val": "hacker",
    }, req_id=47)
    # DynamoDB will reject this because "name" is not a key attribute
    if "error" in r:
        passed.append(("update with non-key columns in key", r["error"]))
        print(f"  ✅ update with non-key cols in key → error: {r['error'].get('message', '')[:60]}")
        note_bug(
            "no validation of key columns",
            "update_record/delete_record accept any columns in the 'key' object without "
            "validating they're actual key attributes. Non-key columns in WHERE cause "
            "DynamoDB service errors with unhelpful messages."
        )
    else:
        note_bug("update with non-key cols succeeded unexpectedly", str(r.get("result", ""))[:100])

    # 4i. Invalid PartiQL syntax
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": "SELEC * FORM test_users WHER id = 'x'",
    }, req_id=48)
    check("invalid PartiQL syntax", r, expect_ok=False)

    # 4j. test_connection with bad endpoint
    r = send(proc, "test_connection", {
        "params": {
            "region": "us-east-1",
            "access_key_id": "test",
            "secret_access_key": "test",
            "endpoint": "http://localhost:19999",
        },
    }, req_id=49)
    check("test_connection bad endpoint", r, expect_ok=False)

    # 4k. test_connection with no params at all
    r = send(proc, "test_connection", {"params": {}}, req_id=50)
    check("test_connection empty params", r, expect_ok=False)

    # 4l. execute_query with only whitespace
    r = send(proc, "execute_query", {"params": CONN, "query": "   "}, req_id=51)
    check("query whitespace-only", r, expect_ok=False)

    # 4m. execute_query with null query
    r = send(proc, "execute_query", {"params": CONN, "query": None}, req_id=52)
    check("query null value", r, expect_ok=False)

    print()
    print("=" * 70)
    print("  SECTION 5: DDL EDGE CASES")
    print("=" * 70)

    # 5a. get_create_table_sql with empty table name
    r = send(proc, "get_create_table_sql", {"params": CONN, "table_name": ""}, req_id=60)
    check("create_table_sql empty name", r, expect_ok=False)

    # 5b. get_create_table_sql with missing table_name
    r = send(proc, "get_create_table_sql", {"params": CONN}, req_id=61)
    check("create_table_sql missing name", r, expect_ok=False)

    # 5c. get_add_column_sql with missing column
    r = send(proc, "get_add_column_sql", {"params": CONN, "table": TABLE}, req_id=62)
    check("add_column_sql missing column", r, expect_ok=False)

    # 5d. get_alter_column_sql with missing old_column
    r = send(proc, "get_alter_column_sql", {"params": CONN, "table": TABLE}, req_id=63)
    check("alter_column_sql missing old_column", r, expect_ok=False)

    # 5e. get_create_table_sql with special chars in table name
    r = send(proc, "get_create_table_sql", {"params": CONN, "table_name": "my-table (v2)"}, req_id=64)
    check("create_table_sql special chars", r)
    if "result" in r:
        sql = r["result"]
        if isinstance(sql, list) and sql:
            print(f"       ↳ SQL: {sql[0]}")

    print()
    print("=" * 70)
    print("  SECTION 6: PROTOCOL ABUSE")
    print("=" * 70)

    # 6a. Missing jsonrpc field
    r = send(proc, "", {}, req_id=70, raw=json.dumps({"method": "ping", "params": {"params": CONN}, "id": 70}) + "\n")
    if "result" in r or "error" in r:
        passed.append(("missing jsonrpc field", r))
        print(f"  ✅ missing jsonrpc field handled: {'result' if 'result' in r else 'error'}")
    else:
        note_bug("missing jsonrpc field", f"No valid response: {str(r)[:100]}")

    # 6b. Missing method field
    r = send(proc, "", {}, req_id=71, raw=json.dumps({"jsonrpc": "2.0", "params": {}, "id": 71}) + "\n")
    if "error" in r:
        passed.append(("missing method field", r["error"]))
        print(f"  ✅ missing method → error: {r['error'].get('message', '')[:60]}")
    else:
        note_bug("missing method field accepted", f"Got: {str(r)[:100]}")

    # 6c. Missing id field
    r = send(proc, "", {}, req_id=72, raw=json.dumps({"jsonrpc": "2.0", "method": "ping", "params": {"params": CONN}}) + "\n")
    # JSON-RPC says missing id = notification, no response expected
    # But the plugin might still respond
    if r:
        passed.append(("missing id field", r))
        print(f"  ✅ missing id field: got response (notification handling)")
    else:
        print("  ✅ missing id field: no response (correct notification behavior)")

    # 6d. Very large request ID
    r = send(proc, "ping", {"params": CONN}, req_id=999999999999999)
    check("very large request ID", r)

    # 6e. String request ID
    r = send(proc, "ping", {"params": CONN}, req_id=74, raw=json.dumps({"jsonrpc": "2.0", "method": "ping", "params": {"params": CONN}, "id": "string-id-123"}) + "\n")
    if "result" in r:
        if r.get("id") == "string-id-123":
            passed.append(("string request ID echoed", r))
            print("  ✅ string request ID echoed correctly")
        else:
            note_bug("string request ID not echoed", f"Sent id='string-id-123', got id={r.get('id')}")
    else:
        note_bug("string request ID failed", str(r)[:100])

    # 6f. Null request ID
    r = send(proc, "ping", {"params": CONN}, req_id=75, raw=json.dumps({"jsonrpc": "2.0", "method": "ping", "params": {"params": CONN}, "id": None}) + "\n")
    if "result" in r or "error" in r:
        passed.append(("null request ID", r))
        print(f"  ✅ null request ID handled")
    else:
        note_bug("null request ID", f"No valid response: {str(r)[:100]}")

    # 6g. Extra unknown fields in request
    r = send(proc, "ping", {"params": CONN}, req_id=76, raw=json.dumps({"jsonrpc": "2.0", "method": "ping", "params": {"params": CONN}, "id": 76, "extra": "field", "hack": True}) + "\n")
    check("extra unknown fields", r)

    # 6h. Empty JSON object
    r = send(proc, "", {}, req_id=77, raw="{}\n")
    if "error" in r:
        passed.append(("empty JSON object", r["error"]))
        print(f"  ✅ empty JSON object → error: {r['error'].get('message', '')[:60]}")
    else:
        note_bug("empty JSON object accepted", str(r)[:100])

    # 6i. JSON array instead of object
    r = send(proc, "", {}, req_id=78, raw='[1, 2, 3]\n')
    if "error" in r:
        passed.append(("JSON array input", r["error"]))
        print(f"  ✅ JSON array → error: {r['error'].get('message', '')[:60]}")
    else:
        note_bug("JSON array accepted", str(r)[:100])

    print()
    print("=" * 70)
    print("  SECTION 7: PAGINATION & LIMIT")
    print("=" * 70)

    # 7a. Query with LIMIT
    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"SELECT * FROM {TABLE} LIMIT 2",
    }, req_id=80)
    check("query with LIMIT 2", r)
    if "result" in r:
        rows = r["result"].get("rows", [])
        has_more = r["result"].get("has_more", False)
        pagination = r["result"].get("pagination")
        print(f"       ↳ {len(rows)} rows, has_more={has_more}, pagination={pagination}")
        if len(rows) == 2 and has_more:
            passed.append(("LIMIT 2 returns 2 rows with has_more", rows))
            print("  ✅ LIMIT 2 returns 2 rows with has_more=True")
        elif len(rows) <= 2:
            print(f"  ⚠️  LIMIT 2 returned {len(rows)} rows (DynamoDB PartiQL LIMIT behavior)")

    # 7b. Check if pagination token can be used for next page
    if "result" in r and r["result"].get("pagination"):
        token = r["result"]["pagination"].get("next_token")
        if token:
            # There's no documented way to pass next_token back — this is a gap
            note_bug(
                "no pagination input support",
                "execute_query returns pagination.next_token but there's no way to pass it "
                "back in a subsequent request. The extractor has extract_limit/extract_page "
                "but they're never used in execute_query. Clients can't paginate."
            )

    print()
    print("=" * 70)
    print("  SECTION 8: METADATA CORRECTNESS")
    print("=" * 70)

    # 8a. get_columns — check is_nullable correctness
    r = send(proc, "get_columns", {"params": CONN, "table": TABLE}, req_id=90)
    check("get_columns", r)
    if "result" in r:
        cols = r["result"]
        # In DynamoDB, non-key attributes are ALWAYS nullable
        non_nullable_non_pk = [c["name"] for c in cols if not c.get("is_pk") and not c.get("is_nullable")]
        if non_nullable_non_pk:
            note_bug(
                "is_nullable always false",
                f"Non-key columns marked is_nullable=false: {non_nullable_non_pk}. "
                "In DynamoDB, all non-key attributes are optional/nullable. "
                "Only key attributes should have is_nullable=false."
            )
        else:
            print("  ✅ is_nullable correctness looks OK")

    # 8b. get_databases — hardcoded response
    r = send(proc, "get_databases", {"params": CONN}, req_id=91)
    check("get_databases", r)
    if "result" in r:
        dbs = r["result"]
        if dbs == [{"name": "default"}]:
            note_bug(
                "get_databases hardcoded",
                "Always returns [{\"name\": \"default\"}] regardless of connection. "
                "DynamoDB doesn't have databases, but returning a fake 'default' is misleading. "
                "Should return [] or document this as a no-op."
            )

    # 8c. get_schemas — empty
    r = send(proc, "get_schemas", {"params": CONN}, req_id=92)
    check("get_schemas", r)

    # 8d. get_foreign_keys — empty
    r = send(proc, "get_foreign_keys", {"params": CONN, "table": TABLE}, req_id=93)
    check("get_foreign_keys", r)

    # 8e. get_routines — empty
    r = send(proc, "get_routines", {"params": CONN}, req_id=94)
    check("get_routines", r)

    # 8f. get_views — empty
    r = send(proc, "get_views", {"params": CONN}, req_id=95)
    check("get_views", r)

    # 8g. get_indexes — check GSI unique flag
    r = send(proc, "get_indexes", {"params": CONN, "table": TABLE}, req_id=96)
    check("get_indexes", r)
    if "result" in r:
        indexes = r["result"]
        for idx in indexes:
            if not idx.get("is_primary") and idx.get("is_unique"):
                note_bug(
                    "GSI marked as unique",
                    f"Index '{idx.get('name')}' is a GSI but marked is_unique=true. "
                    "DynamoDB GSIs do not enforce uniqueness."
                )

    print()
    print("=" * 70)
    print("  SECTION 9: execution_time_ms")
    print("=" * 70)

    r = send(proc, "execute_query", {
        "params": CONN,
        "query": f"SELECT * FROM {TABLE}",
    }, req_id=100)
    check("query for timing", r)
    if "result" in r:
        exec_time = r["result"].get("execution_time_ms", -1)
        if exec_time == 0:
            note_bug(
                "execution_time_ms always 0",
                "execute_query always returns execution_time_ms=0. "
                "The handler never measures query duration. "
                "Clients relying on this for performance monitoring get no data."
            )

    # Cleanup
    proc.stdin.close()
    proc.wait(timeout=5)

    # Clean up edge case items
    for pk_val in ["sparse-1", "sparse-2"]:
        try:
            ddb_call("DeleteItem", {
                "TableName": "edge_cases",
                "Key": {"pk": {"S": pk_val}},
            })
        except Exception:
            pass

    # Clean up test items from edge case inserts
    for uid in ["usr-nested", "usr-array", "usr-bool", "usr-null", "usr-unicode",
                "usr-quotes", "usr-numbers", "usr-empty", "usr-long", "usr-deep"]:
        try:
            ddb_call("DeleteItem", {
                "TableName": TABLE,
                "Key": {"id": {"S": uid}, "created_at": {"S": "2026-01-01T00:00:00Z"}},
            })
        except Exception:
            pass

    # Report
    print()
    print("=" * 70)
    print(f"  RESULTS: {len(passed)} passed, {len(failed)} failed, {len(bugs)} bugs found")
    print("=" * 70)

    if failed:
        print("\n  FAILURES:")
        for name, detail in failed:
            print(f"  ❌ {name}: {str(detail)[:120]}")

    if bugs:
        print(f"\n  🐛 BUGS & GAPS ({len(bugs)}):")
        for i, (name, detail) in enumerate(bugs, 1):
            print(f"  {i}. [{name}]")
            print(f"     {detail}")

    print()
    if not failed:
        print("  ✅ All checks passed (bugs/gaps listed above are design issues, not crashes)")
    else:
        print(f"  ⚠️  {len(failed)} check(s) FAILED")

    return 0 if not failed else 1


if __name__ == "__main__":
    sys.exit(main())
