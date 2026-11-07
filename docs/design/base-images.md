# Base images by role

**Status:** Built. Every goatcore, goatstuff and goatrodeo Dockerfile names its base images through a role; the gate `base-images` runs on every pull request that touches a Dockerfile, a role file or an Ansible role. Decision: [ADR 1220](../adr/1220-public-base-images-by-role-overlay-supplies-its-own.md).
**Related:** [ADR 0700](../adr/0700-chainguard-first.md) · [ADR 0703](../adr/0703-pinned-digests-and-pin-regression-ci.md) · [ADR 1197](../adr/1197-public-core-versioned-dependency-private-overlay-repo.md) · how to use your own bases: [`../operations/build-on-your-own-base-images.md`](../operations/build-on-your-own-base-images.md)

## BLUF

A goat image starts from a base image chosen by role: the Rust build stage, the Go build stage, the runtime of a glibc binary, and so on. The public repositories choose Chainguard's free images, which anyone pulls without an account, each pinned by digest. An environment that wants other bases, such as a private Chainguard catalogue or its own mirror, writes its own one-line file for a role in its overlay, and every image it builds uses that file. No Dockerfile changes.

## The roles

Each role is the file `ops/base-images/<role>/Dockerfile`, holding exactly one `FROM` line pinned by digest. The build argument is `BASE_` plus the role in capitals.

| Role | Build argument | Used for | Public default |
|---|---|---|---|
| `rust-build` | `BASE_RUST_BUILD` | Rust build stages | `cgr.dev/chainguard/rust:latest-dev` |
| `go-build` | `BASE_GO_BUILD` | Go build stages, `CGO_ENABLED=0` | `cgr.dev/chainguard/go:latest` |
| `node-build` | `BASE_NODE_BUILD` | the panoscope web build (goatstuff) | `cgr.dev/chainguard/node:latest-dev` |
| `runtime-cc` | `BASE_RUNTIME_CC` | runtime of a glibc-linked binary | `cgr.dev/chainguard/glibc-dynamic:latest` |
| `runtime-static` | `BASE_RUNTIME_STATIC` | runtime of a static binary | `cgr.dev/chainguard/static:latest` |
| `runtime-shell` | `BASE_RUNTIME_SHELL` | a runtime that runs scripts (`onboard-runner`) | `cgr.dev/chainguard/wolfi-base:latest` |

goatcore defines all but `node-build`; goatstuff defines `rust-build`, `node-build` and `runtime-cc`; goatrodeo defines `go-build` and `runtime-static`. A role file and `base-images.py` are byte-identical wherever two repositories carry them; the assembly refuses a shared path that differs.

The free catalogue tags only `latest` and `latest-dev`. The digest is the pin: a rebuild of the same commit starts from the same bytes, and Dependabot moves the digest when Chainguard rebuilds.

Every runtime runs as user 65532. `runtime-cc` and `runtime-static` carry no shell and no package manager. `runtime-shell` carries a shell and `apk`; `ops/onboard-runner/Dockerfile` installs its packages with `apk`, or `apt-get` on a Debian base, and needs glibc 2.39 or newer because it runs the host's `goat` binary, built for Ubuntu 24.04.

A binary runs only on a glibc at least as new as the one it was built against. An environment that replaces `rust-build` replaces `runtime-cc` in the same change.

## How a Dockerfile names its bases

```dockerfile
ARG BASE_RUST_BUILD=cgr.dev/chainguard/rust:latest-dev@sha256:5412…
ARG BASE_RUNTIME_CC=cgr.dev/chainguard/glibc-dynamic:latest@sha256:671a…

FROM ${BASE_RUST_BUILD} AS rust-builder
…
FROM ${BASE_RUNTIME_CC}
```

The `ARG` lines sit before the first `FROM`, and their defaults equal the role files, so `docker build` with no arguments builds on the public bases. `python3 ops/base-images/base-images.py sync` rewrites the defaults from the role files.

## How a build picks the bases

Every image build passes the role files of the tree it builds in:

| Build | Where it runs | Which role files |
|---|---|---|
| `_image-build.yml` (goatcore, goatstuff and goatrodeo, one copy each) | the public repository's CI, publishing `ghcr.io/usareur-af/<image>` | the public defaults |
| `mgmt-host-bootstrap.yml` | goatcore CI | the public defaults |
| an environment's own build | the environment's CI or a workstation, in an assembled tree | the environment's overlay files where present, the public defaults elsewhere |

The step is `python3 ops/base-images/base-images.py args --dockerfile <file> --github-output`. It prints `BASE_<ROLE>=<image>` for each base the Dockerfile declares, passes them as `build-args`, and labels the image `org.opencontainers.image.base.name` with the image its final stage starts from. A Dockerfile that declares a base with no role file fails the build.

An assembled tree is goatcore (or goatstuff) with the environment's repository copied over it. A role file in the overlay replaces the public one at the same path. `base-images.py build --unit <stem> --tag <ref>` builds one image in such a tree with `docker buildx`, reading context, Dockerfile and target from `.github/workflows/<stem>.yml`.

## Refresh

Dependabot watches each role directory (`.github/dependabot.yml`) and opens a pull request when the pinned tag has a newer build. The `base-images` check on that pull request fails until `base-images.py sync` has rewritten the Dockerfile defaults. Pushing the sync rewrites exactly the Dockerfiles that use the role, and their image workflows' `paths` filters rebuild exactly those images.

## The gate

`.github/workflows/base-images.yml` runs `base-images.py --self-test`, then `base-images.py check`. The check fails when:

- a role file holds anything but one digest-pinned `FROM`, or names a subscription registry: a private Chainguard organisation `cgr.dev/<org>/` or `registry1.dso.mil`. The free catalogue `cgr.dev/chainguard/` is not a subscription;
- a Dockerfile declares `ARG BASE_<X>` with no role `x`, uses `FROM ${BASE_<X>}` without declaring it, or has a default that differs from its role file;
- a Dockerfile `FROM`, a role default or a compose `image:` under `ops/ansible/` names a subscription registry and is not listed in `ops/base-images/vendor-refs.allow`;
- a line in `vendor-refs.allow` no longer matches anything.

`vendor-refs.allow` is empty. The check runs only in the public repositories; an overlay's role files may name any registry its owner has access to.
