# What this release contains

This repository, `goatcore`, is the substrate of goat: the operator CLI, the build tool, the relay, the identity and fabric services, the Ansible roles and playbooks that deploy them, and the design record. You bring the hosts, the secrets and the configuration for your own goatnet. A goatnet is one environment with its own mesh, identity service, certificate authority and data fabric.

To see it work with nothing but a laptop, follow the ten-minute rehearsal in the [README](../README.md): a scripted converger stands in for the hosts.

## Status words

Each component below has one of four words, and each word means one thing.

| Word | Means |
|---|---|
| **Tested in CI** | A workflow under `.github/workflows/` builds it and runs its tests on each pull request |
| **Built** | The code is here and a workflow checks that it parses or compiles. No workflow runs its behaviour |
| **Not run** | The code is here and no workflow here runs it |
| **WIP** | Being finished. Not for production use |

## Components

| Component | Path | Status | Workflow |
|---|---|---|---|
| `goat`, the operator CLI | `crates/goat-cli/` | Tested in CI | `ci.yml`, `goat-cli-pr.yml` |
| `goatbuilder`, build, release, deploy and standup | `crates/goatbuilder/` | Tested in CI | `ci.yml` |
| Release binaries of `goat` and `goatbuilder` for linux-amd64, linux-arm64 and darwin-arm64, with a signed `SHA256SUMS` | `.github/workflows/goat-release.yml`, `.github/workflows/goatbuilder-release.yml` | Not run | They run on a tag `goat-cli-v<semver>` or `goatbuilder/v<semver>` |
| Error catalog and interpreter | `crates/goat-errors/`, `ops/errors/` | Tested in CI | `ci.yml`, `error-catalog.yml` |
| `panoscope-core`, the portal client shared with `goatstuff` | `crates/panoscope-core/` | Tested in CI | `ci.yml` |
| Relay and relay protocol | `goat-relay/`, `goat-relay-protocol/` | Tested in CI | `goat-relay.yml`, `goat-relay-protocol.yml` |
| Federated signal server | `signal-server/` | Tested in CI | `signal-server.yml` |
| Offline CA and enrollment bundles | `ops/enrollment/` | Tested in CI | `enrollment.yml` |
| Identity admin console | `ops/goat-id-console/` | Tested in CI | `goat-id-console.yml` |
| Fabric portal | `ops/goat-zenoh-portal/` | Tested in CI | `goat-zenoh-portal-test.yml` |
| Grant-keyed encryption SDK | `ops/grant-crypto/` | Tested in CI | `grant-crypto.yml` |
| Fleet visibility agent | `ops/snitch/` | Tested in CI | `snitch.yml` |
| Other Go modules: `admit-audit`, `onboard-runner`, `mgmt-host-bootstrap`, `time-hygiene`, `relay-probe`, `wg-cp0-mesh-reach`, `smoketest`, `signal-federation-probe`, `federation/reference`, `zenoh/loadgen` | `ops/<name>/` | Tested in CI | `ci.yml`, job `go` |
| Ansible roles and playbooks | `ops/ansible/` | Built | `ci.yml` checks that playbooks parse and role references resolve |
| Standup procedure on hosts | `docs/operations/goatnet-standup.md`, `ops/scripts/goat-standup.sh` | Not run | |
| Tutorials 1 to 3, rehearsed on fixtures | `docs/tutorials/`, `ops/goatbuilder/fixtures/goatnet-from-scratch/` | Tested in CI | `ci.yml` in `hello-galaxy-goatenv` |
| goatlite, one box from one file | `docs/tutorials/00-goatlite.md`, `ops/scripts/goatlite-render.py` | WIP | |
| Container images | `ops/zenoh/`, `ops/netbird/`, `ops/postgres/`, and the `*-image.yml` workflows | Not run | They run on a push to `main` |
| Terraform modules | `terraform/modules/` | Not run | |
| Gates | `ops/ci/` | Tested in CI | `ci.yml` runs the ones that apply; each is a script you can run locally |

**goatlite. Status: WIP, not for production use.** The gate is a person who did not write it taking a fresh Ubuntu VM to a green `goatbuilder assay` and a round-trip publish with only `docs/tutorials/00-goatlite.md`. Its rehearsal on the fixture is Tested in CI; its run on a real host is Not run. Production is the full preset in [Tutorial 1](tutorials/01-new-goatnet.md).

## What you bring

What you need depends on how far you go. The rehearsal needs a laptop; everything past it needs hosts.

| To do this | You bring |
|---|---|
| The rehearsal | Rust, to build `goatbuilder` from `crates/` |
| goatlite | One Ubuntu 24.04 host or VM with 4 cores and 16 GB that you can reach by SSH with a key; a laptop on the same LAN with Rust and the NetBird client. The images are public on ghcr.io and need no account |
| A full goatnet on one LAN | The hosts: a core pair and an observability pair is the baseline, one core host is enough to start. Each runs Ubuntu 24.04 with disk encryption. Also a secret store, and `yq`, `sshpass`, `go` and `age`, which `goatbuilder doctor` checks |
| A full goatnet on the internet | The above, plus your own AWS account, a Cloudflare zone and a domain, for the relays and public DNS |
| Your own images | A fork. The image workflows publish to `ghcr.io/<your owner>/` with `GITHUB_TOKEN` alone |

## What works from a clone, and what needs your configuration

These work from a clone with no configuration:

- `goatbuilder standup zagros --site alpha --mock <dir>` and the other verbs under `--mock`, against the fixtures.
- `goatbuilder explain` and `goat explain`, with an error code or pasted text.
- `goat --help`, and the `goat acl` verbs run inside a checkout (they read its `ops/ansible/netbird_acl_*.yml`; elsewhere pass `--declaration <dir>`).
- Every script in `ops/ci/`, from a git checkout.

These need configuration from you first:

| Thing | What it needs |
|---|---|
| A goatnet on hosts | A row in `ops/pins/fleet-pins.yaml`, a declaration in `ops/ansible/group_vars/<name>_hosts/`, inventory in `ops/ansible/inventory/`. For goatlite, `ops/scripts/goatlite-render.py` writes all of them from `goatlite.yaml` |
| The CA | `ops/scripts/goatlite-render.py` mints one for goatlite. For a full goatnet, see `docs/operations/goatnet-standup.md` |
| Image digests | `fleet:` lines in `ops/pins/fleet-pins.yaml` and image defaults in `ops/compose/mgmt-host/docker-compose.yml` name digests under `ghcr.io/usareur-af`. Set `GHCR_ORG` and these digests when you run your own builds |
| `netbird-mgmt` | The image has no CA of its own. Bind-mount your CA bundle into the container and set `SSL_CERT_FILE`, or it cannot validate your internal TLS endpoints. `ops/compose/mgmt-host/docker-compose.yml` does not set it |
| `goat admin lan` | List your hosts in `~/.goat/lan-hosts.yaml` |
| `goat demo` | A demo server to name with `--url`. None of the five repositories contains one |
| `goat` contexts for your goatnet | `ops/validation/goatnet-runners.yaml` starts empty, and goatlite adds its row |
| Release ladder | `ops/release/ladder.yaml` starts with no rungs. Add your goatnets in order |

## Where the pieces live

Five repositories make up the project. Only this one is needed to run a goatnet.

| Repository | Holds | Relation to this one |
|---|---|---|
| `goatcore` (here) | the substrate | |
| [`goatdocs`](https://github.com/usareur-af/goatdocs) | architecture overviews, the design system, reference material, partner and vendor templates | links here |
| [`goatstuff`](https://github.com/usareur-af/goatstuff) | inspection surfaces, demos, synthetic publishers | builds `panoscope-core` from here by tag `goatcore/v0.1.0` |
| [`goatenv`](https://github.com/usareur-af/goatenv) | one example goatnet, `zagros`, as an overlay generated from four invented facts, and `terraform-zagros/`, an optional cloud footprint | uses this repository at the tag in `goatcore.ref` |
| [`hello-galaxy-goatenv`](https://github.com/usareur-af/hello-galaxy-goatenv) | the tutorials as fixtures with recorded output | rehearses against this repository at `GOATBUILDER_REF` |

## How to change the main things

Each row names the file that holds the fact, so a change has one home.

| To change | Edit | Then run |
|---|---|---|
| A goatlite goatnet's name, host, SSH user or admin | `goatlite.yaml` | `python3 ops/scripts/goatlite-render.py --site <name>`, then `python3 ops/scripts/standup-config-preflight.py <name>_hosts` |
| A site's settings on a full goatnet | `ops/ansible/group_vars/<name>_hosts/` | `python3 ops/scripts/standup-config-preflight.py <name>_hosts` |
| Which image a component runs | `ops/pins/fleet-pins.yaml` | `goatbuilder preflight <component> --env <goatnet>`, then `goatbuilder deploy` |
| Who can reach what on the mesh | your policy directory, starting from `ops/warrant/demo/` | `python3 ops/warrant/generate.py --policy-dir <dir> --target ops/ansible/netbird_acl_matrix.yml`, rebuild `goat`, then `goat acl test` |
| Where secrets are kept | `secrets: {store: <name>}` in the goatbuilder configuration, or `GOAT_SECRET_STORE` for one command. Stores are listed in `docs/design/pluggable-dependencies.md` | `goatbuilder doctor` |
| The image namespace | `GHCR_ORG`, or `ghcr_org` in `ops/ansible/group_vars/all.yml` | a build of your images, then new digests in `ops/pins/fleet-pins.yaml` |
| What a role deploys | `ops/ansible/roles/<role>/` | `ansible-playbook --syntax-check`, then the role's phase in the standup |
| An error message | `ops/errors/catalog.yaml` | `goat explain <code>` |
| The services a goatnet hosts | `ops/serve/services.yaml` | `python3 ops/serve/generate.py` |
| A decision | a new file in `docs/adr/`, and a row in `docs/adr/INDEX.md` | a pull request; see [CONTRIBUTING.md](../CONTRIBUTING.md) |

The goat book ([`docs/goat-cli-book/`](goat-cli-book/)) and the goatbuilder book ([`docs/goatbuilder-book/`](goatbuilder-book/)) cover each verb in full.
