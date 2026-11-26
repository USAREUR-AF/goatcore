# Target types and platform families

A machine is one platform family (its hardware) carrying one target type (its
role). Each is a data file: `targets/<type>.yaml` here, `families/<family>.yaml`
beside it. `goatbuilder join-node --class <type>` and `goatbuilder standup`
read them. A new type is a new file.

| Where | What |
|---|---|
| `ops/goatbuilder/targets/`, `families/` | the types and families this repository ships. goatbuilder also carries them, for a checkout with no such file; a checkout whose directory has any reads only its own there |
| `ops/goatbuilder/overlay/targets/`, `overlay/families/` | a private overlay's own types and families. A name goatbuilder or the tree already has is refused (GB-374) |
| `ops/goatbuilder/schema/target-type-v1.schema.json`, `platform-family-v1.schema.json` | the two schemas. An unknown field is refused (GB-374) |

In an assembled tree (`.goat-assembled`) the first row's files must be
goatcore's: byte for byte the marker's `goatcore_ref` commit, read from the
tree's own git, or that commit plus the recorded pre-disclosure patches. Any
other file there is refused (GB-378), and so is a `goatcore_ref` that is not a
commit id of 40 hexadecimal characters. A plain checkout's files are read as
they are. A tree assembled from local directories plans and does not run. The
rule keeps an overlay from replacing a type by accident; a tree without the
marker is a checkout, so it is not a control against someone who edits the tree.

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
| `encryption` | the minimum encryption scope a machine of the type requires (`root`). Each family the type accepts must have a model that delivers it (GB-425, below). `doctor --target` names it and does not check it on the machine: it reads the checkout, not the machine |
| `declares` | what a machine of the type declares in its host_vars besides its family: `tap` (below) |
| `services` | the components of `ops/pins/fleet-pins.yaml` a machine of the type runs. After the phases, `join-node` phase 13 deploys each to the one machine (`deploy --group <group> --node <machine>`), and `doctor --target --live` reads each back. One with no component, group, pin or play is refused before any phase (GB-384); one the pins declare `absent` for the group is skipped. For a type `standup` builds in phase 14, phase 14 deploys each to the type's machines at the site after its plays (`deploy --group <group>`), and refuses one it cannot have before any phase. Not for a type built in `obs-legs` |
| `source` | the repository its plays `from: source` are in, at a full commit: `repository` (a git URL with no credential in it) and `commit` (40 hexadecimal characters). A run of `join-node` or `standup` fetches it before its first phase, below |
| `phases` | what `join-node` phase 13 runs, in order |

A phase entry is one of four kinds. `when: founder` or `when: joiner` limits
it to that kind of site.

- **Plays:** `label`, and `plays:`, each with `play`, `args`, and `from` when the
  play is not in this repository (`overlay`, `source`, or `owner/repository`).
  Nothing requires the play when the file is read or a plan is printed. A run
  that reaches a missing play is refused before its first phase (GB-377). A
  play `from: source` is a path inside the type's `source:` repository; any
  other is a file of `ops/ansible`, named alone. join-node
  adds `-l <node>`; standup's phase 14 adds no limit, so `args` keep the play
  to the type's group. They are `-e key=value` pairs and nothing else (no
  other option, no `@` file), and exactly one sets target, to `target={group}`
  (a type in the core group may write `target={site_group}`).
- **A built-in step:** `builtin: <name>`, whose logic is goatbuilder's own. A
  name goatbuilder does not have is refused (GB-374).
- **A release component:** `label`, `component: <name>` and `skip: <the reason>`.
  Where the pins have a release pin of it for the type's group, `join-node` adds
  a phase 3 gate that runs `goatbuilder deploy --dry-run` for it (every check
  deploy makes before a host, the release's verification included) and a phase
  13 step that runs its
  play on the machine; where they have none, the entry is a declared skip with
  that reason and what is missing. Not for a type standup builds.
- **A declared skip:** `label` and `skip: <the reason>`.

### A type's plays in another repository

A type may declare `source:` with `repository` and a full `commit`, and mark the
plays that are in it `from: source`. On the operator's machine, before its
first phase, a run of `join-node` or `standup` fetches that commit with the
person's own git access into `<state_dir>/goat-type-sources/<type>/<commit>`, and
on every later run holds the kept copy to the commit: HEAD is the commit and no
file differs. Each play `from: source` must be a file there. The copy may carry
nothing that would change what goatcore's own roles mean: no directory named
after a role goatcore has, no `group_vars` or `host_vars` (beside a play they
win over the checkout's), no `library`, `module_utils`, `collections` or
`*_plugins` directory, in any case of letters, no link and no submodule, and
nothing where a role of goatcore's reads a file beside the playbook
(`{{ playbook_dir }}/../enrollment/...` and the like): for a play from a source,
`playbook_dir` is the play's own directory in the copy, so such a role finds
nothing there and fails, rather than reading the source's file. The
files are read from the commit's own tree, and an index entry marked so that
git does not compare it is refused. Its plays run with goatcore's
roles first on the roles path, then the roles beside the play, then the copy's
own, and with `-i vars-here.ini`, an inventory that names no host, so Ansible
reads the checkout's `group_vars` and `host_vars`. Anything else is refused
before any phase (GB-420), and each play's command checks the copy again just
before it runs. A plan prints where the copy is kept and fetches nothing.
goatbuilder holds no credential for the repository. A type `standup` builds in
`obs-legs` has no play from its source.

Text may use `{site}`, `{site_us}` (`-` as `_`), `{site_group}`, `{group}` and
`{founder}`. `{founder}` is several words ("the founder zagros"), so only a
label or a skip reason names it, never `args` or `preflight`.

## A machine's family

A machine declares its family with the Ansible variable `platform_family`. It
is read as Ansible resolves it, highest first: `host_vars/<node>`; the
machine's inventory line, the type's own inventory before
`mgmt-hosts-mesh.ini` before `mgmt-hosts.ini`; its group's `group_vars`, then
`group_vars/all`; its group's `[<group>:vars]`, then `[all:vars]`. In a vars
directory the last file wins. A family its type does not accept is refused
before any phase (GB-376), by `join-node` and by `standup`. A machine that
declares none is not checked, and neither is a templated value or one set only
on a parent group.

A type that converges as another shares its group: a phone sits in the core's
group, so it inherits that group's `group_vars`. Declare a phone's family in
its `host_vars`. At a joining site `standup` cannot tell a phone from a core
machine in that group, so it passes a machine there that the core type or a
type converging as core accepts; at a founder, which has no phone, it asks the
core type alone. `join-node --class` holds a machine to its own type.

A family's `models` are keys of `platforms` in
`ops/bootstrap/hardware-classes.yaml`; each platform is in exactly one family.
Each model maps to its entry, the way a machine of it is prepared up to a
reachable Ubuntu host:

| Field | What it holds |
|---|---|
| `name`, `arch` | the model's name, and `amd64`, `arm64` or `unknown` |
| `firmware`, `boot_media` | one line each: the firmware settings or boot chain, and what the OS is installed from and onto |
| `os` | `image`, one line; `ubuntu`, each release and its state on the model (`measured`, `built`, `designed`, `not built`, `not supported`, `unknown`). `24.04` and `26.04` are always stated |
| `encryption` | `scope`, what the preparation encrypts: `root`, `data` (a data volume, not the root), `none` or `unknown`; `unlock` and `recovery`, how the disk opens at boot and when that fails (`tpm2`, `tang`, `yubikey-hmac`, `usb-key`, `console-passphrase`, `ssh-passphrase`, `cloud-kms`, `phone-unlock`, `unknown`) |
| `extras` | what the model needs beyond the image, one line each: a kernel module, a library, a configuration |
| `builder` | the golden-path builder, a path in this repository, or `none: ` and the reason |
| `directions` | written directions, a path in this repository, where there are any |

A family may instead list its models' names (`models: [rack-1]`), which says
nothing about them. A type's `encryption` is accepted only for families at
least one model of which has that `scope`; any other family is refused when the
files are read (GB-425), and a listed name or an `unknown` scope delivers
nothing. `doctor --target` names the models that deliver it and the ones that
do not. Every value in the shipped files is cited to its source in
`docs/design/goatbuilder-build-targets.md` section 3.1.

## A network tap's mode

A machine of a type that `declares: [tap]` (`network-tap.yaml`) states in its
host_vars how it sits on the link it records. There is no default: `join-node`
and `doctor --target` refuse a machine with no `tap:` or a wrong one (GB-380).

```yaml
tap:
  mode: inline                 # listen (fail-open) | inline (fail-closed)
  ports: [eth1, eth2]          # the link's interfaces
  management: eth0             # management, the mesh, the default route
```

- **listen** fails open: the link does not pass through the machine. It names
  its `source`: `copper-tap` (two ports, one per direction), `mirror-port`
  (one port) or `bypass-relay` (one or two).
- **inline** fails closed: the machine bridges exactly two ports, so the link
  is down whenever the machine is off, restarting or broken. It has no
  `source`. `join-node` and `doctor --target` print `inline, fail-closed` on
  the machine.
- The management interface is never a port. A value is read as written; a
  templated one is refused.

The file is read from `host_vars/<machine>.yml` (or `.yaml`), then the files
of `host_vars/<machine>/` in order; the last that sets `tap` wins.

goatrope installs the tap itself (its ports, the bridge or the egress drop,
the watchdog, the recording ring) at a pinned, signed release. The type names it
as a `component`: where `ops/pins/fleet-pins.yaml` pins the release component
`goatrope` for the tap's group, `join-node` runs deploy's own dry run for it in
phase 3 (fresh source, ladder, digest, signed intent, release, secrets), before
anything on the machine changes, and phase 13 installs it. The play is the
component's own `deploy_playbook`, which brings the tap up, else
`release-file-deploy.yml`, which puts the file in place and starts nothing.
Where there is no pin, phase 13 is a declared skip that says why: goatrope has
no release yet, so a machine joined as `network-tap` today is not a working tap.
A pin that is not a release pin, or a `goatrope` that is not a release
component, is refused before any phase (GB-381), never skipped. No site declares a tap, so `standup`
builds none.

## An octo-pod

`octo-pod.yaml` names the release component `octo-pod`, as the tap names
goatrope. Where the pins have a release pin of it for the pod's group, phase 13
runs `ops/ansible/octo-pod-deploy.yml`: the verified package, then octo-pod's
own `bootstrap`, `init --pod <machine>` for a pod with no identity, `apply` and
`verify`. Where they have none, phase 13 is a declared skip. The public pins have
no octo-pod row; a deployment with pods declares it in its own:

```yaml
  octo-pod:
    var: octo_pod_release
    artifact_type: release
    owns_state: true             # the pod's identity and what it keeps
    rollback: pin-revert         # the package before; the identity and data stay
    fleet: "USAREUR-AF/octo-pod@octo-pod-v0.6.0@sha256:9a6db6919fad34486a6643066a30e26b487e7e4f9307ac6b62d61d7dcc86a538"
    release:
      checksums: SHA256SUMS
      files: {amd64: octo-pod_0.6.0_all.deb, arm64: octo-pod_0.6.0_all.deb}
      signer: https://github.com/USAREUR-AF/octo-pod/.github/workflows/release.yml
```

The package's name carries its version, so a new pin moves `files` with it.

## Checking a site's machines

`goatbuilder doctor --target <type> --site <site>` (`--node <machine>` for one)
reads the same: each of the site's machines of the type, its declared family,
and the type's plays. It reaches no host.

With `--live` it also reads each network tap over Ansible, changes nothing, and
holds its end state to its declared mode: addresses, what sends, the default
route, the bridge or the egress drop, offloads, the recorder's interface, what
boot waits for, the watchdog and the encrypted root. A fact it could not read is
unknown, which is not a pass.

## Not built

OS versions, acceptance rows, `doctor --target`'s checks on the machine of any
type but a network tap, and the VM harness
(docs/design/goatbuilder-build-targets.md) are later changes. The schema takes
no field for OS versions or acceptance rows yet.

`standup` reads which plays each obs leg runs from `obs.yaml`. It keeps its own
leg labels, and it runs every leg at a founder and a joiner alike, as it did
before the type files: a leg entry's `label` and `when` are read by `join-node`
only. Moving them onto the file changes standup's plans, so it is its own
change.
