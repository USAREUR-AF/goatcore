# Target types and platform families

A machine is one platform family (its hardware) carrying one target type (its
role). Each is a data file: `targets/<type>.yaml` here, `families/<family>.yaml`
beside it. `goatbuilder join-node --class <type>` and `goatbuilder standup`
read them. A new type is a new file.

| Where | What |
|---|---|
| `ops/goatbuilder/targets/`, `families/` | the types and families this repository ships; goatbuilder also carries them, for a checkout that has none |
| `ops/goatbuilder/overlay/targets/`, `overlay/families/` | a private overlay's own types and families. A name this repository already uses is refused (GB-374) |
| `ops/goatbuilder/schema/target-type-v1.schema.json`, `platform-family-v1.schema.json` | the two schemas. An unknown field is refused (GB-374) |

## A type file

| Field | Meaning |
|---|---|
| `type` | the name, and the file's stem |
| `families` | the platform families the role accepts; never a model. A family with no file is refused (GB-375) |
| `group` | the inventory group; default `{site_us}_<type, - as _>_hosts` |
| `preflight` | the target of `standup-config-preflight.py`; default `{site_group}` |
| `inventory` | an inventory of the type's own, read when the checkout has it |
| `site` | the group_vars key that declares the machine (`declared_by`), the key that says why a site has none (`absent_reason`), and where `standup` builds it: `phase-14`, or `obs-legs` (obs alone) |
| `converges_as` | the type runs another type's phases, in the same group |
| `phases` | what `join-node` phase 13 runs, in order |

A phase entry is one of three kinds. `when: founder` or `when: joiner` limits
it to that kind of site.

- **Plays:** `label`, and `plays:`, each with `play`, `args`, and `from` when the
  play is not in this repository (`overlay`, or `owner/repository`). Nothing
  requires the play when the file is read or a plan is printed. A run that
  reaches a missing play is refused before its first phase (GB-377). join-node
  adds `-l <node>`.
- **A built-in step:** `builtin: <name>`, whose logic is goatbuilder's own. A
  name goatbuilder does not have is refused (GB-374).
- **A declared skip:** `label` and `skip: <the reason>`.

Text may use `{site}`, `{site_us}` (`-` as `_`), `{site_group}`, `{group}` and
`{founder}`.

## A machine's family

A machine declares its family with the Ansible variable `platform_family`: in
`host_vars/<node>.yml`, on its inventory line, or in its group's `group_vars`,
read in that order. A family its type does not accept is refused before any
phase (GB-376), by `join-node` and by `standup`. A machine that declares none
is not checked.

A family's `models` are keys of `platforms` in
`ops/bootstrap/hardware-classes.yaml`; each platform is in exactly one family.

## Not built

Encryption scope, OS versions, acceptance rows, `doctor --target` and the VM
harness (docs/design/goatbuilder-build-targets.md) are later changes. The
schemas take no field for them yet.
