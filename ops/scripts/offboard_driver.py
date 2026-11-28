#!/usr/bin/env python3
"""Remove one person's access from every plane on every selected goatnet."""

from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

import offboard_accounts
import offboard_departure
import offboard_subject

PLANES = (
    "host-ssh-keys", "operator-keys", "netbird-devices", "netbird-setup-keys",
    "netbird-user", "fabric-slot", "sponsorship", "ory", "collab-kanidm",
    "public-door-kanidm", "onepassword", "github",
)
# The team credential store (ADR 1203) is a plane only where this host is
# pointed at one (BAO_ADDR) or the operator names the person's store login.
STORE_PLANES = PLANES[:11] + ("openbao",) + PLANES[11:]
CHECKED, NOT_RUN, CHECK_FAILED = "removed-and-checked", "not-run", "check-failed"
UNBOUND = "removed-login-unbound"
NOT_APPLICABLE = "not-applicable-no-keys"
# Planes that are one fact for the whole team, recorded under every goatnet.
TEAM, TEAM_PLANES = "team", ("onepassword", "openbao", "github")
# External accounts a deployment declares (ADR 1272 section 3).
ACCOUNT = "account"
BY_HAND = "not automated: do by hand"
COLUMNS = ("where", "what", "found", "result", "verified", "audit")
# The departure-record row kept open when an audit entry did not reach the fabric.
AUDIT_ROW = "audit-entries"
HEADER = {key: key for key in COLUMNS}
VERIFIED_STATUSES = (CHECKED, NOT_APPLICABLE)


def utcnow() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def outcome(status=NOT_RUN, before="unknown", after="unknown", detail="not run") -> dict:
    return {"status": status, "before": before, "after": after, "detail": detail,
            "verified": status in VERIFIED_STATUSES}


def declared_value(repo, key: str) -> str:
    """One plain `key: value` line of ops/ansible/group_vars/all.yml, the file a
    checkout declares its own names in; '' when the file or the key is absent."""
    try:
        text = (Path(repo) / "ops/ansible/group_vars/all.yml").read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        return ""
    found = re.search(rf"^{re.escape(key)}:[ \t]*(.*?)[ \t]*(?:#.*)?$", text, re.M)
    return found.group(1).strip().strip("\"'") if found else ""


def public_door_admin_var(repo) -> str:
    """The variable the public door's idm_admin password is read from:
    GOAT_IDM_ADMIN_PW, else the variable the checkout names as
    public_door_idm_admin_pw_env when that one is set and GOAT_IDM_ADMIN_PW is not."""
    if not os.getenv("GOAT_IDM_ADMIN_PW"):
        name = declared_value(repo, "public_door_idm_admin_pw_env")
        if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name) and os.getenv(name):
            return name
    return "GOAT_IDM_ADMIN_PW"


def atomic_json(path: Path, value: dict, private_parent: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700 if private_parent else 0o777)
    if private_parent:
        path.parent.chmod(0o700)
    fd, name = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    try:
        os.chmod(name, 0o600)
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        try:
            os.unlink(name)
        except FileNotFoundError:
            # A successful os.replace moved the temporary file into place.
            pass


def run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, text=True, capture_output=True, check=False)


def json_output(proc: subprocess.CompletedProcess[str], label: str) -> dict:
    if proc.returncode:
        raise RuntimeError(f"{label} exited {proc.returncode}: {proc.stderr.strip() or proc.stdout.strip()}")
    try:
        value = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{label} returned unreadable JSON") from exc
    if not isinstance(value, dict):
        raise RuntimeError(f"{label} returned a non-object")
    return value


def summary_presence(value: object, absent: tuple[str, ...]) -> str:
    if not isinstance(value, str) or value.startswith(("UNREAD:", "skipped:")):
        return "unknown"
    return "absent" if any(word in value for word in absent) else "present"


def verify_status(value: dict, fabric_teammates: int = 0,
                  host_ssh_verified: bool = False) -> dict[str, tuple[bool, str]]:
    errors = value.get("read_errors")
    readable = isinstance(errors, list) and not errors
    mesh = value.get("mesh_status")
    mesh_ok = (isinstance(mesh, dict) and mesh.get("read_ok") is True
               and mesh.get("peer_ids") == [] and mesh.get("errors") == [])
    ory = value.get("ory_status")
    ory_ok = False
    if (isinstance(ory, dict) and ory.get("read_ok") is True and ory.get("errors") == []
            and type(ory.get("session_count")) is int and ory.get("session_count") == 0):
        ory_ok = ory.get("identity_present") is False or str(ory.get("state", "")).lower() == "inactive"
    operator = value.get("operator-key")
    operator_ok = (isinstance(operator, str) and not operator.startswith(("UNREAD:", "skipped:"))
                   and " active" not in f" {operator.lower()}"
                   and " requested" not in f" {operator.lower()}")
    nb = value.get("netbird-role")
    nb_ok = isinstance(nb, str) and nb.lower() in ("no dashboard user", "user (blocked)")
    structured = {}
    for plane, key in (("host-ssh-keys", "ssh_status"),
                       ("netbird-setup-keys", "setup_key_status"),
                       ("fabric-slot", "fabric_status"),
                       ("sponsorship", "sponsorship_status")):
        status = value.get(key)
        structured[plane] = (
            isinstance(status, dict) and status.get("read_ok") is True
            and status.get("present") is False and status.get("errors") == []
        )
    fabric = value.get("fabric_status")
    fabric_present = isinstance(fabric, dict) and fabric.get("present") is True
    fabric_ok = structured["fabric-slot"] or (
        fabric_teammates > 0 and isinstance(fabric, dict)
        and fabric.get("read_ok") is True and fabric_present and fabric.get("errors") == []
    )
    fabric_detail = "fabric slot is absent"
    if fabric_teammates > 0 and fabric_present:
        noun = "member" if fabric_teammates == 1 else "members"
        fabric_detail = (f"fabric org kept because it is shared by {fabric_teammates} "
                         f"other active {noun} on this goatnet")
    return {
        "ory": (ory_ok, "login inactive or absent and zero live sessions"),
        "host-ssh-keys": (
            structured["host-ssh-keys"] and host_ssh_verified,
            ("host authorized_keys readback passed and no active console keys remain"
             if host_ssh_verified else
             "host authorized_keys readback did not run; console inventory alone cannot prove removal"),
        ),
        "netbird-devices": (mesh_ok, "zero matching devices"),
        "operator-keys": (readable and operator_ok, "no active or requested operator keys"),
        "netbird-user": (readable and nb_ok, "dashboard user absent or blocked"),
        "netbird-setup-keys": (structured["netbird-setup-keys"], "no active setup keys remain"),
        "fabric-slot": (fabric_ok, fabric_detail),
        "sponsorship": (structured["sponsorship"], "sponsorship is absent"),
    }


def require_readable_status(value: dict, label: str) -> None:
    errors = value.get("read_errors")
    if not isinstance(errors, list) or errors:
        raise RuntimeError(f"{label} has unreadable plane(s): {errors!r}")
    for plane in ("mesh_status", "ory_status"):
        status = value.get(plane)
        if not isinstance(status, dict) or status.get("read_ok") is not True:
            raise RuntimeError(f"{label} has unreadable {plane.replace('_status', '')} state")
    mesh = value["mesh_status"]
    peer_ids, mesh_errors = mesh.get("peer_ids"), mesh.get("errors")
    if (not isinstance(peer_ids, list)
            or any(not isinstance(peer, str) or not peer.strip() for peer in peer_ids)
            or not isinstance(mesh_errors, list)
            or any(not isinstance(error, str) for error in mesh_errors)
            or mesh_errors):
        raise RuntimeError(f"{label} has malformed or unreadable mesh state")
    for plane in ("ssh_status", "setup_key_status", "fabric_status", "sponsorship_status"):
        status = value.get(plane)
        if not isinstance(status, dict) or status.get("read_ok") is not True:
            raise RuntimeError(f"{label} has unreadable {plane.replace('_status', '')} state")
    for plane in ("operator-key", "netbird-role", "ssh", "sponsorship"):
        if not isinstance(value.get(plane), str) or value[plane].startswith(("UNREAD:", "skipped:")):
            raise RuntimeError(f"{label} has unreadable {plane} state")


def ssh_revoke_evidence(proc: subprocess.CompletedProcess[str]) -> dict | None:
    """Return the CLI's structured SSH evidence, ignoring malformed output lines."""
    if proc.returncode:
        return None
    for line in reversed(proc.stdout.splitlines()):
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(value, dict):
            continue
        results = value.get("results")
        if value.get("direction") != "revoke --full" or not isinstance(results, list):
            continue
        ssh = [row for row in results
               if isinstance(row, dict) and row.get("plane") == "ssh"]
        return (ssh[0] if len(ssh) == 1
                and ssh[0].get("outcome") in ("done", "no-change") else None)
    return None


def ssh_binding_errors(evidence: dict | None, login: str) -> list[str]:
    if evidence is None:
        return ["structured SSH revoke evidence is missing"]
    errors = []
    reported = evidence.get("github_login")
    if not isinstance(reported, str) or reported.casefold() != login.casefold():
        errors.append(f"github_login did not match resolved login {login}")
    if evidence.get("github_login_source") != "flag":
        errors.append("github_login_source was not flag")
    if evidence.get("github_keys_read_ok") is not True:
        errors.append("GitHub key read was not successful")
    return errors


def ory_access(value: object) -> str:
    if not isinstance(value, dict) or value.get("read_ok") is not True:
        return "unknown"
    if value.get("identity_present") is False:
        return "absent"
    sessions = value.get("session_count")
    if (value.get("identity_present") is True and str(value.get("state", "")).lower() == "inactive"
            and type(sessions) is int and sessions == 0):
        return "absent"
    return "present"


def manifest_org_prefix(repo: Path, goatnet: str, handle: str) -> str:
    path = repo / "ops/operators" / goatnet / f"{handle}.yml"
    try:
        data = offboard_subject.yaml.load(path.read_text(encoding="utf-8"),
                                          Loader=offboard_subject.UniqueLoader)
        org = data.get("org", data.get("vendor")) if isinstance(data, dict) else None
        prefix = org.get("short_tag") if isinstance(org, dict) else None
    except (OSError, UnicodeError, offboard_subject.yaml.YAMLError,
            offboard_subject.Refusal) as exc:
        raise RuntimeError(f"{goatnet}: cannot read fabric identifier") from exc
    if org is None:
        # A manifest with no org block (one synced for a person who was never
        # given an org tag) names no fabric org. Onboarding mints the org under
        # the handle when no tag is given, so the handle is the identifier the
        # fabric and setup-key reads look for. A block that is present and
        # holds no valid tag is still refused: that manifest is wrong.
        return handle
    if not offboard_subject.slug(prefix):
        raise RuntimeError(f"{goatnet}: manifest has no valid org.short_tag")
    return prefix


def active_tag_teammates(repo: Path, goatnet: str, handle: str, tag: str) -> int:
    """Count other active handles with this tag exactly in one goatnet directory."""
    env_dir = repo / "ops/operators" / goatnet
    try:
        entries = list(env_dir.iterdir())
    except OSError:
        return 0
    teammates = 0
    for path in entries:
        if not path.is_file() or path.suffix != ".yml" or path.stem == handle:
            continue
        try:
            data = offboard_subject.yaml.load(path.read_text(encoding="utf-8"),
                                              Loader=offboard_subject.UniqueLoader)
            if not isinstance(data, dict):
                continue
            if "org" in data and "vendor" in data:
                continue
            identity = data.get("identity")
            org = data.get("org") if "org" in data else data.get("vendor")
            candidate_tag = org.get("short_tag") if isinstance(org, dict) else None
            status = identity.get("status") if isinstance(identity, dict) else None
            if (isinstance(candidate_tag, str) and candidate_tag.strip() == tag
                    and isinstance(status, str) and status.strip() == "active"):
                teammates += 1
        except (OSError, UnicodeError, offboard_subject.yaml.YAMLError,
                offboard_subject.Refusal):
            continue
    return teammates


def shared_credentials(teammates: dict[str, int], org_prefixes: dict[str, str]) -> list[dict]:
    result = [
        {"credential": "portal legacy operator key", "scope": "per-goatnet"},
        {"credential": "goatnet CA age recipients", "scope": "per-goatnet"},
    ]
    result.extend(
        {"credential": "shared org fabric certificate bundle", "goatnet": goatnet,
         "org_tag": org_prefixes[goatnet]}
        for goatnet, count in teammates.items() if count > 0
    )
    return result


def print_shared_credentials(credentials: list[dict]) -> None:
    print("\n==> shared credentials to assess separately")
    for item in credentials:
        if item["credential"] == "shared org fabric certificate bundle":
            print(f"    {item['goatnet']} org {item['org_tag']} fabric certificate bundle "
                  "(shared with active teammates; not rotated here)")
        elif item["credential"] == "portal legacy operator key":
            print("    portal legacy operator key (shared per goatnet; not rotated here)")
        else:
            print("    goatnet CA age recipients (rotation is a separate CA operation)")


class Kanidm:
    """Minimal administrator client for the two sign-on doors."""

    def __init__(self, base: str, password: str = "", token: str = ""):
        self.base, self.password, self.token_value = base.rstrip("/"), password, token

    def request(self, method: str, path: str, payload=None, token="") -> tuple[int, object, str]:
        data = None if payload is None else json.dumps(payload).encode()
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        request = Request(self.base + path, data=data, headers=headers, method=method)
        try:
            with urlopen(request, timeout=20) as response:
                return response.status, response.headers, response.read().decode()
        except HTTPError as exc:
            return exc.code, exc.headers, exc.read().decode(errors="replace")
        except (URLError, OSError) as exc:
            raise RuntimeError(f"Kanidm request failed: {exc}") from exc

    def token(self) -> str:
        if self.token_value:
            return self.token_value
        if not self.password:
            raise RuntimeError("Kanidm administrator credential is missing")
        status, headers, _ = self.request("POST", "/v1/auth", {"step": {"init": "idm_admin"}})
        sid = headers.get("x-kanidm-auth-session-id", "")
        if status >= 400 or not sid:
            raise RuntimeError("Kanidm administrator sign-in initialization failed")
        status, headers, _ = self.auth(sid, {"step": {"begin": "password"}})
        sid = headers.get("x-kanidm-auth-session-id", sid)
        if status >= 400:
            raise RuntimeError("Kanidm administrator sign-in negotiation failed")
        status, _, body = self.auth(sid, {"step": {"cred": {"password": self.password}}})
        try:
            token = json.loads(body)["state"]["success"]
        except (json.JSONDecodeError, KeyError, TypeError):
            token = ""
        if status >= 400 or not token:
            raise RuntimeError("Kanidm administrator sign-in failed")
        return token

    def auth(self, sid: str, payload: object) -> tuple[int, object, str]:
        request = Request(
            self.base + "/v1/auth", data=json.dumps(payload).encode(), method="POST",
            headers={"Content-Type": "application/json", "x-kanidm-auth-session-id": sid},
        )
        try:
            with urlopen(request, timeout=20) as response:
                return response.status, response.headers, response.read().decode()
        except HTTPError as exc:
            return exc.code, exc.headers, exc.read().decode(errors="replace")

    def find(self, email: str, token: str) -> dict | None:
        status, _, body = self.request("GET", "/v1/person", token=token)
        if status != 200:
            raise RuntimeError(f"Kanidm account listing returned HTTP {status}")
        try:
            rows = json.loads(body)
        except json.JSONDecodeError as exc:
            raise RuntimeError("Kanidm account listing returned unreadable JSON") from exc
        if not isinstance(rows, list):
            raise RuntimeError("Kanidm account listing returned a non-array")
        matches = [row for row in rows if isinstance(row, dict)
                   and any(str(v).casefold() == email.casefold()
                           for v in row.get("attrs", {}).get("mail", []))]
        if len(matches) > 1:
            raise RuntimeError("Kanidm account lookup is ambiguous")
        return matches[0] if matches else None

    def read(self, email: str) -> tuple[str, dict | None]:
        token = self.token()
        return token, self.find(email, token)

    @staticmethod
    def account_name(row: dict) -> str:
        names = row.get("attrs", {}).get("name", [])
        if len(names) != 1 or not names[0]:
            raise RuntimeError("Kanidm account has no unique sign-in name")
        return str(names[0])

    @staticmethod
    def is_expired(row: dict) -> bool:
        values = row.get("attrs", {}).get("account_expire", [])
        if len(values) != 1:
            return False
        try:
            expiry = datetime.fromisoformat(str(values[0]).replace("Z", "+00:00"))
        except (TypeError, ValueError):
            return False
        if expiry.tzinfo is None:
            return False
        return expiry <= datetime.now(timezone.utc)

    def remove(self, email: str, delete: bool, known: tuple[str, dict | None]) -> tuple[str, str]:
        token, row = known
        before = "absent" if row is None or (not delete and self.is_expired(row)) else "present"
        if row is not None:
            name = quote(self.account_name(row), safe="")
            method = "DELETE" if delete else "PUT"
            path = (f"/v1/person/{name}" if delete else
                    f"/v1/person/{name}/_attr/account_expire")
            payload = None if delete else ["1970-01-01T00:00:00Z"]
            status, _, _ = self.request(
                method, path, payload, token,
            )
            if status not in (200, 204) and not (delete and status == 404):
                action = "deletion" if delete else "expiry"
                raise RuntimeError(f"Kanidm account {action} returned HTTP {status}")
        row = self.find(email, token)
        if row is None:
            return before, "absent"
        if delete:
            raise RuntimeError("Kanidm readback still shows the collaboration account")
        if not self.is_expired(row):
            raise RuntimeError("Kanidm readback still shows an unexpired account")
        return before, "absent"


class Driver:
    def __init__(self, args: argparse.Namespace):
        self.args, self.errors = args, []
        self.planes = STORE_PLANES if args.openbao else PLANES
        self.receipt = {
            "schema": "goat.offboard.receipt.v1", "subject": args.subject,
            "started_at": utcnow(), "finished_at": None, "outcome": "running",
            "github": {"login": args.github_login or None,
                       "source": args.github_source},
            "shared_credentials": [],
            "goatnets": {g: {plane: outcome() for plane in self.planes} for g in args.goatnets},
            "accounts": {a["id"]: outcome() for a in offboard_accounts.automated(args.accounts)},
            "by_hand": [{"id": a["id"], "what": a["what"]}
                        for a in offboard_accounts.manual(args.accounts)],
        }

    def save(self) -> None:
        atomic_json(self.args.receipt, self.receipt, self.args.default_receipt)

    def fail(self, message: str) -> None:
        self.errors.append(message)
        print(f"    CHECK FAILED  {message}", file=sys.stderr)

    def record(self, goatnet: str, plane: str, value: dict) -> None:
        how = {
            "host-ssh-keys": "goat admin platform-admin revoke/status",
            "operator-keys": "goat admin platform-admin revoke/status",
            "netbird-devices": "goat admin onboard --revoke plus platform-admin status",
            "netbird-setup-keys": "goat admin onboard --revoke plus platform-admin status",
            "netbird-user": "goat admin platform-admin revoke/status",
            "fabric-slot": ("goat admin onboard --revoke plus platform-admin status; "
                            "a shared org may remain for active teammates"),
            "sponsorship": "goat admin platform-admin revoke/status",
            "ory": "goat admin onboard --revoke plus Kratos active-session status",
            "collab-kanidm": "Kanidm person DELETE plus list readback",
            "public-door-kanidm": "Kanidm account_expire PUT plus list readback",
            "onepassword": "op group user revoke plus group-member readback",
            "openbao": ("ops/secrets/openbao/revoke-person ends every token of the "
                        "person's store identity, then a dry run reads back zero"),
            "github": ("GitHub enterprise-team membership, org membership, outside-collaborator, "
                       "and pending invitation DELETE plus list readback"),
        }
        service = {
            "collab-kanidm": "collab Kanidm idm_admin",
            "public-door-kanidm": "public-door Kanidm idm_admin",
            "onepassword": "1Password CLI account",
            "openbao": "OpenBao token of the operator (goat-people-admin)",
            "github": "GitHub CLI account",
        }.get(plane, f"goat-cli context {goatnet}-op")
        value.update({"how": how[plane], "operator": self.args.operator,
                      "service_identity": service,
                      "verified": value["status"] in VERIFIED_STATUSES})
        if plane == "github":
            value.update({"login": self.args.github_login or None,
                          "login_source": self.args.github_source})
        self.receipt["goatnets"][goatnet][plane] = value
        self.save()

    def audit_plane(self, goatnet: str, plane: str, value: dict) -> dict:
        result = value["status"] in VERIFIED_STATUSES
        argv = ["goat", "--context", f"{goatnet}-op", "audit", "emit",
                "--actor", self.args.operator, "--surface", "offboard-driver",
                "--action", f"offboard.{plane}", "--target", self.email,
                "--env", goatnet, "--outcome", "ok" if result else "failed",
                "--args-digest", f"subject={self.email} plane={plane}"]
        if plane == "github":
            argv[-1] += (f" github_login={self.args.github_login}"
                         f" github_source={self.args.github_source}")
        if not result:
            argv += ["--error", value["detail"]]
        proc = run(argv)
        value = dict(value)
        # Two facts kept apart: was the removal read back, and did its audit
        # entry reach the fabric. A run missing either does not succeed.
        value["removal_verified"] = result
        value["audit"] = "recorded" if proc.returncode == 0 else "not recorded"
        if proc.returncode:
            message = proc.stderr.strip() or proc.stdout.strip() or "audit command failed"
            self.fail(f"{goatnet} {plane} audit: {message}")
            value["status"] = CHECK_FAILED
            value["verified"] = False
            value["detail"] += f"; operator-action audit failed: {message}"
        return value

    def preflight(self) -> bool:
        missing = [name for name in ("goat", "gh", "op") if shutil.which(name) is None]
        if missing:
            self.fail("preflight: missing command(s): " + ", ".join(missing))
        if shutil.which("gh") and run(["gh", "auth", "status"]).returncode:
            self.fail("preflight: GitHub CLI is not authenticated")
        elif shutil.which("gh"):
            exists = run(["gh", "api", f"users/{self.args.github_login}"])
            if exists.returncode:
                detail = f"GitHub login {self.args.github_login} does not exist"
                self.fail(f"preflight: {detail}")
                for goatnet in self.args.goatnets:
                    self.record(goatnet, "github",
                                outcome(CHECK_FAILED, "unknown", "unknown", detail))
        if shutil.which("op") and run(["op", "whoami"]).returncode:
            self.fail("preflight: 1Password CLI is not signed in")
        for name in ("COLLAB_KANIDM_URL", "PUBLIC_DOOR_KANIDM_URL",
                     public_door_admin_var(self.args.repo)):
            if not os.getenv(name):
                self.fail(f"preflight: {name} is missing")
        if not os.getenv("COLLAB_KANIDM_IDM_ADMIN_PW"):
            self.fail("preflight: COLLAB_KANIDM_IDM_ADMIN_PW is missing (the onboarding API token cannot delete people)")
        if self.args.openbao and not (os.getenv("BAO_ADDR") and self.args.openbao_user):
            self.fail("preflight: openbao: needs both BAO_ADDR and --openbao-user")
        if not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?", self.args.operator):
            self.fail("preflight: --operator or GOAT_OPERATOR_HANDLE is required for audit attribution")
        if self.errors:
            return False

        self.before_status = {}
        for goatnet, _, email, _ in self.subjects:
            args = self.status_args(goatnet, email)
            try:
                status = json_output(run(args), f"{goatnet} preflight status")
                require_readable_status(status, f"{goatnet} preflight status")
                self.before_status[goatnet] = status
            except RuntimeError as exc:
                self.fail(str(exc))

        self.kanidm_before = {}
        for goatnet, _, _, _ in self.subjects:
            for plane in ("collab-kanidm", "public-door-kanidm"):
                try:
                    client = self.kanidm_client(plane)
                    self.kanidm_before[(goatnet, plane)] = (client, client.read(self.email))
                except Exception as exc:
                    self.fail(f"preflight: {goatnet} {plane}: {exc}")

        try:
            self.op_before = self.read_onepassword_groups()
        except RuntimeError as exc:
            self.fail(f"preflight: onepassword: {exc}")
        try:
            self.github_before = self.read_github_orgs()
            self.github_teams_before = self.read_github_enterprise_teams()
        except RuntimeError as exc:
            self.fail(f"preflight: github: {exc}")
        if self.args.openbao:
            try:
                self.bao_before = self.openbao_sessions()
            except RuntimeError as exc:
                self.fail(f"preflight: openbao: {exc}")
        self.account_who, self.account_before = {}, {}
        for account in offboard_accounts.automated(self.args.accounts):
            identifier = account["id"]
            try:
                reason = offboard_accounts.cannot_run(account, self.args.repo)
                if reason:
                    raise RuntimeError(reason)
                who = offboard_accounts.identities(
                    account, self.args.repo, self.subjects, self.email,
                    self.args.github_login, self.args.account_handles)
                self.account_who[identifier] = who
                self.account_before[identifier] = (
                    offboard_accounts.call(self.args.repo, account, "check", who)
                    if who else {"held": False, "items": [], "detail": "declared: none held"})
            except (RuntimeError, offboard_accounts.Refusal) as exc:
                self.fail(f"preflight: account {identifier}: {exc}")
                self.record_account(account, outcome(NOT_RUN, "unknown", "unknown", str(exc)),
                                    audit=False)
        # Last, and only when everything else can run: one operator-action
        # entry per goatnet. A run that cannot write its audit entries would
        # remove access it then cannot attribute, so it stops here instead.
        if not self.errors:
            for goatnet in self.args.goatnets:
                proc = run(["goat", "--context", f"{goatnet}-op", "audit", "emit",
                            "--actor", self.args.operator, "--surface", "offboard-driver",
                            "--action", "offboard.preflight", "--target", self.email,
                            "--env", goatnet, "--outcome", "ok",
                            "--args-digest", f"subject={self.email} reason={self.args.reason}"])
                if proc.returncode:
                    message = proc.stderr.strip() or proc.stdout.strip() or "audit command failed"
                    self.fail(f"preflight: {goatnet} audit: an operator-action entry cannot be "
                              f"written, so nothing was removed: {message}")
        return not self.errors

    def record_account(self, account: dict, value: dict, audit: bool = True) -> None:
        """One external account's row: recorded once, audited on every goatnet."""
        if audit:
            for goatnet in self.args.goatnets:
                value = self.audit_plane(goatnet, f"account.{account['id']}", value)
        value.update({"how": f"plug-in {offboard_accounts.PLUGINS}/{account['type']} "
                             "check, remove, check",
                      "operator": self.args.operator,
                      "service_identity": ("credential in "
                                           + (", ".join(account["credential_env"]) or "the plug-in's own session")),
                      "verified": value["status"] in VERIFIED_STATUSES})
        self.receipt["accounts"][account["id"]] = value
        self.save()

    def account(self, account: dict) -> None:
        """Remove what the person holds in one declared account and read it back."""
        identifier, who = account["id"], self.account_who[account["id"]]
        before = self.account_before[identifier]
        found = "present" if before["held"] else "absent"
        try:
            if not who:
                value = outcome(CHECKED, "absent", "absent", "declared: the person holds none")
            else:
                if before["held"]:
                    offboard_accounts.call(self.args.repo, account, "remove", who)
                after = offboard_accounts.call(self.args.repo, account, "check", who)
                if after["held"]:
                    raise RuntimeError("still held after removal: "
                                       + (", ".join(after["items"]) or after["detail"]))
                value = outcome(CHECKED, found, "absent",
                                after["detail"] or "nothing held at read-back")
        except RuntimeError as exc:
            value = outcome(CHECK_FAILED, found, "unknown", str(exc))
            self.fail(f"account {identifier}: {exc}")
        self.record_account(account, value)

    def status_args(self, goatnet: str, email: str) -> list[str]:
        handle = next(handle for env, handle, _, _ in self.subjects if env == goatnet)
        return ["goat", "--context", f"{goatnet}-op", "admin", "platform-admin", "status",
                email, "--env", goatnet, "--handle", handle,
                "--org-prefix", self.org_prefixes[goatnet], "--json", "--no-1password"]

    def goatnet(self, goatnet: str, handle: str, email: str) -> None:
        ctx = f"{goatnet}-op"
        status_args = self.status_args(goatnet, email)
        before = self.before_status[goatnet]
        platform = run(["goat", "--context", ctx, "admin", "platform-admin", "revoke",
                        email, "--env", goatnet, "--full", "--no-ory", "--no-1password", "--apply",
                        "--json", "--github", self.args.github_login, *self.args.pubkey_args])
        ssh_evidence = ssh_revoke_evidence(platform)
        host_ssh_verified = (ssh_evidence is not None
                             and ssh_evidence.get("host_readback") is True)
        if platform.returncode:
            self.fail(f"{goatnet} platform-admin removal failed: {platform.stderr.strip() or platform.stdout.strip()}")
        onboard = run(["goat", "--context", ctx, "admin", "onboard", handle, "--env", goatnet,
                       "--revoke", "--native", "--collab", "revoke"])
        if onboard.returncode:
            self.fail(f"{goatnet} onboarding removal failed: {onboard.stderr.strip() or onboard.stdout.strip()}")
        try:
            after = json_output(run(status_args), f"{goatnet} post-removal status")
            require_readable_status(after, f"{goatnet} post-removal status")
            checks = verify_status(after, self.teammates[goatnet], host_ssh_verified)
        except RuntimeError as exc:
            after, checks = None, {}
            self.fail(str(exc))

        before_values = self.goatnet_before(goatnet)
        command_ok = {
            "host-ssh-keys": platform.returncode == 0,
            "operator-keys": platform.returncode == 0,
            "netbird-devices": onboard.returncode == 0,
            "netbird-setup-keys": onboard.returncode == 0,
            "netbird-user": platform.returncode == 0,
            "fabric-slot": onboard.returncode == 0,
            "sponsorship": platform.returncode == 0,
            "ory": onboard.returncode == 0,
        }
        for plane in ("host-ssh-keys", "operator-keys", "netbird-devices",
                      "netbird-setup-keys", "netbird-user", "fabric-slot",
                      "sponsorship", "ory"):
            state_ok, detail = checks.get(plane, (False, "post-removal status was unreadable"))
            ok = command_ok[plane] and state_ok
            status = CHECKED if ok else CHECK_FAILED
            if plane == "host-ssh-keys" and platform.returncode == 0:
                binding_errors = ssh_binding_errors(ssh_evidence, self.args.github_login)
                after_ssh = after.get("ssh_status") if isinstance(after, dict) else None
                after_console_clear = (isinstance(after_ssh, dict)
                                       and after_ssh.get("read_ok") is True
                                       and after_ssh.get("present") is False
                                       and after_ssh.get("errors") == [])
                count_errors = []
                if binding_errors:
                    detail = "; ".join(binding_errors)
                    ok = False
                    status = CHECK_FAILED
                else:
                    for field in ("keys_checked", "console_rows_any_status", "pubkey_files"):
                        count = ssh_evidence.get(field)
                        if type(count) is not int or count < 0:
                            count_errors.append(f"{field} was not a non-negative integer")
                    if count_errors:
                        detail = "; ".join(count_errors)
                        ok = False
                        status = CHECK_FAILED
                if not binding_errors and not count_errors and ssh_evidence["keys_checked"] == 0:
                    before_ssh = before.get("ssh_status") if isinstance(before, dict) else None
                    no_key_errors = []
                    if not (isinstance(before_ssh, dict)
                            and before_ssh.get("read_ok") is True
                            and before_ssh.get("present") is False
                            and before_ssh.get("identifiers") == []
                            and before_ssh.get("errors") == []):
                        no_key_errors.append(
                            "pre-removal SSH status did not prove an empty console inventory")
                    if ssh_evidence.get("console_rows_any_status") != 0:
                        no_key_errors.append("console_rows_any_status was not zero")
                    if ssh_evidence.get("pubkey_files") != 0:
                        no_key_errors.append("pubkey_files was not zero")
                    if not after_console_clear:
                        no_key_errors.append("post-removal console SSH status was not empty")
                    if no_key_errors:
                        detail = "; ".join(no_key_errors)
                        ok = False
                        status = CHECK_FAILED
                    elif self.args.github_source == "operator-unverified":
                        detail = ("GitHub login is not bound to the person's manifest; add "
                                  "identity.github to the manifest and run again")
                        ok = False
                        status = UNBOUND
                    else:
                        detail = (f"no attributable keys: GitHub login {self.args.github_login}, "
                                  "zero console rows, zero --pubkey-file keys, and zero GitHub keys")
                        ok = True
                        status = NOT_APPLICABLE
                elif not binding_errors and not count_errors:
                    ok = host_ssh_verified and after_console_clear
                    if not ok:
                        status = CHECK_FAILED
                        detail = ("host authorized_keys readback did not pass or console SSH "
                                  "state remains")
                    elif self.args.github_source == "operator-unverified":
                        status = UNBOUND
                        detail = ("GitHub login is not bound to the person's manifest; add "
                                  "identity.github to the manifest and run again")
                        ok = False
                    else:
                        status = CHECKED
                        detail = ("host authorized_keys readback passed and no active console "
                                  "keys remain")
            after_value = "absent" if ok or status == UNBOUND else "present-or-unknown"
            if (plane == "fabric-slot" and ok and self.teammates[goatnet] > 0
                    and self.structured_presence(after, "fabric_status") == "present"):
                after_value = "retained-for-teammates"
            value = outcome(status, before_values[plane],
                            after_value, detail)
            if plane == "host-ssh-keys" and ssh_evidence is not None:
                value.update({
                    "github_login": ssh_evidence.get("github_login"),
                    "github_login_source": ssh_evidence.get("github_login_source"),
                    "github_keys_read_ok": ssh_evidence.get("github_keys_read_ok"),
                    "keys_checked": ssh_evidence.get("keys_checked"),
                    "console_rows_any_status": ssh_evidence.get("console_rows_any_status"),
                    "pubkey_files": ssh_evidence.get("pubkey_files"),
                    "host_readback": ssh_evidence.get("host_readback"),
                    "readback_at": utcnow(),
                })
            self.record(goatnet, plane, self.audit_plane(goatnet, plane, value))
            if status == CHECK_FAILED:
                self.fail(f"{goatnet} {plane}: {detail}")

    def goatnet_before(self, goatnet: str) -> dict[str, str]:
        """What the pre-removal status read found on each per-goatnet plane."""
        before = self.before_status[goatnet]
        return {
            "host-ssh-keys": self.structured_presence(before, "ssh_status"),
            "operator-keys": summary_presence(before.get("operator-key") if before else None, ("none",)),
            "netbird-devices": ("absent" if before and before.get("mesh_status", {}).get("peer_ids") == [] else
                                "present" if before and isinstance(before.get("mesh_status", {}).get("peer_ids"), list) else "unknown"),
            "netbird-setup-keys": self.structured_presence(before, "setup_key_status"),
            "netbird-user": summary_presence(before.get("netbird-role") if before else None,
                                               ("no dashboard user", "(blocked)")),
            "fabric-slot": self.structured_presence(before, "fabric_status"),
            "sponsorship": self.structured_presence(before, "sponsorship_status"),
            "ory": ory_access(before.get("ory_status") if before else None),
        }

    def found_before(self) -> dict[tuple[str, str], str]:
        """Every row of the run and what the preflight read found there, before
        anything is removed. Team-wide planes are one row, not one per goatnet."""
        found = {}
        for goatnet in self.args.goatnets:
            found.update({(goatnet, plane): value
                          for plane, value in self.goatnet_before(goatnet).items()})
            for plane in ("collab-kanidm", "public-door-kanidm"):
                row = self.kanidm_before[(goatnet, plane)][1][1]
                gone = row is None or (plane == "public-door-kanidm" and Kanidm.is_expired(row))
                found[(goatnet, plane)] = "absent" if gone else "present"
        found[(TEAM, "onepassword")] = (
            "present" if self.onepassword_present(self.op_before) else "absent")
        if self.args.openbao:
            found[(TEAM, "openbao")] = "present" if self.bao_before else "absent"
        found[(TEAM, "github")] = "present" if self.github_present() else "absent"
        for identifier, before in self.account_before.items():
            found[(ACCOUNT, identifier)] = "present" if before["held"] else "absent"
        return found

    def confirm(self) -> bool:
        """Name the person and the counts, and take the handle typed back.
        --yes answers for a run with no terminal; without either, nothing is
        removed."""
        found = self.found_before()
        held = sum(value == "present" for value in found.values())
        handle = self.subjects[0][1]
        reason = self.args.reason + (f" ({self.args.reason_text})" if self.args.reason_text else "")
        print("\n==> confirm")
        print(f"    person:    {handle} <{self.email}>")
        print(f"    reason:    {reason}; confirmed by {self.args.confirmed_by}")
        if self.args.record_as and self.args.record_as != handle:
            print(f"    record:    the departure of {self.args.record_as}; {handle} is listed "
                  "as another handle of the same person")
        print(f"    goatnets:  {' '.join(self.args.goatnets)}")
        print(f"    access:    found in {held} of {len(found)} places; all {len(found)} are "
              "removed or read back")
        for row in self.receipt["by_hand"]:
            print(f"    by hand:   {row['id']}: {BY_HAND}")
        if self.args.yes:
            print("    confirmed by --yes")
            return True
        if not sys.stdin.isatty():
            self.fail("confirmation: no terminal to confirm on; pass --yes to remove "
                      f"{handle}'s access without the question")
            return False
        try:
            typed = input(f"    type the handle {handle} to remove this access: ").strip()
        except EOFError:
            typed = ""
        if typed != handle:
            self.fail("confirmation: the handle typed did not match; nothing was removed")
            return False
        return True

    def table_rows(self) -> list[dict]:
        """One row per plane per goatnet and one per team-wide plane: where,
        what, found, result, verified, and why when it failed."""
        rows, team_done = [], set()
        sources = [(goatnet, plane, planes[plane])
                   for goatnet, planes in self.receipt["goatnets"].items()
                   for plane in self.planes]
        sources += [(ACCOUNT, identifier, value)
                    for identifier, value in self.receipt["accounts"].items()]
        for goatnet, plane, value in sources:
            where = TEAM if goatnet != ACCOUNT and plane in TEAM_PLANES else goatnet
            if where == TEAM:
                if plane in team_done:
                    continue
                team_done.add(plane)
            status, before = value.get("status"), value.get("before")
            audit = {"recorded": True, "not recorded": False}.get(value.get("audit"))
            unaudited = audit is False and value.get("removal_verified") is True
            if unaudited:
                result = ("removed" if before == "present" else "none held") + ", audit not recorded"
            elif value.get("after") == "retained-for-teammates":
                result = "kept for teammates"
            elif status == UNBOUND:
                result = "removed, login unbound"
            elif status in VERIFIED_STATUSES:
                result = "removed" if before == "present" else "none held"
            elif status == NOT_RUN:
                result = "NOT RUN"
            else:
                result = "FAILED"
            row = {"where": where, "what": plane, "found": before, "result": result,
                   "verified": value.get("verified") is True or unaudited, "audit": audit}
            if not row["verified"] or unaudited:
                row["why"] = self.scrub(str(value.get("detail", "")))
            rows.append(row)
        rows += [{"where": ACCOUNT, "what": row["id"], "found": "not read",
                  "result": "do by hand", "verified": None, "audit": None, "why": BY_HAND}
                 for row in self.receipt["by_hand"]]
        return rows

    def scrub(self, text: str) -> str:
        """The record and the table name the person by handle, never by login."""
        email = getattr(self, "email", "")
        return re.sub(re.escape(email), "<login>", text, flags=re.I) if email else text

    def print_table(self, rows: list[dict]) -> None:
        print("\n==> result")
        widths = [max(len(str(row[key])) for row in [*rows, HEADER]) for key in COLUMNS]
        for row in [HEADER, *rows]:
            cells = [str(row[key]) if key not in ("verified", "audit") or row is HEADER
                     else {True: "yes", False: "NO", None: "-"}[row.get(key)] for key in COLUMNS]
            line = "  ".join(cell.ljust(width) for cell, width in zip(cells, widths))
            print("    " + (line + "  " + row.get("why", "")).rstrip())

    @staticmethod
    def structured_presence(value: dict, key: str) -> str:
        status = value.get(key)
        if not isinstance(status, dict) or status.get("read_ok") is not True:
            return "unknown"
        return "present" if status.get("present") is True else "absent"

    def kanidm_client(self, plane: str) -> Kanidm:
        if plane == "collab-kanidm":
            return Kanidm(os.environ["COLLAB_KANIDM_URL"], os.environ["COLLAB_KANIDM_IDM_ADMIN_PW"])
        return Kanidm(os.environ["PUBLIC_DOOR_KANIDM_URL"],
                      os.environ[public_door_admin_var(self.args.repo)])

    def kanidm(self, goatnet: str, plane: str) -> None:
        client, known = self.kanidm_before[(goatnet, plane)]
        try:
            before, after = client.remove(self.email, plane == "collab-kanidm", known)
            action = "absent" if plane == "collab-kanidm" else "absent or expired"
            value = outcome(CHECKED, before, after, f"account {action} and checked")
        except Exception as exc:
            before = "absent" if known[1] is None else "present"
            value = outcome(CHECK_FAILED, before, "unknown", str(exc))
            self.fail(f"{goatnet} {plane}: {exc}")
        self.record(goatnet, plane, self.audit_plane(goatnet, plane, value))

    @staticmethod
    def configured_list(name: str, default: str) -> list[str]:
        value = os.getenv(name, "").strip() or default
        result = [item.strip() for item in value.split(",") if item.strip()]
        if not result:
            raise RuntimeError(f"{name} does not name any targets")
        return result

    def onepassword_groups(self) -> list[tuple[str, str]]:
        proc = run(["op", "group", "list", "--format", "json"])
        try:
            rows = json.loads(proc.stdout) if proc.returncode == 0 else None
        except json.JSONDecodeError:
            rows = None
        if not isinstance(rows, list):
            raise RuntimeError("cannot list 1Password groups")
        groups = []
        for row in rows:
            if not isinstance(row, dict):
                raise RuntimeError("1Password group list contains a malformed row")
            identifier = str(row.get("id") or row.get("name") or "").strip()
            name = str(row.get("name") or identifier).strip()
            if not identifier:
                raise RuntimeError("1Password group list contains an unnamed group")
            groups.append((identifier, name))
        return groups

    def read_onepassword_groups(self) -> dict[str, tuple[str, list]]:
        return {identifier: (name, self.op_group(identifier, "during preflight"))
                for identifier, name in self.onepassword_groups()}

    def onepassword(self) -> dict:
        before_any = self.onepassword_present(self.op_before)
        for identifier, (name, users) in self.op_before.items():
            present = any(str(u.get("email", "")).casefold() == self.email.casefold()
                          for u in users if isinstance(u, dict))
            if present and run(["op", "group", "user", "revoke", "--group", identifier,
                                "--user", self.email]).returncode:
                raise RuntimeError(f"cannot remove user from 1Password group {name}")
            users = self.op_group(identifier, "after removal")
            if any(str(u.get("email", "")).casefold() == self.email.casefold()
                   for u in users if isinstance(u, dict)):
                raise RuntimeError(f"user remains in 1Password group {name}")
        after_groups = self.read_onepassword_groups()
        if self.onepassword_present(after_groups):
            raise RuntimeError("user appears in the post-removal 1Password group inventory")
        checked = [name for name, _ in after_groups.values()]
        return outcome(CHECKED, "present" if before_any else "absent", "absent",
                       "absent from every post-removal team group: " + ", ".join(checked))

    def onepassword_present(self, groups: dict[str, tuple[str, list]]) -> bool:
        return any(
            str(user.get("email", "")).casefold() == self.email.casefold()
            for _, users in groups.values() for user in users if isinstance(user, dict)
        )

    def op_group(self, group: str, when: str) -> list:
        proc = run(["op", "group", "user", "list", group, "--format", "json"])
        # The CLI prints `null`, not `[]`, for a group with no members. That is
        # a successful read of an empty group; anything else that is not a
        # list, or a non-zero exit, is a read that failed.
        unread = object()
        try:
            value = json.loads(proc.stdout) if proc.returncode == 0 else unread
        except json.JSONDecodeError:
            value = unread
        if value is None:
            return []
        if not isinstance(value, list):
            raise RuntimeError(f"cannot read 1Password group {group} {when}")
        return value

    def openbao_sessions(self) -> int:
        """Count the person's live store tokens (revoke-person's dry run)."""
        proc = run([str(self.args.repo / "ops/secrets/openbao/revoke-person"),
                    self.args.openbao_user, "--dry-run"])
        if proc.returncode:
            raise RuntimeError(proc.stderr.strip() or "revoke-person dry run failed")
        if "no entity alias named" in proc.stdout:
            return 0
        match = re.search(r": (\d+) token\(s\) would be revoked", proc.stdout)
        if not match:
            raise RuntimeError("cannot read revoke-person dry-run output")
        return int(match.group(1))

    def openbao(self) -> dict:
        if self.bao_before:
            proc = run([str(self.args.repo / "ops/secrets/openbao/revoke-person"),
                        self.args.openbao_user])
            if proc.returncode:
                raise RuntimeError(proc.stderr.strip() or "revoke-person failed")
        remaining = self.openbao_sessions()
        if remaining:
            raise RuntimeError(f"{remaining} store session(s) remain for {self.args.openbao_user}")
        return outcome(CHECKED, "present" if self.bao_before else "absent", "absent",
                       f"no live store session for {self.args.openbao_user}; next sign-in "
                       "is stopped by plane 10 (Kanidm group membership)")

    def github_list(self, org: str, kind: str) -> set[str]:
        proc = run(["gh", "api", "--paginate", f"orgs/{org}/{kind}?per_page=100", "--jq", ".[].login"])
        if proc.returncode:
            raise RuntimeError(f"cannot read GitHub {org} {kind}")
        return {line.strip().casefold() for line in proc.stdout.splitlines() if line.strip()}

    @staticmethod
    def github_rows(endpoint: str, label: str) -> list[dict]:
        proc = run(["gh", "api", "--paginate", endpoint, "--jq", ".[] | @json"])
        if proc.returncode:
            raise RuntimeError(f"cannot read GitHub {label}")
        rows = []
        for line in proc.stdout.splitlines():
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                raise RuntimeError(f"cannot read GitHub {label}: malformed response") from exc
            if not isinstance(row, dict):
                raise RuntimeError(f"cannot read GitHub {label}: malformed row")
            rows.append(row)
        return rows

    def github_invitations(self, org: str) -> tuple[list[dict], dict[str, list[dict]]]:
        org_rows = self.github_rows(
            f"orgs/{org}/invitations?per_page=100", f"{org} organisation invitations")
        repos = self.github_rows(f"orgs/{org}/repos?per_page=100", f"{org} repositories")
        repo_rows = {}
        for row in repos:
            repo = row.get("name")
            if not isinstance(repo, str) or not repo:
                raise RuntimeError(f"cannot read GitHub {org} repositories: malformed row")
            repo_rows[repo] = self.github_rows(
                f"repos/{org}/{repo}/invitations?per_page=100",
                f"{org}/{repo} repository invitations")
        return org_rows, repo_rows

    def github_invitation_matches(self, row: dict, repository: bool = False) -> bool:
        person = row.get("invitee") if repository else row
        login = person.get("login") if isinstance(person, dict) else None
        email = row.get("email")
        return ((isinstance(login, str)
                 and login.casefold() == self.args.github_login.casefold())
                or (not repository and isinstance(email, str)
                    and email.casefold() == self.email.casefold()))

    def matching_github_invitations(
            self, invitations: tuple[list[dict], dict[str, list[dict]]]
    ) -> list[tuple[str | None, object]]:
        matches = []
        org_rows, repositories = invitations
        for location, rows in [(None, org_rows), *repositories.items()]:
            for row in rows:
                if self.github_invitation_matches(row, location is not None):
                    identifier = row.get("id")
                    if not isinstance(identifier, (str, int)) or isinstance(identifier, bool):
                        raise RuntimeError(
                            f"GitHub invitation in {location or 'organisation'} has no valid id")
                    matches.append((location, identifier))
        return matches

    def github_orgs(self) -> list[str]:
        # The organisations to remove the person from: GOAT_OFFBOARD_GITHUB_ORGS,
        # else what the checkout declares as goat_offboard_github_orgs. With
        # neither, the plane names no target and fails the run.
        return list(dict.fromkeys(self.configured_list(
            "GOAT_OFFBOARD_GITHUB_ORGS", declared_value(self.args.repo, "goat_offboard_github_orgs"))))

    def read_github_orgs(self) -> dict[
            str, tuple[set[str], set[str], tuple[list[dict], dict[str, list[dict]]]]
    ]:
        return {org: (self.github_list(org, "members"),
                      self.github_list(org, "outside_collaborators"),
                      self.github_invitations(org))
                for org in self.github_orgs()}

    def github_enterprises(self) -> list[str]:
        # The enterprises whose teams can grant organisation membership:
        # GOAT_OFFBOARD_GITHUB_ENTERPRISES, else the checkout's declared
        # goat_offboard_github_enterprises. None declared is allowed: such a
        # membership then makes the organisation removal fail, and the error
        # names this setting.
        value = (os.getenv("GOAT_OFFBOARD_GITHUB_ENTERPRISES", "").strip()
                 or declared_value(self.args.repo, "goat_offboard_github_enterprises") or "")
        return list(dict.fromkeys(item.strip() for item in value.split(",") if item.strip()))

    def github_team_members(self, enterprise: str, team: str) -> set[str]:
        rows = self.github_rows(f"enterprises/{enterprise}/teams/{team}/memberships?per_page=100",
                                f"enterprise {enterprise} team {team} members")
        return {row["login"].casefold() for row in rows if isinstance(row.get("login"), str)}

    def read_github_enterprise_teams(self) -> dict[str, dict[str, set[str]]]:
        teams: dict[str, dict[str, set[str]]] = {}
        for enterprise in self.github_enterprises():
            try:
                rows = self.github_rows(f"enterprises/{enterprise}/teams?per_page=100",
                                        f"enterprise {enterprise} teams")
            except RuntimeError as exc:
                raise RuntimeError(
                    f"{exc}: the signed-in GitHub account must be an owner of enterprise "
                    f"{enterprise}, and its token must hold admin:enterprise "
                    "(gh auth refresh -s admin:enterprise)") from exc
            teams[enterprise] = {}
            for row in rows:
                slug = row.get("slug")
                if not isinstance(slug, str) or not slug:
                    raise RuntimeError(f"cannot read GitHub enterprise {enterprise} teams: malformed row")
                teams[enterprise][slug] = self.github_team_members(enterprise, slug)
        return teams

    def github_enterprise_holdings(self, teams: dict[str, dict[str, set[str]]]) -> list[tuple[str, str]]:
        login = self.args.github_login.casefold()
        return [(enterprise, team) for enterprise, rows in teams.items()
                for team, members in rows.items() if login in members]

    def github(self) -> dict:
        login = self.args.github_login.casefold()
        orgs = self.github_orgs()
        held_teams = self.github_enterprise_holdings(self.github_teams_before)
        before_any = bool(held_teams)
        # Enterprise teams first: while one grants organisation membership,
        # an organisation owner cannot remove it (GitHub answers 403).
        for enterprise, team in held_teams:
            endpoint = f"enterprises/{enterprise}/teams/{team}/memberships/{self.args.github_login}"
            if run(["gh", "api", "-X", "DELETE", endpoint]).returncode:
                raise RuntimeError(f"cannot remove {self.args.github_login} from enterprise "
                                   f"{enterprise} team {team}")
        if held_teams:
            run(["sleep", "3"])
        for enterprise, team in held_teams:
            if login in self.github_team_members(enterprise, team):
                raise RuntimeError(f"GitHub enterprise {enterprise} team {team} membership remains")
        for org in orgs:
            members, outsiders, invitations = self.github_before[org]
            matches = self.matching_github_invitations(invitations)
            before_any |= login in members or login in outsiders or bool(matches)
            if login in members:
                if run(["gh", "api", "-X", "DELETE", f"orgs/{org}/memberships/{self.args.github_login}"]).returncode:
                    raise RuntimeError(
                        f"cannot remove GitHub membership from {org}; if an enterprise team "
                        "grants it, declare goat_offboard_github_enterprises and run again")
                run(["sleep", "3"])
            outsiders = self.github_list(org, "outside_collaborators")
            if login in outsiders:
                if run(["gh", "api", "-X", "DELETE", f"orgs/{org}/outside_collaborators/{self.args.github_login}"]).returncode:
                    raise RuntimeError(f"cannot remove GitHub repository grants from {org}")
                run(["sleep", "3"])
            for location, identifier in matches:
                endpoint = (f"orgs/{org}/invitations/{identifier}" if location is None
                            else f"repos/{org}/{location}/invitations/{identifier}")
                if run(["gh", "api", "-X", "DELETE", endpoint]).returncode:
                    target = org if location is None else f"{org}/{location}"
                    raise RuntimeError(f"cannot remove GitHub invitation from {target}")
            if login in self.github_list(org, "members"):
                raise RuntimeError(f"GitHub membership remains in {org}")
            if login in self.github_list(org, "outside_collaborators"):
                raise RuntimeError(f"GitHub repository grants remain in {org}")
            remaining = self.matching_github_invitations(self.github_invitations(org))
            if remaining:
                locations = ", ".join(org if location is None else f"{org}/{location}"
                                      for location, _ in remaining)
                raise RuntimeError(f"GitHub invitation remains in {locations}")
        status = UNBOUND if self.args.github_source == "operator-unverified" else CHECKED
        detail = ("absent from organisations, direct repository grants, and pending invitations"
                  + (", and from enterprise teams" if self.github_enterprises() else ""))
        if status == UNBOUND:
            detail += ("; GitHub login is not bound to the person's manifest; add "
                       "identity.github to the manifest and run again")
        return outcome(status, "present" if before_any else "absent", "absent", detail)

    def github_present(self) -> bool:
        login = self.args.github_login.casefold()
        return bool(self.github_enterprise_holdings(self.github_teams_before)) or any(
            login in members or login in outsiders
            or bool(self.matching_github_invitations(invitations))
            for members, outsiders, invitations in self.github_before.values())

    def execute(self, subjects: list[tuple[str, str, str, str | None]]) -> int:
        self.subjects = subjects
        self.email = subjects[0][2]
        print(f"\n==> offboard {self.args.subject} — goatnets:{' '.join(self.args.goatnets)} — APPLY")
        print(f"    receipt: {self.args.receipt}")
        self.save()
        try:
            self.org_prefixes = {
                goatnet: manifest_org_prefix(self.args.repo, goatnet, handle)
                for goatnet, handle, _, _ in subjects
            }
        except RuntimeError as exc:
            self.fail(f"preflight: {exc}")
            return 1
        self.teammates = {
            goatnet: active_tag_teammates(self.args.repo, goatnet, handle,
                                          self.org_prefixes[goatnet])
            for goatnet, handle, _, _ in subjects
        }
        self.receipt["shared_credentials"] = shared_credentials(
            self.teammates, self.org_prefixes)
        if not self.preflight():
            return 1
        if not self.confirm():
            return 1
        try:
            self.departure = offboard_departure.begin(
                self.args.departure_dir, self.args.record_as or subjects[0][1], self.args.reason,
                self.args.reason_text, self.args.effective, self.args.confirmed_by,
                self.args.operator, self.args.goatnets,
                [(row["id"], row["what"]) for row in self.receipt["by_hand"]],
                [account["id"] for account in self.args.accounts],
                also=subjects[0][1])
        except offboard_departure.Refusal as exc:
            self.fail(f"departure record: {exc}")
            return 1
        print(f"    departure record: {self.departure[0]}")
        for goatnet, handle, email, _ in subjects:
            print(f"\n==> goatnet {goatnet}\n    resolved handle: {handle}; login email: {email}")
            self.goatnet(goatnet, handle, email)
            self.kanidm(goatnet, "collab-kanidm")
            self.kanidm(goatnet, "public-door-kanidm")
        team_wide = [("onepassword", self.onepassword), ("github", self.github)]
        if self.args.openbao:
            team_wide.insert(1, ("openbao", self.openbao))
        for plane, action in team_wide:
            try:
                value = action()
            except Exception as exc:
                before_present = {
                    "onepassword": lambda: self.onepassword_present(self.op_before),
                    "openbao": lambda: bool(getattr(self, "bao_before", 0)),
                    "github": self.github_present,
                }[plane]()
                value = outcome(CHECK_FAILED, "present" if before_present else "absent",
                                "unknown", str(exc))
                self.fail(f"{plane}: {exc}")
            for goatnet in self.args.goatnets:
                self.record(goatnet, plane, self.audit_plane(goatnet, plane, dict(value)))
        for account in offboard_accounts.automated(self.args.accounts):
            self.account(account)
        verified = all(
            value.get("verified") is True
            for rows in [*self.receipt["goatnets"].values(), self.receipt["accounts"]]
            for value in rows.values()
        )
        return 0 if not self.errors and verified else 1

    def finish(self, code: int) -> int:
        self.receipt["finished_at"] = utcnow()
        statuses = [value.get("status")
                    for rows in [*self.receipt["goatnets"].values(), self.receipt["accounts"]]
                    for value in rows.values()]
        if code == 0 and all(status in VERIFIED_STATUSES for status in statuses):
            self.receipt["outcome"] = "success"
        elif UNBOUND in statuses and not any(status in (CHECK_FAILED, NOT_RUN)
                                             for status in statuses):
            self.receipt["outcome"] = "incomplete"
        else:
            self.receipt["outcome"] = "failed"
        if self.errors:
            self.receipt["errors"] = self.errors
        for goatnet, rows in self.receipt["goatnets"].items():
            for plane, value in rows.items():
                if "how" not in value:
                    self.record(goatnet, plane, value)
        self.save()
        rows = self.table_rows()
        departure = getattr(self, "departure", None)
        unaudited = sum(1 for row in rows if row.get("audit") is False)
        if departure and unaudited:
            by_hand = departure[1].setdefault("by_hand", [])
            if not any(item.get("id") == AUDIT_ROW for item in by_hand):
                by_hand.append({
                    "id": AUDIT_ROW, "state": "open",
                    "what": (f"{unaudited} operator-action audit entries did not reach the fabric; "
                             "they are in the local fallback log of the computer that ran the "
                             "driver. Re-emit them with goat audit emit, then close this row")})
        if departure:
            try:
                value = offboard_departure.finish(
                    departure[0], departure[1], self.args.operator,
                    self.receipt["outcome"], rows)
                self.receipt["departure_record"] = str(departure[0])
                self.save()
            except offboard_departure.Refusal as exc:
                self.fail(f"departure record: {exc}")
                self.receipt.update({"outcome": "failed", "errors": self.errors})
                self.save()
                code, departure = 1, None
        self.print_table(rows)
        print_shared_credentials(self.receipt["shared_credentials"])
        if departure:
            waiting = [row for row in value.get("by_hand") or [] if row.get("state") != "done"]
            print(f"\n==> departure record ({value['state']}): {departure[0]}")
            for row in waiting:
                print(f"    by hand, open: {row['id']}: {row['what']}")
            if waiting:
                print("    close each with: python3 ops/scripts/offboard_departure.py close "
                      f"{value['handle']} <row> --by <handle>")
        print("\n==> verdict")
        if self.receipt["outcome"] == "success":
            print("    every plane ran and read back empty")
        elif self.receipt["outcome"] == "incomplete":
            print("    INCOMPLETE — a removed login is not bound to the person's manifest",
                  file=sys.stderr)
        else:
            print("    NOT OFFBOARDED — one or more planes did not run or did not verify", file=sys.stderr)
        print(f"    receipt: {self.args.receipt}")
        return code


ROLE_ORDER = ("dev", "trial", "pre-prod", "production")


def declared_goatnets(repo):
    """Goatnet ids from ops/goatnets/cross-env.yaml, least to most production."""
    path = Path(repo) / "ops" / "goatnets" / "cross-env.yaml"
    try:
        doc = offboard_subject.yaml.load(path.read_text(encoding="utf-8"),
                                         Loader=offboard_subject.yaml.SafeLoader) or {}
    except (OSError, UnicodeError, offboard_subject.yaml.YAMLError) as exc:
        sys.exit(f"--all-goatnets reads {path}: {exc}")
    rows = list((doc.get("goatnets") or {}).values())
    rank = {role: i for i, role in enumerate(ROLE_ORDER)}
    rows.sort(key=lambda row: rank.get(row.get("role"), len(ROLE_ORDER)))
    return [row["id"] for row in rows]


def main() -> int:
    repo_proc = run(["git", "rev-parse", "--show-toplevel"])
    if repo_proc.returncode:
        print("offboard: REFUSE — run inside the goat repository", file=sys.stderr)
        return 1
    repo = Path(repo_proc.stdout.strip())
    parser = argparse.ArgumentParser(
        description=("One fail-closed driver removes a person from every access plane on "
                     "every selected goatnet. A plane it cannot run or cannot verify fails "
                     "the run."),
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""examples:
  ops/scripts/offboard.sh <email-or-handle> --goatnet <name>
  ops/scripts/offboard.sh <email-or-handle> --all-goatnets --apply \\
    --reason left-service --confirmed-by <handle>
  ops/scripts/offboard.sh <email-or-handle> --goatnet <name> --apply \\
    --github-user <login> --github-user-unverified

ordered planes:
  1. host-ssh-keys          7. sponsorship
  2. operator-keys          8. ory
  3. netbird-devices        9. collab-kanidm
  4. netbird-setup-keys    10. public-door-kanidm
  5. netbird-user          11. onepassword
  6. fabric-slot           12. openbao (only with BAO_ADDR or --openbao-user)
                           13. github

apply credentials and environment:
  goat contexts <goatnet>-op; signed-in gh and op; COLLAB_KANIDM_URL;
  COLLAB_KANIDM_IDM_ADMIN_PW; PUBLIC_DOOR_KANIDM_URL;
  GOAT_IDM_ADMIN_PW (or the variable the checkout names as
  public_door_idm_admin_pw_env); GOAT_OFFBOARD_GITHUB_ORGS (or the
  checkout's goat_offboard_github_orgs); GOAT_OFFBOARD_GITHUB_ENTERPRISES
  (or goat_offboard_github_enterprises; gh signed in as an enterprise
  owner with admin:enterprise);
  BAO_ADDR plus a store token and --openbao-user for the openbao plane;
  GOAT_OPERATOR_HANDLE; GOAT_PROFILE_DIR.

The driver sends no notice to the person.

the departure record (ADR 1272):
  --apply needs --reason and --confirmed-by. Before the first removal the
  driver names the person and the counts and takes the handle typed back
  (--yes answers for a run with no terminal), then writes the record under
  ops/departures/: handle, reason, who confirmed, and one result row per
  plane at the finish. It holds no login email and no credential.

external accounts (ADR 1272):
  ops/offboard/accounts.yaml declares the accounts outside the goatnets.
  Each is a plug-in (ops/offboard/plugins/<type>: check, remove) or type
  manual. A declared plug-in that cannot run fails the run. A manual account
  is listed as "not automated: do by hand" and stays open in the record.

what it does NOT do (ADR 1187 section 7):
  It does not delete the Ory identity; --purge remains a separate decision
  under ADR 1031. It does not touch machine custody under ADR 1096. It does
  not rotate shared credentials, but reports them for separate assessment.""",
    )
    parser.add_argument("subject", help="email address or manifest handle to offboard")
    parser.add_argument("--goatnet", action="append", dest="goatnets", default=[],
                        help="select one goatnet; repeat to select more than one")
    parser.add_argument("--all-goatnets", action="store_true",
                        help="select every goatnet ops/goatnets/cross-env.yaml declares")
    parser.add_argument("--apply", action="store_true",
                        help="perform removals; without this flag, only print the plan")
    parser.add_argument("--reason", default="", choices=("", *offboard_departure.REASONS),
                        help=("why the person left, required with --apply: new-position "
                              "(moved to another military or government position), "
                              "left-service, or other (with --reason-text)"))
    parser.add_argument("--reason-text", default="",
                        help="one line saying why; required with --reason other")
    parser.add_argument("--confirmed-by", default="",
                        help=("handle of the person who confirmed the departure; "
                              "required with --apply"))
    parser.add_argument("--effective", default=None,
                        help="the day the departure took effect, YYYY-MM-DD (default: today)")
    parser.add_argument("--record-as", default=None, metavar="HANDLE",
                        help=("write this run into the open departure record of HANDLE, the "
                              "same person's other handle; this run's handle is listed under "
                              "other_handles (one record per person)"))
    parser.add_argument("--yes", action="store_true",
                        help="remove without the typed confirmation")
    parser.add_argument("--departure-dir", type=Path, default=None,
                        help=("where departure records are kept (default: "
                              f"{offboard_departure.DIRECTORY} in this checkout)"))
    parser.add_argument("--account-handle", action="append", default=[],
                        metavar="ID=HANDLE",
                        help=("the person's handle in a declared external account that "
                              "matches by declared handle; ID=- states they hold none; "
                              "repeatable"))
    parser.add_argument("--github-user", default="",
                        help=("cross-check the manifest's GitHub login, or supply it when "
                              "used with --github-user-unverified"))
    parser.add_argument("--github-user-unverified", action="store_true",
                        help=("acknowledge that --github-user is operator-supplied because "
                              "no selected manifest names identity.github"))
    parser.add_argument("--openbao-user", default="",
                        help=("the person's sign-in name in the team credential store "
                              "(ADR 1203); required when BAO_ADDR is set"))
    parser.add_argument("--operator", default=os.getenv("GOAT_OPERATOR_HANDLE", ""),
                        help="kebab-case operator handle for per-plane audit attribution")
    parser.add_argument("--pubkey-file", action="append", default=[],
                        help="additional public-key file to sweep from hosts; repeatable")
    parser.add_argument("--receipt", type=Path,
                        help=("override the receipt path (default: "
                              "$GOAT_PROFILE_DIR/offboard/<subject>-<stamp>.json, or "
                              "~/.goat/offboard/<subject>-<stamp>.json)"))
    args = parser.parse_args()
    if args.all_goatnets:
        args.goatnets = declared_goatnets(repo)
    args.goatnets = list(dict.fromkeys(args.goatnets))
    if not args.goatnets:
        parser.error("--goatnet <g> or --all-goatnets is required")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    safe = "".join(c if c.isalnum() or c in "._-" else "_" for c in args.subject)
    args.default_receipt = args.receipt is None
    profile_value = os.getenv("GOAT_PROFILE_DIR", "").strip()
    profile_dir = Path(profile_value).expanduser() if profile_value else Path.home() / ".goat"
    args.receipt = args.receipt or profile_dir / "offboard" / f"{safe}-{stamp}.json"
    args.repo = repo
    args.pubkey_args = [part for path in args.pubkey_file for part in ("--pubkey-file", path)]
    args.github_login = args.github_user
    args.openbao = bool(os.getenv("BAO_ADDR") or args.openbao_user)
    args.github_source = "unresolved"
    args.departure_dir = args.departure_dir or repo / offboard_departure.DIRECTORY
    try:
        if args.apply or args.reason:
            offboard_departure.check_reason(args.reason, args.reason_text)
        if args.apply:
            offboard_departure.check_confirmer(
                args.confirmed_by, declared_value(repo, "goat_offboard_confirmers"))
        if args.effective is not None:
            offboard_departure.check_effective(args.effective)
        if args.record_as is not None:
            offboard_departure.check_record_as(args.departure_dir, args.record_as)
        args.accounts = offboard_accounts.load(repo)
        args.account_handles = offboard_accounts.parse_supplied(args.account_handle)
        unknown = sorted(set(args.account_handles) - {a["id"] for a in args.accounts})
        if unknown:
            raise offboard_accounts.Refusal(
                "--account-handle names no declared account: " + ", ".join(unknown))
    except (offboard_departure.Refusal, offboard_accounts.Refusal) as exc:
        print(f"offboard: REFUSE — {exc}", file=sys.stderr)
        return 1
    try:
        subjects = offboard_subject.resolve(repo / "ops/operators", args.subject, args.goatnets)
    except offboard_subject.Refusal as exc:
        if not args.apply:
            print(f"offboard: REFUSE — {exc}", file=sys.stderr)
            return 1
        driver = Driver(args)
        driver.fail(f"subject resolution: {exc}")
        return driver.finish(1)
    try:
        args.github_login, args.github_source = offboard_subject.resolve_github(
            subjects, args.github_user, args.github_user_unverified, repo / "ops/operators")
    except offboard_subject.Refusal as exc:
        args.github_login = exc.github_login or ""
        args.github_source = exc.github_source
        if not args.apply:
            print(f"offboard: REFUSE — {exc}", file=sys.stderr)
            return 1
        driver = Driver(args)
        driver.fail(f"GitHub login resolution: {exc}")
        return driver.finish(1)
    if not args.apply:
        print(f"\n==> offboard {args.subject} — goatnets:{' '.join(args.goatnets)} — PLAN ONLY")
        print(f"    receipt: {args.receipt} (not created in plan mode)")
        try:
            org_prefixes = {
                goatnet: manifest_org_prefix(repo, goatnet, handle)
                for goatnet, handle, _, _ in subjects
            }
        except RuntimeError as exc:
            print(f"offboard: REFUSE — {exc}", file=sys.stderr)
            return 1
        teammates = {
            goatnet: active_tag_teammates(repo, goatnet, handle, org_prefixes[goatnet])
            for goatnet, handle, _, _ in subjects
        }
        print(f"    GitHub login: {args.github_login} ({args.github_source})")
        if args.github_source == "operator-unverified":
            print("    INCOMPLETE: this unverified login cannot produce a successful offboarding; "
                  "add identity.github to the manifest and run again")
        for goatnet, handle, email, _ in subjects:
            print(f"    {goatnet}: {handle} <{email}>")
            print(f"    planes on {goatnet}:")
            for number, plane in enumerate(PLANES[:10], 1):
                print(f"      {number:2}. {plane}")
        print("    team-wide planes:")
        for number, plane in enumerate((STORE_PLANES if args.openbao else PLANES)[10:], 11):
            print(f"      {number:2}. {plane}")
        print("    external accounts"
              + (":" if args.accounts else
                 f": none declared ({offboard_accounts.DECLARATION} is absent or empty)"))
        for account in args.accounts:
            if account["type"] == "manual":
                print(f"      {account['id']}: {BY_HAND}: {account['what']}")
                continue
            reason = offboard_accounts.cannot_run(account, repo)
            print(f"      {account['id']}: plug-in {account['type']}, matched by "
                  f"{account['match']}" + (f" (CANNOT RUN: {reason})" if reason else ""))
        print_shared_credentials(shared_credentials(teammates, org_prefixes))
        print(f"\n==> departure record: written under {args.departure_dir} before the "
              "first removal")
        if args.reason:
            print(f"    reason: {args.reason}; the person may return by: "
                  f"{offboard_departure.RETURN_RULE[args.reason]}")
            for identifier, what in offboard_departure.FOLLOW_ON[args.reason]:
                print(f"    by hand, kept open in the record: {identifier}: {what}")
        print("    re-run with --apply --reason <class> --confirmed-by <handle>; every "
              "plane must run and verify")
        return 0
    driver = Driver(args)
    code = 1
    try:
        code = driver.execute(subjects)
    except Exception as exc:
        driver.fail(f"unexpected driver failure: {exc}")
    return driver.finish(code)


if __name__ == "__main__":
    raise SystemExit(main())
