<p align="center">
  <img src="docs/assets/goat-logo.svg" alt="goat" width="320">
</p>

<h1 align="center">goat</h1>

<p align="center">
  A self-hosted data fabric that keeps working when the network around it fails.
</p>

<p align="center">
  <a href="https://github.com/usareur-af/goatcore/actions/workflows/ci.yml"><img src="https://github.com/usareur-af/goatcore/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/USAREUR-AF/goatcore"><img src="https://api.scorecard.dev/projects/github.com/USAREUR-AF/goatcore/badge" alt="OpenSSF Scorecard"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License: Apache 2.0"></a>
</p>

<p align="center">
  <img src="docs/assets/panoscope.gif" alt="panoscope showing synthetic peers on a local Zenoh fabric, then the live stream of a newly published topic" width="960">
  <br>
  <sub>panoscope, goat's live view of the data fabric, running on a laptop with synthetic data. Its code is in <a href="https://github.com/usareur-af/goatstuff/tree/main/ops/panoscope">goatstuff</a>.</sub>
</p>

> [!NOTE]
> goat is pre-1.0: interfaces can still change between releases.
> To build on goat, use the [`stable`](https://github.com/usareur-af/goatcore/tree/stable) branch or the `goatcore/v` tag it points at. To contribute, open a pull request against `main`. [How the two fit together](docs/design/community-and-stable-line.md).

## What goat is

goat connects people, machines and services across sites into one network, and lets them publish and subscribe to data with access decided by who they are, not where they sit. It is built for places where connectivity is poor or intermittent: once a device has joined, it keeps talking to its peers directly, with or without the cloud and with or without the central services that enrolled it.

goat is an integration of established open-source projects, assembled and operated as one system:

| Concern | Built on |
|---|---|
| Network | [WireGuard](https://www.wireguard.com/) peer-to-peer tunnels, managed by [NetBird](https://github.com/netbirdio/netbird), with relays for peers that cannot reach each other directly |
| Data | [Eclipse Zenoh](https://zenoh.io/) publish/subscribe with no central broker |
| Identity | [Ory](https://www.ory.sh/) for people and services, [SPIFFE](https://spiffe.io/) for workloads, an offline certificate authority for machines |
| Site state | PostgreSQL, replicated active-active between sites |

On top of those, this repository provides two command-line tools: `goatbuilder` builds, releases and deploys a goat environment, and `goat` operates one.

## What you can rely on

- **Nothing sensitive faces the internet.** Every service listens only inside the encrypted network. The public internet sees DNS records, not open ports.
- **Joined devices keep working when the centre goes down.** The management service is needed to enrol a device, not to keep it connected. If it fails, peers carry on talking to each other.
- **There is no central data broker to lose.** Data moves between publishers and subscribers directly, so no single server stands between them.
- **Machines pull their configuration.** Each machine fetches what it is meant to run and reports what it is running; a deployment is finished when every machine's report matches the declaration.
- **Every error says what to do next.** Each message from `goat` and `goatbuilder` states what went wrong, why, and how to fix it; `goat explain` takes an error code or pasted output.

## Try it

The fastest way to see goat is a rehearsal: `goatbuilder` stands up a complete example environment against a simulated set of machines, on your laptop, in seconds. Nothing touches a network. Download `goatbuilder` for Linux or macOS from the [releases](https://github.com/usareur-af/goatcore/releases) and run it from a clone of this repository:

```bash
git clone https://github.com/usareur-af/goatcore && cd goatcore

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"; ARCH="$(uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')"
curl -fsSL -o goatbuilder "https://github.com/usareur-af/goatcore/releases/download/goatbuilder/v0.6.1/goatbuilder-$OS-$ARCH"
chmod +x goatbuilder

WORK="$(mktemp -d)/zagros"
cp -R ops/goatbuilder/fixtures/goatnet-from-scratch/01-founder "$WORK"
./goatbuilder standup zagros --site alpha --mock "$WORK" --ack "you" --yes
./goatbuilder status --env zagros --mock "$WORK"
```

Each release lists a SHA-256 checksum per binary in `SHA256SUMS`, signed with cosign. To build it yourself instead, `cd crates && cargo build --release -p goatbuilder` puts it in `crates/target/release/`; the first build often takes half an hour.

The example environment, `zagros`, has one site with two core machines, one monitoring machine and one recovery machine. [Tutorial 1](docs/tutorials/01-new-goatnet.md) explains each step and carries on to real hardware.

To run goat for real on a single machine with no cloud account, start with [goatlite](docs/tutorials/00-goatlite.md).

To have a coding agent do it with you, give it this line with your own values: `Clone https://github.com/usareur-af/goatcore and follow .claude/skills/goatlite/SKILL.md to stand up a goatlite named <name> on <ssh-user>@<host>, with <your-email> as its administrator.`

## Documentation

| | |
|---|---|
| **Learn** | [goatlite: one machine, one file](docs/tutorials/00-goatlite.md) · [A new environment](docs/tutorials/01-new-goatnet.md) · [A second site](docs/tutorials/02-first-federated-node.md) · [Sharing with a partner](docs/tutorials/03-boundary-node.md) |
| **Use** | [Stand up a goatnet](docs/operations/goatnet-standup.md) · [The `goat` CLI](docs/goat-cli-book/src/getting-started.md) · [`goatbuilder`](docs/reference/goatbuilder/01-what-it-is.md) · [Operations runbooks](docs/operations/) |
| **Understand** | [Architecture overview](https://github.com/usareur-af/goatdocs/blob/main/docs/design/architecture-spine.md) · [Component designs](docs/design/) · [Decision record](docs/adr/INDEX.md) |
| **This release** | [What is included and what you provide](docs/RELEASE-CONTENTS.md) |

## Related repositories

| Repository | Contents |
|---|---|
| [goatenv](https://github.com/usareur-af/goatenv) | A complete example environment, kept separate from the code it deploys |
| [hello-galaxy-goatenv](https://github.com/usareur-af/hello-galaxy-goatenv) | A step-by-step walkthrough that builds that environment |
| [goatstuff](https://github.com/usareur-af/goatstuff) | Inspection tools and demonstration applications |
| [goatdocs](https://github.com/usareur-af/goatdocs) | Architecture, strategy, design system and integration guides |
| [zagros-moon-pod](https://github.com/usareur-af/zagros-moon-pod) | A pod a partner runs to join a goat data fabric and share data under their own custody |

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The `goat` and `goatbuilder` command-line tools and their shared libraries (Rust) |
| `ops/` | Ansible roles and playbooks, the portal and identity console, enrollment tooling, and the CI checks |
| `goat-relay/`, `signal-server/` | The relay and the signalling server |
| `terraform/` | Reusable cloud modules |
| `docs/` | Tutorials, runbooks, component designs and the decision record |

## Contributing

Pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before you start; every CI check is a script under `ops/ci/` that you can run locally. Report security issues privately as described in [SECURITY.md](SECURITY.md).

## License

goat was written by U.S. Federal government employees. In the U.S. it is not subject to copyright and is in the public domain; in countries where copyright is recognized, it is released under the [Apache License 2.0](LICENSE). See [INTENT.md](INTENT.md). Third-party components and their licences are listed in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md); see also [NOTICE](NOTICE).
