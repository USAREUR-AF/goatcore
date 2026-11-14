# 3. Configuration

goatbuilder knows nothing about a repository except what its config file says.
The config names the components, the environments and their host groups, the
promotion path between environments, any sovereign peerings, and where secrets
live.

| Field | Value |
|---|---|
| File | `ops/goatbuilder/goatbuilder.yaml` |
| Schema | `ops/goatbuilder/schema/goatbuilder-v1.schema.json` (`schema: goatbuilder/v1`) |
| In goat's repository | **Generated** by `ops/goatbuilder/gen-config.py` from `ops/pins/fleet-pins.yaml`, `ops/release/ladder.yaml`, `ops/goatbuilder/project.yml` (`repo:`, `deploy:`) and `ops/goatbuilder/posture.yml` (`secrets:`). Do not edit it; change a source and regenerate. A deployment that keeps its values in a private overlay carries its own `project.yml` and `posture.yml` there |
| Gate | `.github/workflows/goatbuilder-schema.yml` validates the file and the schema's refusal tests on every pull request |

## The shape

Five top-level keys are required: `schema`, `repo`, `components`,
`environments` and `promotion`. Four are optional: `preset`, `peerings`,
`deploy` and `secrets`. An unknown or malformed field fails validation.

```yaml
schema: goatbuilder/v1
preset: full                       # full | lite: lite supplies defaults
repo:
  source: example-org/zagros       # <org>/<repo>: where declarations and tags live
  image_namespace: ghcr.io/example-org   # where component images are pushed
  signing: intent-sign             # how intents are signed
  release_source: example-org/zagros     # who signs the goatbuilder binaries; default: source
deploy:
  aws_profile: zagros              # GOAT_AWS_PROFILE overrides it; default: default
  ssh_user: ubuntu                 # the account join-node first reaches a machine as
components:
  portal:
    image_workflow: portal-image.yml        # the CI workflow that builds it
    declared_in: ops/pins/fleet-pins.yaml   # where its digest is declared
    var: portal_image                       # the group var the play reads
environments:
  - name: zagros_dev               # the pins' goatnet key
    converger: push-ansible
    groups: [zagros_dev_hosts]     # the Ansible member groups (<site>_hosts)
  - name: zagros_preprod
    converger: push-ansible
    groups: [zagros_preprod_hosts]
  - name: zagros
    converger: push-ansible
    groups: [zagros_hosts]
promotion:
  path:
    - name: dev
      kind: rung
      environment: zagros_dev      # an environment declared above
    - name: preprod
      kind: rung
      environment: zagros_preprod
      gates: [{id: candidate-record}, {id: validation-evidence}]
    - name: prod
      kind: rung
      environment: zagros
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

`peerings:` declares each sovereign peering: the partner's identifier (`with`),
the environment and site that host the edge, the class that runs the dedicated
edge router (`edge_class`), the tunnel (`wireguard` or `ipsec`), the trust
mechanism (`spire-federation`) and the key prefix the partner's data arrives
under (`ingress_prefix`, of the form `from/<partner>`). `goatbuilder federate
<with>` refuses a peering that is not declared here.

```yaml
peerings:
  - with: <partner>
    environment: <goatnet>
    site: <site>
    edge_class: core
    tunnel: wireguard
    trust: spire-federation
    ingress_prefix: from/<partner>
```

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

A config that is YAML but does not validate (the schema, or a cross-field
rule) is GB-025, exit 1. A file that is not YAML at all is GB-245, exit 2:
its fix is the syntax error the message names, at its line and column. A
config that cannot be read is GB-244. In goat's repository, regenerate rather
than edit:

```bash
python3 ops/goatbuilder/gen-config.py
```
