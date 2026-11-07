#!/usr/bin/env python3
"""acceptance-identity.py — create a goatnet's goat-acceptance sponsor identity (ADR 1207).

The candidate-intake acceptance rows run as this non-person identity: a portal
org whose sponsorship is capped at browse, with no operator power. Its context
`acceptance` lives on the management host the rows run `goat` on.

usage:
  acceptance-identity.py <env> <admin-ssh> <admin-context> <target-ssh> [--port N]
  acceptance-identity.py --print-config <env>   the zone and 1Password account a run uses

  <admin-ssh>      host that makes the one operator call (POST /api/orgs:
                   create, approve, sponsorship=browse)
  <admin-context>  a goat context holding an operator key: `<name>` on
                   <admin-ssh>, or `local:<name>` for one on this laptop, whose
                   token then rides that single request over ssh stdin and is
                   stored nowhere on the host
  <target-ssh>     host that gets ~/.goat/contexts/acceptance
  --port N         ssh port for a door (env-door.sh <env> --ssh-target)

The new key is never printed or written to this machine's disk: it goes to
1Password (goat-site-services/<env>-acceptance-sponsor, field api_key) and
over ssh stdin to the target. Output shows its length and the
identity's whoami (the proof the key works), which must read operator=false, sponsor_max_tier=browse.

Rotate: goat admin rotate goat-acceptance --key-only, then re-run the install
half. Cut off at once: goat admin sponsorship goat-acceptance --revoke.
"""
import importlib.util, json, os, shlex, subprocess, sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# Examples for a tree whose group_vars declare no site_dns_zone.
EXAMPLE_ZONES = {"zagros-dev": "netbird.zagros-dev.example", "zagros": "netbird.zagros.example"}
EXAMPLE_OP_ACCOUNT = "example.1password.com"


def site_dns_zone(goatnet):
    """The goatnet's site_dns_zone from its group_vars, else the example's."""
    r = subprocess.run([sys.executable, os.path.join(REPO, "ops/scripts/goatnet-facts.py"), "var", goatnet, "site_dns_zone"],
                       capture_output=True, text=True)
    zone = r.stdout.strip() if r.returncode == 0 else ""
    if zone and "{{" not in zone:
        return zone
    if goatnet in EXAMPLE_ZONES:
        return EXAMPLE_ZONES[goatnet]
    sys.exit(f"acceptance-identity: ops/ansible/group_vars/{goatnet.replace('-', '_')}_hosts declares no site_dns_zone")


def op_account():
    """The account the secrets posture pins (OP_ACCOUNT, else `secrets: op_account:`
    in ops/goatbuilder/goatbuilder.yaml, ADR 1204), else the example's."""
    spec = importlib.util.spec_from_file_location("goat_posture", os.path.join(REPO, "ops/secrets/posture.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.op_account(REPO) or EXAMPLE_OP_ACCOUNT


if __name__ == "__main__" and sys.argv[1:2] == ["--print-config"]:
    # The values a run on <env> would use, without contacting anything.
    print(json.dumps({"zone": site_dns_zone(sys.argv[2]), "op_account": op_account()}))
    sys.exit(0)

env, admin, admin_ctx, target = sys.argv[1:5]
port = sys.argv[sys.argv.index("--port") + 1] if "--port" in sys.argv else ""
zone = site_dns_zone(env)
op_acct = op_account()  # resolved before anything is created
portal = f"https://portal.{env}.{zone}"
onboard = f"https://onboard.{env}.{zone}"
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
       "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]

def ssh(host, cmd, data=None, p=""):
    h = host
    args = SSH + (["-p", p] if p else [])
    r = subprocess.run(args + [h, cmd], input=data, capture_output=True, text=True)
    return r

body = json.dumps({"prefix": "goat-acceptance",
                   "display_name": "Acceptance checks (non-person, ADR 1207)",
                   "auto_approve": True, "sponsor_authorized": True,
                   "sponsor_max_tier": "browse"})
if admin_ctx.startswith("local:"):
    # The operator token is this laptop's; the host only carries the one
    # request. Header from stdin, so it is in no argv and no file there.
    with open(os.path.expanduser(f"~/.goat/contexts/{admin_ctx[6:]}/portal.token")) as f:
        tok = f.read().strip()
    r = ssh(admin, f"curl -s -w '\\n%{{http_code}}' -X POST -H 'Content-Type: application/json' "
                   f"-H @/dev/stdin --data-binary {shlex.quote(body)} {portal}/api/orgs",
            f"Authorization: Bearer {tok}\n", port)
    del tok
else:
    r = ssh(admin, f'curl -s -w "\\n%{{http_code}}" -X POST -H "Content-Type: application/json" '
                   f'-H "Authorization: Bearer $(cat ~/.goat/contexts/{admin_ctx}/portal.token)" '
                   f'--data-binary @- {portal}/api/orgs', body, port if admin == target else "")
out = r.stdout.rsplit("\n", 1)
code = out[-1].strip() if out else ""
if code != "201" and code != "200":
    print(f"create refused: HTTP {code}: {out[0][:300] if out else r.stderr[:300]}")
    sys.exit(1)
resp = json.loads(out[0])
key = resp.get("api_key", "")
if not key:
    print("create returned no api_key:", {k: v for k, v in resp.items() if k != "api_key"})
    sys.exit(1)
print(f"created: slot={resp.get('slot_id') or resp.get('prefix')} handle={resp.get('system_handle')} "
      f"approved={resp.get('approved')} key len={len(key)}")

# 1Password: template through a pipe, so the value is in no argv and no file.
tmpl = json.dumps({"title": f"{env}-acceptance-sponsor", "category": "API_CREDENTIAL",
                   "fields": [{"id": "api_key", "label": "api_key", "type": "CONCEALED", "value": key},
                              {"id": "notesPlain", "label": "notesPlain", "type": "STRING", "purpose": "NOTES",
                               "value": f"ADR 1207: the goat-acceptance non-person sponsor identity on {env} "
                                        f"(browse-capped sponsorship, no operator power). Installed as goat context "
                                        f"'acceptance' on the host the live rows run goat on."}]})
rfd, wfd = os.pipe()
os.write(wfd, tmpl.encode()); os.close(wfd)
envp = {k: v for k, v in os.environ.items() if k != "GCP_HEIMDALL_OP_ITEM"}
envp["OP_ACCOUNT"] = op_acct
op = subprocess.run(["op", "item", "create", "--vault", "goat-site-services",
                     "--template", f"/dev/fd/{rfd}", "--format", "json"],
                    pass_fds=(rfd,), capture_output=True, text=True, env=envp)
os.close(rfd)
print("1password:", "stored" if op.returncode == 0 else f"FAILED {op.stderr.strip()[:200]}")

# The context on the target host: profile + token, 0700/0600.
profile = (f'# goat context "acceptance" — the goat-acceptance non-person sponsor identity\n'
           f'# (ADR 1207). Browse-capped sponsorship; holds no operator power. Its key\n'
           f'# lives in 1Password goat-site-services/{env}-acceptance-sponsor.\n'
           f'portal_endpoint = "{portal}"\n'
           f'onboard_endpoint = "{onboard}"\n'
           f'device_id = "goat-acceptance-{env}"\n'
           f'goatnet = "{env}"\n')
install = ('umask 077; d=~/.goat/contexts/acceptance; mkdir -p "$d"; '
           'IFS= read -r tok; printf %s "$tok" > "$d/portal.token"; cat > "$d/profile.toml"; '
           'chmod 600 "$d/portal.token" "$d/profile.toml"; ls -l "$d" | tail -n +2 | wc -l')
r = ssh(target, install, key + "\n" + profile, port)
print("host context:", "installed" if r.returncode == 0 else f"FAILED {r.stderr.strip()[:200]}")

# Verify the key is what ADR 1207 says: a sponsor at browse, not an operator.
r = ssh(target, f'curl -s -H "Authorization: Bearer $(cat ~/.goat/contexts/acceptance/portal.token)" {portal}/api/whoami', None, port)
try:
    w = json.loads(r.stdout)
    print("whoami:", {k: w.get(k) for k in ("operator", "sponsor_authorized", "sponsor_max_tier", "display_name")})
except Exception:
    print("whoami unreadable:", r.stdout[:200], r.stderr[:200])
