#!/usr/bin/env python3
"""Create a NetBird setup key without putting its plaintext in SSM or files.

AWS_VAULT_PROFILE names the aws-vault profile of the account that holds the
instance; the script reads no other credential setting.
"""

import argparse
import base64
import datetime as dt
import getpass
import hashlib
import json
import os
import re
import secrets
import shlex
import string
import subprocess
import sys
import time
import uuid


DEFAULTS = {"bootstrap": (1, 1), "admin": (7, 3), "operator": (30, 25), "peer": (7, 1)}
TERMINAL = {"Success", "Failed", "TimedOut", "Cancelled"}


def aws_command():
    """The SSM command prefix, under the caller's aws-vault profile."""
    profile = os.environ.get("AWS_VAULT_PROFILE", "")
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", profile):
        raise RuntimeError(
            "AWS_VAULT_PROFILE must name the aws-vault profile of the account that holds the instance")
    return ["aws-vault", "exec", profile, "--no-session", "--", "aws", "ssm"]


def arguments(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", required=True)
    parser.add_argument("--role", required=True, choices=DEFAULTS)
    parser.add_argument("--ttl-days")
    parser.add_argument("--usage-limit")
    parser.add_argument("--note", default="")
    parser.add_argument("--groups", default="")
    parser.add_argument("--ssm-instance", required=True)
    parser.add_argument("--ssm-region", required=True)
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", args.site):
        parser.error("--site must be a site identifier (letters, digits, underscore or hyphen)")
    if not re.fullmatch(r"i-[a-f0-9]{8}(?:[a-f0-9]{9})?", args.ssm_instance):
        parser.error("--ssm-instance must be an EC2 instance ID")
    if not re.fullmatch(r"[a-z]{2}(?:-[a-z]+)+-[0-9]+", args.ssm_region):
        parser.error("--ssm-region must be an AWS region name")
    ttl_default, usage_default = DEFAULTS[args.role]
    for field, default in (("ttl_days", ttl_default), ("usage_limit", usage_default)):
        value = getattr(args, field)
        if value is None:
            setattr(args, field, default)
        elif not re.fullmatch(r"[0-9]{1,10}", value) or not 1 <= int(value) <= 2147483647:
            parser.error("--" + field.replace("_", "-") + " must be a positive 32-bit integer")
        else:
            setattr(args, field, int(value))
    if args.ttl_days > ttl_default and not args.note.strip():
        parser.error("a TTL above the role default requires an explanation in --note")
    try:
        args.expires = dt.datetime.now(dt.timezone.utc) + dt.timedelta(days=args.ttl_days)
    except (ValueError, OverflowError):
        parser.error("--ttl-days exceeds the supported calendar range")
    args.auto_groups = [group.strip() for group in args.groups.split(",") if group.strip()]
    return args


def sql_literal(value):
    # Escape backslashes as well as quotes. Keep physical newlines out of SQL
    # literals, including any psql meta-command embedded in an operator note.
    value = value.replace("\\", "\\\\").replace("'", "''")
    value = value.replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")
    return "E'" + value + "'"


def request(args, key_value, key_id, key_name, created_by):
    key_hash = base64.b64encode(hashlib.sha256(key_value.encode("ascii")).digest()).decode("ascii")
    # Same key and masked-display format as mgmt-host-bootstrap/main.go.
    key_secret = key_value[:5] + "****"
    key_type = "one-off" if args.role == "peer" else "reusable"
    literal = sql_literal
    sql = f"""BEGIN;
CREATE TABLE IF NOT EXISTS setup_keys_audit (
  id TEXT PRIMARY KEY, ts TIMESTAMPTZ DEFAULT NOW(),
  event TEXT NOT NULL, key_id TEXT NOT NULL, key_name TEXT,
  role TEXT, created_by TEXT, note TEXT, ttl_days INT, usage_limit INT
);
DO $account_guard$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM accounts) THEN
    RAISE EXCEPTION 'no NetBird account exists';
  END IF;
END; $account_guard$;
INSERT INTO setup_keys
  (id, account_id, key, key_secret, name, type, created_at,
   expires_at, updated_at, revoked, used_times, usage_limit, ephemeral, auto_groups)
VALUES
  ({literal(key_id)}, (SELECT id FROM accounts ORDER BY created_at ASC LIMIT 1),
   {literal(key_hash)}, {literal(key_secret)}, {literal(key_name)}, {literal(key_type)},
   NOW(), {literal(args.expires.strftime('%Y-%m-%dT%H:%M:%SZ'))}::timestamptz, NOW(),
   false, 0, {args.usage_limit}, false, {literal(json.dumps(args.auto_groups))});
INSERT INTO setup_keys_audit
  (id, event, key_id, key_name, role, created_by, note, ttl_days, usage_limit)
VALUES
  ({literal('audit_' + key_id)}, 'create', {literal(key_id)}, {literal(key_name)},
   {literal(args.role)}, {literal(created_by)}, {literal(args.note)},
   {args.ttl_days}, {args.usage_limit});
COMMIT;
"""
    audit = {
        "ts": dt.datetime.now(dt.timezone.utc).isoformat(), "event": "create",
        "key_id": key_id, "key_name": key_name, "role": args.role,
        "created_by": created_by, "ttl_days": args.ttl_days,
        "usage_limit": args.usage_limit, "auto_groups": args.auto_groups, "note": args.note,
    }
    # Encode SQL data rather than inserting it into a shell heredoc. A note
    # containing a heredoc terminator or shell syntax cannot become a command.
    encoded = base64.b64encode(sql.encode("utf-8")).decode("ascii")
    command = (
        "set -eu\n"
        f"printf '%s' {shlex.quote(encoded)} | base64 -d | "
        "docker exec -i postgres-a psql -X -v ON_ERROR_STOP=1 -U postgres netbird "
        ">/dev/null 2>/dev/null\n"
        f"printf '%s\\n' {shlex.quote(json.dumps(audit))} >> /var/log/setup-key-audit.log\n"
    )
    return {"commands": [command]}


def aws_call(arguments, payload=None):
    try:
        result = subprocess.run(
            aws_command() + arguments, input=payload, text=True, capture_output=True,
            timeout=60, check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        raise RuntimeError("AWS command could not complete; no request was automatically retried") from None
    if result.returncode:
        # CLI diagnostics can echo request data. Do not print either stream.
        raise RuntimeError("AWS command failed; no request was automatically retried")
    return result.stdout.strip()


def run(args):
    random_id = "".join(secrets.choice(string.ascii_lowercase + string.digits) for _ in range(6))
    key_id = f"sk_{args.site}_{args.role}_{random_id}"
    key_name = f"{args.site}-{args.role}-{random_id}"
    key_value = str(uuid.uuid4()).upper()
    payload = json.dumps(request(args, key_value, key_id, key_name, getpass.getuser()))
    # /dev/stdin supplies the JSON through a pipe, never a plaintext temp file
    # or a credential-bearing process argument.
    try:
        command_id = aws_call([
            "send-command", "--region", args.ssm_region, "--instance-ids", args.ssm_instance,
            "--document-name", "AWS-RunShellScript", "--parameters", "file:///dev/stdin",
            "--output", "text", "--query", "Command.CommandId",
        ], payload)
    except RuntimeError as error:
        raise RuntimeError(f"{error}; check SSM history for key {key_id} before retrying") from None
    try:
        uuid.UUID(command_id)
    except (ValueError, AttributeError):
        raise RuntimeError(f"SSM returned no valid command ID; check history for key {key_id} before retrying") from None
    status = "Pending"
    for attempt in range(30):
        try:
            status = aws_call([
                "get-command-invocation", "--region", args.ssm_region,
                "--instance-id", args.ssm_instance, "--command-id", command_id,
                "--query", "Status", "--output", "text",
            ])
        except RuntimeError:
            status = "Pending"
        if status in TERMINAL:
            break
        if attempt < 29:
            time.sleep(2)
    if status != "Success":
        # Print only known states; an unexpected provider response may contain
        # sensitive data. A failed audit write can follow a committed key.
        state = status if status in TERMINAL else "not confirmed"
        raise RuntimeError(f"SSM command {command_id}: {state}; check its outcome before retrying")
    return {
        "key_id": key_id, "key_name": key_name, "key_value": key_value,
        "role": args.role, "ttl_days": args.ttl_days, "usage_limit": args.usage_limit,
        "expires_at": args.expires.strftime("%Y-%m-%dT%H:%M:%SZ"), "note": args.note,
    }


def main():
    args = arguments(sys.argv[1:])
    try:
        aws_command()  # a missing profile stops the run before a key is generated
        output = run(args)
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        return 1
    # Existing interface: the operator copies this one-time output into the
    # secret store. Neither SSM nor either audit log receives the plaintext.
    print(json.dumps(output))
    return 0


if __name__ == "__main__":
    sys.exit(main())
