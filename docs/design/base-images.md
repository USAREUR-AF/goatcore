# Base images by role

**Status:** Built. Every goatcore and goatstuff Dockerfile that used a Chainguard base names its bases through a role; the gate `base-images` runs on every pull request that touches a Dockerfile, a role file or an Ansible role. Decision: [ADR 1220](../adr/1220-public-base-images-by-role-overlay-supplies-its-own.md).
**Related:** [ADR 0700](../adr/0700-chainguard-first.md) · [ADR 0703](../adr/0703-pinned-digests-and-pin-regression-ci.md) · [ADR 1197](../adr/1197-public-core-versioned-dependency-private-overlay-repo.md) · how to use your own bases: [`../operations/build-on-your-own-base-images.md`](../operations/build-on-your-own-base-images.md)

## BLUF

A goat image starts from a base image chosen by role: the Rust build stage, the Go build stage, the runtime of a glibc binary, and so on. The public repositories choose images anyone may pull and redistribute. An environment that wants other bases, such as a hardened commercial catalogue, writes its own one-line file for a role in its overlay, and every image it builds uses that file. No Dockerfile changes.

## The roles

Each role is the file `ops/base-images/<role>/Dockerfile`, holding exactly one `FROM` line pinned by digest. The build argument is `BASE_` plus the role in capitals.

| Role | Build argument | Used for | Public default (repository) |
|---|---|---|---|
| `rust-build` | `BASE_RUST_BUILD` | Rust build stages | `docker.io/library/rust:1.92-bookworm` (goatcore, goatstuff) |
| `go-build` | `BASE_GO_BUILD` | Go build stages, `CGO_ENABLED=0` | `docker.io/library/golang:1.26-bookworm` (goatcore) |
| `node-build` | `BASE_NODE_BUILD` | the panoscope web build | `docker.io/library/node:22-bookworm-slim` (goatstuff) |
| `runtime-cc` | `BASE_RUNTIME_CC` | runtime of a glibc-linked binary | `gcr.io/distroless/cc-debian13:nonroot` (goatcore, goatstuff) |
| `runtime-static` | `BASE_RUNTIME_STATIC` | runtime of a static binary | `gcr.io/distroless/static-debian13:nonroot` (goatcore) |
| `runtime-shell` | `BASE_RUNTIME_SHELL` | a runtime that runs scripts (`onboard-runner`) | `docker.io/library/python:3.13-slim-trixie` (goatcore) |

Every runtime runs as user 65532. `runtime-cc` and `runtime-static` carry no shell and no package manager. `runtime-shell` carries a shell and either `apt-get` or `apk`; `ops/onboard-runner/Dockerfile` installs its packages with whichever is present, and needs glibc 2.39 or newer because it runs the host's `goat` binary, built for Ubuntu 24.04.

A binary runs only on a glibc at least as new as the one it was built against. The public pair (`rust-build` on Debian 12, `runtime-cc` on Debian 13) satisfies that. An environment that replaces `rust-build` replaces `runtime-cc` in the same change.

## How a Dockerfile names its bases

```dockerfile
ARG BASE_RUST_BUILD=docker.io/library/rust:1.92-bookworm@sha256:e90e…
ARG BASE_RUNTIME_CC=gcr.io/distroless/cc-debian13:nonroot@sha256:0847…

FROM ${BASE_RUST_BUILD} AS rust-builder
…
FROM ${BASE_RUNTIME_CC}
```

The `ARG` lines sit before the first `FROM`, and their defaults equal the role files, so `docker build` with no arguments builds on the public bases. `python3 ops/base-images/base-images.py sync` rewrites the defaults from the role files.

## How a build picks the bases

Every image build passes the role files of the tree it builds in:

| Build | Where it runs | Which role files |
|---|---|---|
| `_image-build.yml` (goatcore and goatstuff, one copy each) | the public repository's CI, publishing `ghcr.io/usareur-af/<image>` | the public defaults |
| `mgmt-host-bootstrap.yml` | goatcore CI | the public defaults |
| an environment's own build | the environment's CI or a workstation, in an assembled tree | the environment's overlay files where present, the public defaults elsewhere |

The step is `python3 ops/base-images/base-images.py args --dockerfile <file> --github-output`. It prints `BASE_<ROLE>=<image>` for each base the Dockerfile declares, passes them as `build-args`, and labels the image `org.opencontainers.image.base.name` with the image its final stage starts from. A Dockerfile that declares a base with no role file fails the build.

An assembled tree is goatcore (or goatstuff) with the environment's repository copied over it. A role file in the overlay replaces the public one at the same path. `base-images.py build --unit <stem> --tag <ref>` builds one image in such a tree with `docker buildx`, reading context, Dockerfile and target from `.github/workflows/<stem>.yml`.

## Refresh

Dependabot watches each role directory (`.github/dependabot.yml`) and opens a pull request when the pinned tag has a newer build. The `base-images` check on that pull request fails until `base-images.py sync` has rewritten the Dockerfile defaults. Pushing the sync rewrites exactly the Dockerfiles that use the role, and their image workflows' `paths` filters rebuild exactly those images.

## The gate

`.github/workflows/base-images.yml` runs `base-images.py --self-test`, then `base-images.py check`. The check fails when:

- a role file holds anything but one digest-pinned `FROM`, or names a subscription registry (`cgr.dev`, `registry1.dso.mil`);
- a Dockerfile declares `ARG BASE_<X>` with no role `x`, uses `FROM ${BASE_<X>}` without declaring it, or has a default that differs from its role file;
- a Dockerfile `FROM`, a role default or a compose `image:` under `ops/ansible/` names a subscription registry and is not listed in `ops/base-images/vendor-refs.allow`;
- a line in `vendor-refs.allow` no longer matches anything.

`vendor-refs.allow` holds the deployed third-party images that have no redistributable public build. It lists two: `minio_image` and `minio_mc_image` in `ops/ansible/roles/minio/defaults/main.yml`. MinIO no longer publishes its own images, so the public default is Chainguard's free MinIO image, which each user pulls from Chainguard.

The check runs only in the public repositories. An overlay's role files may name any registry its owner has access to.
