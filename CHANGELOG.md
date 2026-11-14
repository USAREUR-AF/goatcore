# Changelog

This repository releases several things, and each has its own tag and its own record of what changed. This page says where to find each one. It holds no entries itself.

## What to build on

| Name | What it is | Use it when |
|---|---|---|
| `goatcore/vX.Y.Z` | A release of the whole repository: a signed tag at the commit of the goatbuilder release with the same version, cut after that commit has run in a standing environment | You want a fixed reference. Pin this tag |
| `stable` | A branch that always points at the newest `goatcore/v` tag. It takes no pull requests | You want the newest release by a name that does not change |
| `main` | The one development branch. Every change lands here | You are contributing |

A whole-tree release is cut at each goatbuilder minor release (`X.Y.0`). It is cut at a patch release only when that patch carries a fix that people on the stable line need. Its release page lists the newest release of each component in that commit, and the versions of other projects it works with ([`ops/release/works-with.md`](ops/release/works-with.md)). The decision is [ADR 1231](docs/adr/1231-one-development-branch-and-a-stable-line-that-follows-releases.md), and the design is [community-and-stable-line.md](docs/design/community-and-stable-line.md).

## Component tags

Each component tag publishes one component's binaries or package from the commit it names. The two kinds of tag share a version only where ADR 1231 says so: `goatcore/vX.Y.Z` and `goatbuilder/vX.Y.Z` are one commit and one version.

| Tag | Component | What changed is recorded in | Published by |
|---|---|---|---|
| `goatbuilder/vX.Y.Z` | `goatbuilder`, the build, release, deploy and standup tool | [`crates/goatbuilder/CHANGELOG.md`](crates/goatbuilder/CHANGELOG.md) | `.github/workflows/goatbuilder-release.yml` |
| `goat-cli-vX.Y.Z` | `goat`, the operator CLI | [`docs/goat-cli-book/src/release-notes.md`](docs/goat-cli-book/src/release-notes.md) | `.github/workflows/goat-release.yml` |
| `grant-crypto-sdk-vX.Y.Z` | the grant-crypto SDK: its C ABI and the Go, Python and TypeScript bindings | the release page | `.github/workflows/grant-crypto-sdk-release.yml` |
| `wg-cp0/vX.Y.Z` | `wg-cp0`, the WireGuard control-plane command | the release page (no `wg-cp0/v` tag has been cut yet) | `.github/workflows/wg-cp0-release.yml` |
| `goat-relay/vX.Y.Z` | the goat-relay container image | nowhere: the tag publishes the image to `ghcr.io/usareur-af/goat-relay` and makes no release page | `.github/workflows/goat-relay-image.yml` |
| `snitch-vX.Y.Z` | `snitch`, its binaries for six platforms | the release page | `.github/workflows/snitch.yml` |

Every release page is at <https://github.com/USAREUR-AF/goatcore/releases>. The text that opens each component's page, with its assets and how to verify them, is in [`.github/release-notes/`](.github/release-notes/).

## How goatbuilder's changelog is written

Each change adds one file under [`crates/goatbuilder/changelog.d/`](crates/goatbuilder/changelog.d/README.md). `ops/release/changelog.py write` renders them into the Unreleased section of `crates/goatbuilder/CHANGELOG.md`, and the version pull request runs `ops/release/changelog.py release <version> <date>`, which writes that version's section and removes the entry files.
