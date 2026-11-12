# 3. Configuration

goatbuilder knows nothing about a repository except what its config file says.
The config names the components, the environments and their host groups, the
promotion path between environments, any sovereign peerings, and where secrets
live.

| Field | Value |
|---|---|
| File | `ops/goatbuilder/goatbuilder.yaml` |
| Schema | `ops/goatbuilder/schema/goatbuilder-v1.schema.json` (`schema: goatbuilder/v1`) |
| In a deployment | **Generated** by `ops/goatbuilder/gen-config.py` from `ops/pins/fleet-pins.yaml`, `ops/release/ladder.yaml`, `ops/goatbuilder/project.yml` (`repo:`, `deploy:`), `ops/goatbuilder/posture.yml` (`secrets:`) and the overlay's `ops/goatbuilder/federation.yml` (environment overlays and peerings). Do not edit it; change a source and regenerate. The public core intentionally ships neither `federation.yml` nor generated deployment config |
| Checks | Run the Python suite and `gen-config.py --check` in the assembled deployment. Public-core CI does not currently run the assembled-output drift check |

## The shape

Five top-level keys are required: `schema`, `repo`, `components`,
`environments` and `promotion`. Four are optional: `preset`, `peerings`,
`deploy` and `secrets`. An unknown or malformed field fails validation.

```yaml
schema: goatbuilder/v1
preset: full                       # full | lite: lite supplies defaults
repo:
  source: <org>/<repo>             # where declarations and tags live
  image_namespace: ghcr.io/<org>   # where component images are pushed
  signing: intent-sign             # how intents are signed
  release_source: <org>/<repo>     # who signs the goatbuilder binaries; default: source
deploy:
  aws_profile: <profile>           # GOAT_AWS_PROFILE overrides it; default: default
  ssh_user: <user>                 # the account join-node first reaches a machine as
components:
  portal:
    image_workflow: portal-image.yml        # the CI workflow that builds it
    declared_in: ops/pins/fleet-pins.yaml   # where its digest is declared
    var: portal_image                       # the group var the play reads
environments:
  - name: <goatnet>                # the pins' goatnet key
    converger: push-ansible
    groups: [<site>_hosts]         # the Ansible member groups
promotion:
  path:
    - name: dev
      kind: rung
      environment: <dev goatnet>
    - name: preprod
      kind: rung
      environment: <preprod goatnet>
      gates: [{id: candidate-record}, {id: validation-evidence}]
    - name: prod
      kind: rung
      environment: <prod goatnet>
      gates:
        - {id: candidate-record}
        - {id: soak, min_days: 1}
        - {id: convergence-evidence}
        - {id: validation-evidence}
        - {id: operator-ack}
secrets:
  store: 1password                 # see chapter 4
```

## Topology: environment, site, class, machine

An **environment** is a goatnet. A **site** is one host site within it; the
first site is the **founder**, which originates the account, the CA and the
declared ACL, and every other site is a **joiner**. A **class** is a machine
class within a site: `core`, `obs`, `breakglass` or `network-jump`. A
**machine** is one host. The unit of release is one class at one site.

Each environment lists its Ansible member groups. A deploy runs the play once
per group and reads the ledger from every host of every group.

The generator takes operator-owned environment declarations from the overlay's
`ops/goatbuilder/federation.yml`, keyed by the environment already declared in
the fleet pins and live release ladder. The pins and ladder own `name`,
`converger` and `groups`; declaring any of those in `federation.yml` is refused
with GB-079. The overlay owns the other environment fields, including `sites`,
`transport`, `inventory`, `peering_edge_reserved_udp` and
`peering_directory`:

```yaml
environments:
  <goatnet>:
    peering_directory:
      url: https://directory.example.org:8443
      ca_cert: directory-ca.pem
    sites:
      - name: <site>
        role: founder
        classes: {core: {}, edge: {}}
peerings: []
```

## Components

A component is one key in `ops/pins/fleet-pins.yaml`. For each component the
config names the image workflow that builds it, the file that declares it and
the variable its play reads. A component with `image_workflow: null` is
deployed but not built by a release candidate in this repository.

Each component also has an **artifact type**, which decides how it rolls back:
`image` (the default), `config`, `acl`, `data`, `cert` or `forward-only`. It is
declared where the component's role attaches the deploy ledger, as
`rollback_artifact_type` in the role's `meta/main.yml`, with
`rollback_reversible` and, for the types that cannot roll back,
`rollback_restore_pointer`. Chapter 8 gives the rule for each type.

## Promotion path

The path is the release ladder: an ordered list of rungs, each bound to one
environment, each with the gates a candidate must pass to enter it. The gate
vocabulary is fixed:

| Gate | Passes when |
|---|---|
| `candidate-record` | an open record for the candidate exists in `ops/release/records/` |
| `validation-evidence` | the rung's live validation rows passed and the evidence is recorded |
| `soak` | the candidate has run on the previous rung for at least `min_days` |
| `convergence-evidence` | every host of the previous rung's environment converged on the candidate |
| `operator-ack` | a named person acknowledged the hop |
| `runtime-health` | on the previous rung, since its deploy and over at least the soak: zero container restarts, zero OOM kills, peak memory at most `max_memory_fraction` (default 0.8) of the container limit — read by `ops/release/rung-health.py` into the record |

goat's own repository declares three rungs, `dev`, `preprod` and `prod`, with
the gates shown above.

## Peerings

The overlay's `ops/goatbuilder/federation.yml` declares each sovereign peering under
`peerings:`: the partner's identifier (`with`),
the environment and site that host the edge, the class that runs the dedicated
edge router (`edge_class`), the tunnel (`wireguard` or `ipsec`), the trust
mechanism (`spire-federation`) and the key prefix the partner's data arrives
under (`ingress_prefix`, of the form `from/<partner>`). `goatbuilder federate
<with>` refuses a peering that is not declared here.

```yaml
# ops/goatbuilder/federation.yml
peerings:
  - with: <partner>
    environment: <goatnet>
    site: <site>
    edge_class: core
    tunnel: wireguard
    trust: spire-federation
    ingress_prefix: from/<partner>
```

`goatbuilder.yaml` contains the generated copy consumed by goatbuilder; do not
edit that copy. On migration, copy every operator-owned environment and peering
field into `federation.yml`, and copy existing federation `signing_key`,
`ca_cert` and `ca_key` paths from `secrets.federation` into the `federation:`
block of `posture.yml`. The generator refuses content found only in the old
output rather than silently deleting it. Once adopted, a type-sensitive digest
covering both sources distinguishes later intentional source changes from
output-only edits. Keep `federation.yml` when removing the last topology
declarations, using explicit `environments: {}` and `peerings: []`. Keep
`posture.yml` with its `store:` and remove its `federation:` block to
intentionally remove the key/CA paths. A missing adopted source is refused when
its generated declarations exist. Duplicate YAML keys, non-text mapping keys,
unquoted dates and unsupported YAML scalar types are refused by path with
GB-079. Empty, partial and null-valued federation files are refused: both keys
and their mapping/list types are required. Run `gen-config.py --check` locally
or wire it in the overlay; public-core CI does not currently run it.

## Presets

`preset: lite` supplies defaults for a single environment and a single
converger, so a small deployment is a handful of lines. An explicit field always
overrides the preset. The conformance fixture `ops/goatbuilder/fixtures/basic/`
is a complete `lite` config in under thirty lines.

## Validate

```bash
goatbuilder validate                              # the repository's config
goatbuilder validate path/to/goatbuilder.yaml     # any file
```

*Fixture output:*

```text
ok    config  basic/goatbuilder.yaml validates against goatbuilder/v1
validate: ok (1 check)
```

A config that does not validate is GB-025. In goat's repository, regenerate
rather than edit:

```bash
python3 ops/goatbuilder/gen-config.py
```
