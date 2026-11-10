# 5. Reaching hosts

goatbuilder decides how to reach an environment by trying to reach it, in a
fixed order, and prints the transport it chose before it runs anything. The
same order is used by `deploy-component.sh`, and the deploy parity gate compares
the two.



## The order

| # | Transport | Chosen when | Inventories loaded |
|---|---|---|---|
| 1 | Mesh, by profile | the laptop's NetBird profile is the environment's and management reads `Connected` | `mgmt-hosts.ini` + `mgmt-hosts-mesh.ini` |
| 2 | Mesh, by reach | the first mesh host of the first member group answers on tcp/22 within three seconds | the same |
| 3 | Door | the environment has an entry in `ops/ansible/inventory/doors.yaml`; goatbuilder opens it with `ops/scripts/door-open.sh` | `mgmt-hosts.ini` + `doors-groupvars.ini` |
| 4 | Direct | none of the above | `mgmt-hosts.ini` |

Reaching the host decides, not the profile's name. A laptop on the right mesh
whose profile is called something else still deploys over the mesh.

**Tailscale is never a transport.** It is the break-glass rescue path for one
operator's own machine (ADR 1114).
`doctor` refuses a door declared as Tailscale.

## Doors

A door reaches a goatnet's management host without joining the laptop to that
goatnet's mesh ([ADR 1117](../../adr/1117-per-env-doors-cross-env-management.md),
[env-doors.md](../../design/env-doors.md)). The registry is
`ops/ansible/inventory/doors.yaml`, one entry per goatnet;
`ops/scripts/generate-door-inventory.py` renders it into the two door
inventories, and `env-door.sh` reads a door's facts from it each time it runs.

An `ssm` door is an AWS Systems Manager port-forward through a cloud mesh peer
to the landed management host on `127.0.0.1:<port>`. The other management hosts
are reached through it. An open door closes after about twenty minutes idle.

```bash
ops/scripts/env-door.sh <goatnet> --print    # what the door would open
ops/scripts/door-open.sh <goatnet>           # open it
```

## Fresh source before the first host

Before any host is touched, `deploy`, `rollback` and `standup` run
`ops/scripts/fresh-source.sh`, the same check the scripts run. It fetches the
trunk and refuses a checkout that lacks trunk commits to `ops/ansible` or
`ops/pins` (GB-064), naming the missing commits and the fast-forward command.
A checkout carrying its own commits runs with a warning. A trunk that cannot be
fetched is refused (GB-065) unless the run passes `--offline`, which judges the
checkout against the cached trunk and writes `source: offline` into the ledger
row. Every play of the run then uses the trunk commit pinned at the start.

## Controls

| Variable | Effect |
|---|---|
| `GOAT_MESH_PROBE=0` | skip the three-second tcp/22 reach test (CI has no mesh) |
| `GOAT_DOOR_OPEN=0` | decide the door without opening it (the parity gates use this) |
| `GOAT_AWS_MODE=profile\|none` | how AWS credentials are found for a door; `none` for a mesh-only run |

## SSH multiplexing

`ops/ansible/ansible.cfg` keeps one SSH connection per host open for ten
minutes (`ControlMaster=auto`, `ControlPersist=600s`). Measured on a management
host: an Ansible task's SSH cost fell from 6.4 s to 1.5 s per call. Hosts
reached through a jump host keep `ControlMaster=no`.

## What a run prints

```text
goatnet <goatnet>: groups [<site>_hosts] via mesh (this laptop reaches <mesh host>:22)
playbook <component>-deploy.yml -e target=<group>; ledger component <component>
```

## Common refusals

| Code | Meaning |
|---|---|
| GB-006 | the door to the goatnet did not open |
| GB-030 | no host of the group is in the loaded inventories (a declared site not yet built) |
| GB-043 | the door to a rung did not open for its live validation rows |
| X-SSH-001 | the host refused the key |
| X-SSH-002 | no route to the host |
| X-SSH-003 | the host key changed |
| GB-064 | the checkout is behind the trunk in `ops/ansible` or `ops/pins` |
| GB-065 | the trunk could not be fetched; an islanded site runs with `--offline` |
| X-AWS-003 | the SSM target is not connected |
| X-AWS-004 | the AWS CLI fell back to an expired login session |
| X-NETBIRD-001 | no such mesh profile |
| X-NETBIRD-002 | the mesh daemon is not running |
