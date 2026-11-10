#!/usr/bin/env python3
"""generate-door-inventory.py — establish a goatnet's env-door from the registry.

A goatnet is deployable off-profile (through its door, without flipping the
laptop's shared netbird profile) only once it appears in the door inventories.
Those are hand-maintained, so a fresh goatnet had none, and every deploy to it
needed a hand-rolled ad-hoc inventory. This closes that: it reads the door
registry (ops/ansible/inventory/doors.yaml) and, for a goatnet whose door is not
yet in the inventories, appends its blocks to the two door inventories
(doors.ini, doors-groupvars.ini). goat-standup.sh runs it as a standup phase, and
the finish gate asserts the block is present, so a fresh goatnet is
door-reachable by construction.

env-door.sh is not one of the artifacts: it reads an ssm door's facts from the
registry each time it runs (`--arm`), so the script names no goatnet and nothing
is rendered into it.

Contract: ADD, never reformat. If a goatnet's block is already present (the
legacy hand-written entries, with their own comments and group splits) it is
left exactly as is; only a goatnet missing from a file gets its generated block
appended. So the tool is safe to run against the hand-owned files and `--check`
gates on presence, not byte-equality.

Usage:  generate-door-inventory.py <goatnet>           # establish its door if missing
        generate-door-inventory.py <goatnet> --check    # nonzero if its door is missing
        generate-door-inventory.py --all [--check]       # every goatnet in the registry
        generate-door-inventory.py --arm <goatnet>       # print its ssm door facts as KEY=value
        generate-door-inventory.py --self-test

--arm is what env-door.sh reads for every goatnet: in an assembled tree
(goatcore + an environment overlay) env-door.sh is goatcore's and the registry
is the overlay's.
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
REG = REPO / "ops/ansible/inventory/doors.yaml"
DOORS_INI = REPO / "ops/ansible/inventory/doors.ini"
GROUPVARS_INI = REPO / "ops/ansible/inventory/doors-groupvars.ini"

SSH_TAIL = "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ControlMaster=no -o ControlPath=none"


def _load_registry(text=None):
    import yaml
    data = yaml.safe_load(text if text is not None else REG.read_text())
    # `doors:` with no entries loads as None, not {} -- the shipped public
    # registry starts empty.
    return (data or {}).get("doors") or {}


def _resolve_door(env, doors):
    """Resolve a goatnet to its door. A FOUNDER has a top-level door of its own.
    A JOINER (ADR 1084, goatnet_founder: false) has NO door of its own — it is
    reached through its FOUNDER's door, where its nodes are listed with
    gv_group: <env>_hosts / <env>_obs_hosts. Resolve that here so a joiner site
    passed on the command line finds the shared door instead of failing closed."""
    if env in doors:
        return doors[env]
    pfx = env.replace("-", "_") + "_"
    for door in doors.values():
        if any(str(h.get("gv_group", "")).startswith(pfx) for h in door.get("mgmt", [])):
            return door
    return None


def _proxy_args(user, port):
    """ssh_common_args for a mgmt host that JUMPS through the landed door host.
    A ProxyCommand (not ProxyJump) so the inner ssh inherits the -o flags — a
    ProxyJump spawns an ssh that does not, and its host-key check hangs."""
    inner = f'ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -W %h:%p -p {port} {user}@127.0.0.1'
    return f'-o ProxyCommand=\\"{inner}\\" {SSH_TAIL}'


def _host_line(door, host, *, connection):
    """One inventory host line. `connection` adds ansible_connection=ssh, which
    doors-groupvars.ini carries (it layers over the base inventory's group) and
    doors.ini does not."""
    user = door["user"]
    conn = "ansible_connection=ssh " if connection else ""
    # router_role travels in the service-deploy (group_vars) block so a goatnet
    # not in the base mgmt-hosts.ini (a tailscale door used standalone) still
    # satisfies the ADR-1084 §D3 admin-write guard, which reads it. doors.ini
    # host-level plays do not need it.
    role = host.get("router_role")
    role_kv = f'router_role={role} ' if (connection and role) else ""
    if door["type"] == "direct":
        # No door: the laptop reaches the host at its inventory address (a LAN
        # goatnet, ADR 1213). The host line is the address as-is, no port-forward,
        # no jump, no ssh overrides — the base inventory's own transport.
        addr = door["host"] if host.get("landed") else host.get("host") or host.get("mesh_ip")
        return f'{host["name"]} ansible_host={addr} {conn}ansible_user={user} {role_kv}'.rstrip()
    if host.get("landed"):
        if door["type"] == "ssm":
            transport = f'ansible_host=127.0.0.1 ansible_port={door["port"]} {conn}ansible_user={user} {role_kv}'.rstrip()
        else:  # tailscale — land directly on the IP, no port-forward
            transport = f'ansible_host={door["ip"]} {conn}ansible_user={user} {role_kv}'.rstrip()
        args = SSH_TAIL
    else:  # jumps through the landed host
        transport = f'ansible_host={host["mesh_ip"]} {conn}ansible_user={user} {role_kv}'.rstrip()
        args = _proxy_args(user, door["port"])
    return f'{host["name"]} {transport} ansible_ssh_common_args="{args}"'


def _door_ini_block(door):
    lines = [f'[{door["mgmt"][0]["group"]}]']
    lines += [_host_line(door, h, connection=False) for h in door["mgmt"]]
    return "\n".join(lines) + "\n"


def _groupvars_ini_blocks(door):
    """One [<gv_group>] block per distinct gv_group (a goatnet may split obs into
    its own group with its own group_vars)."""
    out, seen = [], []
    for h in door["mgmt"]:
        g = h["gv_group"]
        if g in seen:
            continue
        seen.append(g)
        lines = [f'[{g}]'] + [_host_line(door, hh, connection=True)
                              for hh in door["mgmt"] if hh["gv_group"] == g]
        out.append("\n".join(lines) + "\n")
    return out


def _has_group(text, name):
    return re.search(rf'^\[{re.escape(name)}\]', text, re.M) is not None


def _append_block(text, block):
    if not block.endswith("\n"):
        block += "\n"
    sep = "" if text.endswith("\n\n") else ("\n" if text.endswith("\n") else "\n\n")
    return text + sep + block


def establish(env, doors, files):
    """Return (new_files, missing) — new_files with this goatnet's blocks appended
    where absent; `missing` lists the artifacts that lacked it (i.e. were changed)."""
    door = _resolve_door(env, doors)
    if door is None:
        raise SystemExit(
            f"generate-door-inventory: '{env}' has no door in {REG.relative_to(REPO)} — "
            f"neither a top-level entry nor a joiner listed under a founder's door.\n"
            f"  A FOUNDER declares its own door (ssm: type+instance+region+port+user; "
            f"tailscale: type+ip+user; direct: type+host+user — a LAN goatnet with no door).\n"
            f"  A JOINER (goatnet_founder: false) is reached through its founder's door — "
            f"list its nodes under that door with gv_group: {env}_hosts.\n"
            f"  See docs/design/env-doors.md.")
    out = dict(files)
    missing = []

    di = out["doors_ini"]
    if not _has_group(di, door["mgmt"][0]["group"]):
        out["doors_ini"] = _append_block(di, _door_ini_block(door))
        missing.append("doors.ini")

    gv, gv_missing = out["groupvars_ini"], False
    for blk in _groupvars_ini_blocks(door):
        name = blk.splitlines()[0][1:-1]
        if not _has_group(gv, name):
            gv = _append_block(gv, blk)
            gv_missing = True
    out["groupvars_ini"] = gv
    if gv_missing:
        missing.append("doors-groupvars.ini")

    return out, missing


def _read_files():
    return {"doors_ini": DOORS_INI.read_text(),
            "groupvars_ini": GROUPVARS_INI.read_text()}


_PATHS = {"doors_ini": DOORS_INI, "groupvars_ini": GROUPVARS_INI}


def run(envs, check):
    doors = _load_registry()
    files = _read_files()
    missing_by_env = {}
    for env in envs:
        files, missing = establish(env, doors, files)
        if missing:
            missing_by_env[env] = missing
    if check:
        if missing_by_env:
            print("DOOR NOT ESTABLISHED — run ops/scripts/generate-door-inventory.py <goatnet>:")
            for env, arts in missing_by_env.items():
                print(f"  - {env}: missing from {', '.join(arts)}")
            return 1
        print(f"ok: every requested goatnet has an env-door ({', '.join(envs)}).")
        return 0
    changed = False
    for key, path in _PATHS.items():
        if files[key] != path.read_text():
            path.write_text(files[key]); changed = True
            print("updated", path.relative_to(REPO))
    if not changed:
        print(f"door(s) already established: {', '.join(envs)}.")
    return 0


def self_test():
    reg_ssm = """doors:
  demo:
    type: ssm
    instance: i-abc
    region: eu-central-1
    port: 12250
    user: ops
    mgmt:
      - {name: mgmt-demo-01, mesh_ip: 192.0.2.9, group: demo_door_hosts, gv_group: demo_hosts, landed: true}
      - {name: mgmt-demo-02, mesh_ip: 192.0.2.10, group: demo_door_hosts, gv_group: demo_obs_hosts}
"""
    doors = _load_registry(reg_ssm)
    files = {"doors_ini": "[keep_me]\nx y\n", "groupvars_ini": "[keep_me]\nx y\n"}
    out, missing = establish("demo", doors, files)
    problems = []
    if "[demo_door_hosts]" not in out["doors_ini"]:
        problems.append("doors.ini block not added")
    if "ansible_port=12250" not in out["doors_ini"]:
        problems.append("landed ssm port missing")
    if "ProxyCommand" not in out["doors_ini"]:
        problems.append("jump host proxycommand missing")
    if "[demo_hosts]" not in out["groupvars_ini"] or "[demo_obs_hosts]" not in out["groupvars_ini"]:
        problems.append("gv_group split not honored")
    if "ansible_connection=ssh" not in out["groupvars_ini"]:
        problems.append("groupvars missing ansible_connection")
    if "[keep_me]" not in out["doors_ini"] or "[keep_me]" not in out["groupvars_ini"]:
        problems.append("clobbered hand-owned content")
    if set(out) != {"doors_ini", "groupvars_ini"} or missing != ["doors.ini", "doors-groupvars.ini"]:
        problems.append("an artifact other than the two door inventories was written")
    arm = arm_facts("demo", doors) or ""
    if arm.splitlines() != ["INSTANCE=i-abc", "REGION=eu-central-1", "TARGET_IP=192.0.2.9",
                            "TARGET_NAME=mgmt-demo-01", "PORT=12250", "DOOR_USER=ops"]:
        problems.append("--arm does not print the landed host's ssm door facts")
    if arm_facts("demo-obs", doors) != arm_facts("demo", doors):
        problems.append("--arm does not resolve a joiner site to its founder's door")
    # idempotence: re-running adds nothing
    out2, missing2 = establish("demo", doors, out)
    if out2 != out or missing2:
        problems.append("not idempotent")
    # tailscale: no ssm door facts
    ts = _load_registry("""doors:
  ts:
    type: tailscale
    ip: 198.51.100.9
    user: ops
    mgmt:
      - {name: mgmt-ts-01, group: ts_door_hosts, gv_group: ts_hosts, landed: true}
""")
    out3, _ = establish("ts", ts, {"doors_ini": "", "groupvars_ini": ""})
    if arm_facts("ts", ts) is not None:
        problems.append("tailscale wrongly has ssm door facts")
    if "ansible_host=198.51.100.9" not in out3["doors_ini"]:
        problems.append("tailscale lands on ip")
    # direct (ADR 1213): no ssm door facts, the host as-is, no ssh overrides
    direct = _load_registry("""doors:
  pasture:
    type: direct
    host: 192.168.1.50
    user: ubuntu
    mgmt:
      - {name: mgmt-pasture-01, router_role: A, group: pasture_door_hosts, gv_group: pasture_hosts, landed: true}
""")
    out4, _ = establish("pasture", direct, {"doors_ini": "", "groupvars_ini": ""})
    if arm_facts("pasture", direct) is not None:
        problems.append("direct wrongly has ssm door facts")
    if "mgmt-pasture-01 ansible_host=192.168.1.50 ansible_user=ubuntu" not in out4["doors_ini"]:
        problems.append("direct does not land on the host as-is")
    if "ansible_ssh_common_args" in out4["doors_ini"] or "ansible_ssh_common_args" in out4["groupvars_ini"]:
        problems.append("direct carries ssh overrides it has no door for")
    if "[pasture_hosts]\nmgmt-pasture-01 ansible_host=192.168.1.50 ansible_connection=ssh ansible_user=ubuntu router_role=A" not in out4["groupvars_ini"]:
        problems.append("direct groupvars block wrong")
    if problems:
        print("SELF-TEST FAILED:"); [print("  -", p) for p in problems]; return 2
    print("self-test OK — adds missing blocks, preserves hand-owned content, honors gv_group split, "
          "ssm vs tailscale vs direct, idempotent.")
    return 0


def arm_facts(env, doors):
    """KEY=value lines env-door.sh assigns for an ssm door, or None."""
    door = _resolve_door(env, doors)
    if not door or door.get("type") != "ssm" or not door.get("mgmt"):
        return None
    landed = door["mgmt"][0]
    facts = {"INSTANCE": door["instance"], "REGION": door["region"], "TARGET_IP": landed["mesh_ip"],
             "TARGET_NAME": landed["name"], "PORT": door["port"], "DOOR_USER": door["user"]}
    for k, v in facts.items():
        if not re.fullmatch(r"[A-Za-z0-9._-]+", str(v)):
            raise SystemExit(f"generate-door-inventory: door {env}: {k} is not a plain token")
    return "".join(f"{k}={v}\n" for k, v in facts.items())


def main():
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    if "--arm" in sys.argv:
        envs = [a for a in sys.argv[1:] if not a.startswith("--")]
        if len(envs) != 1:
            raise SystemExit("usage: generate-door-inventory.py --arm <goatnet>")
        out = arm_facts(envs[0], _load_registry()) if REG.is_file() else None
        if out is None:
            sys.exit(1)
        sys.stdout.write(out)
        sys.exit(0)
    check = "--check" in sys.argv
    envs = list(_load_registry().keys()) if "--all" in sys.argv else [a for a in sys.argv[1:] if not a.startswith("--")]
    if not envs:
        if "--all" in sys.argv:
            # An empty registry has nothing to generate or verify; the shipped
            # public doors.yaml starts this way.
            print("ok: the doors registry is empty, no door inventories to "
                  + ("check" if check else "generate"))
            sys.exit(0)
        raise SystemExit("usage: generate-door-inventory.py <goatnet> [--check] | --all [--check] | --self-test")
    sys.exit(run(envs, check))


if __name__ == "__main__":
    main()
