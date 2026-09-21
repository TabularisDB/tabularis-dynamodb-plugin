#!/usr/bin/env python3
"""Shared harness for the live Python suites: which binary to drive, and the
DynamoDB Local fixtures they assume.

Both used to be implicit (#79):

* every suite hard-coded ``../target/release/dynamodb-plugin.exe``, so a branch
  build could only be exercised by overwriting ``target/release`` — and the
  suites could not run on Linux or macOS at all, where the binary has no
  ``.exe`` suffix. ``PLUGIN_BINARY`` (env var) overrides it;
* every suite assumed a ``test_users`` table with a *composite* key
  (``id`` HASH + ``created_at`` RANGE) that nothing created, not even
  ``just seed-dynamodb``, so a missing fixture showed up as a wall of
  ``ResourceNotFoundException``. ``ensure_test_users()`` creates and seeds it.

Run this file directly to (re)create the fixtures: ``just seed-fixtures``.
"""

import json
import os
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)

#: Binary under test. Override with ``PLUGIN_BINARY`` to exercise a branch or
#: debug build without overwriting ``target/release``.
BINARY = os.environ.get("PLUGIN_BINARY") or os.path.join(
    REPO,
    "target",
    "release",
    "dynamodb-plugin.exe" if os.name == "nt" else "dynamodb-plugin",
)

ENDPOINT = os.environ.get("DYNAMODB_ENDPOINT", "http://localhost:8000")
REGION = "us-east-1"
ACCESS_KEY = "local"
SECRET_KEY = "local"

#: Composite-key fixture the suites share.
TABLE = "test_users"
TABLE_KEY_SCHEMA = (("id", "HASH"), ("created_at", "RANGE"))
SEED_ITEMS = (
    {
        "id": {"S": "usr-001"},
        "created_at": {"S": "2026-01-01T00:00:00Z"},
        "name": {"S": "Alice"},
        "email": {"S": "alice@example.com"},
        "age": {"N": "31"},
        "active": {"BOOL": True},
        "role": {"S": "tester"},
    },
    {
        "id": {"S": "usr-002"},
        "created_at": {"S": "2026-01-02T00:00:00Z"},
        "name": {"S": "Bob"},
        "email": {"S": "bob@example.com"},
        "age": {"N": "25"},
        "active": {"BOOL": False},
        "role": {"S": "viewer"},
    },
)


def connection(endpoint=None, **overrides):
    """AWS connection params (the dict inside the plugin's ``params`` field)."""
    conn = {
        "region": REGION,
        "access_key_id": ACCESS_KEY,
        "secret_access_key": SECRET_KEY,
        "endpoint": endpoint or ENDPOINT,
    }
    conn.update(overrides)
    return conn


def params(endpoint=None, **overrides):
    """Connection params wrapped the way the suites send them."""
    return {"params": connection(endpoint, **overrides)}


def ddb(target, payload, endpoint=None, access_key=None):
    """Call DynamoDB Local's HTTP API directly — no SDK, and no signature
    verification (see #79: DynamoDB Local *does* reject access keys that are not
    20 alphanumeric characters, which is why the key here is plain ``local``)."""
    request = urllib.request.Request(
        endpoint or ENDPOINT,
        data=json.dumps(payload).encode(),
        headers={
            "Content-Type": "application/x-amz-json-1.0",
            "X-Amz-Target": "DynamoDB_20120810." + target,
            "Authorization": (
                "AWS4-HMAC-SHA256 Credential=%s/20260724/%s/dynamodb/aws4_request"
                % (access_key or ACCESS_KEY, REGION)
            ),
        },
    )
    try:
        response = urllib.request.urlopen(request)
    except Exception as exc:  # noqa: BLE001 - one clear message beats a traceback
        raise RuntimeError(
            "DynamoDB Local not reachable at %s (%s) — start it with `just run-dynamodb`"
            % (endpoint or ENDPOINT, exc)
        ) from exc
    return json.loads(response.read())


def table_exists(name, endpoint=None):
    try:
        ddb("DescribeTable", {"TableName": name}, endpoint=endpoint)
        return True
    except Exception:
        return False


def ensure_table(name, key_schema, endpoint=None):
    """Create ``name`` when it is absent. Returns True if it had to be created."""
    if table_exists(name, endpoint):
        return False
    ddb(
        "CreateTable",
        {
            "TableName": name,
            "KeySchema": [
                {"AttributeName": column, "KeyType": kind} for column, kind in key_schema
            ],
            "AttributeDefinitions": [
                {"AttributeName": column, "AttributeType": "S"} for column, _ in key_schema
            ],
            "BillingMode": "PAY_PER_REQUEST",
        },
        endpoint=endpoint,
    )
    return True


def ensure_test_users(endpoint=None):
    """Create + seed the composite-key fixture the suites assume (#79).

    Idempotent: seeds are re-put so a suite that deleted them still sees them.
    Returns True when the table had to be created.
    """
    created = ensure_table(TABLE, TABLE_KEY_SCHEMA, endpoint=endpoint)
    for item in SEED_ITEMS:
        ddb("PutItem", {"TableName": TABLE, "Item": item}, endpoint=endpoint)
    return created


def ensure_edge_cases(endpoint=None):
    """Single-key ``edge_cases`` table used by the deeper suites."""
    return ensure_table("edge_cases", (("pk", "HASH"),), endpoint=endpoint)


def describe_binary():
    return "%s (PLUGIN_BINARY=%s)" % (
        BINARY,
        os.environ.get("PLUGIN_BINARY") or "unset, using the release default",
    )


def main():
    print("binary:   %s" % describe_binary())
    print("endpoint: %s" % ENDPOINT)
    for ensure, name in ((ensure_test_users, TABLE), (ensure_edge_cases, "edge_cases")):
        try:
            created = ensure(endpoint=ENDPOINT)
        except Exception as exc:  # noqa: BLE001 - report and fail, don't traceback
            print("cannot reach DynamoDB Local at %s: %s" % (ENDPOINT, exc))
            print("start it with `just run-dynamodb` first")
            return 1
        print("%s: %s" % (name, "created and seeded" if created else "already present, re-seeded"))
    return 0


if __name__ == "__main__":
    sys.exit(main())