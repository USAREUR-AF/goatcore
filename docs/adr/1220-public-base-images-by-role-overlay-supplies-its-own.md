# ADR 1220 — Base images are named by role: the public tree defaults to images anyone may redistribute, and an environment's overlay supplies its own

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-11-06 |
| Deciders | operator |
| Repo scope | Multi (goatcore, goatstuff; every overlay that builds images) |
| Amends | [ADR 0700](0700-chainguard-first.md) (Chainguard-first now applies to an environment's overlay, not to the public tree's defaults) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0703](0703-pinned-digests-and-pin-regression-ci.md) (digest pins), [ADR 1197](1197-public-core-versioned-dependency-private-overlay-repo.md) (public core, private overlay), [`../design/base-images.md`](../design/base-images.md), [`../operations/build-on-your-own-base-images.md`](../operations/build-on-your-own-base-images.md) |

## TL;DR (for a layperson)

Every goat container image is built on top of a few standard starting images: one for compiling Rust, one for Go, one small image the finished program runs in. The public repositories now start from images that anyone may download and pass on: Debian-based official images and Google's distroless images. An organisation that wants hardened images from a commercial catalogue, as the goat team does with Chainguard for its own networks, writes one line per starting image in its own configuration, and every image it builds uses them. The public project never ships anything built on a paid catalogue's images, and nobody has to fork a Dockerfile to choose differently.

## Context

Until this decision every public Dockerfile started from `cgr.dev/chainguard/*` images. Chainguard's free catalogue is a limited, changing offer, and Chainguard publishes no licence that lets an open-source project redistribute images built on it. That left the public build two problems: it depended on an offer the vendor can narrow, and every image published under `ghcr.io/usareur-af` carried Chainguard's layers. Thirteen of fourteen `FROM` lines also floated on `:latest`, the only tag the free catalogue offers, so no build could be reproduced.

goat's own goatnets build from an assembled tree: the goatcore checkout with the environment's overlay copied over it ([ADR 1197](1197-public-core-versioned-dependency-private-overlay-repo.md)). A file in the overlay at the same path replaces the public one. That is the natural place for an environment's choice of base images.

## Decision

- **Roles.** A base image is named by its role, not by its vendor. Each role is one file, `ops/base-images/<role>/Dockerfile`, holding one digest-pinned `FROM` line. goatcore defines `rust-build`, `go-build`, `runtime-cc`, `runtime-static` and `runtime-shell`; goatstuff defines `rust-build`, `node-build` and `runtime-cc`.
- **Dockerfiles take bases as build arguments.** A Dockerfile declares `ARG BASE_<ROLE>=<default>` before its first `FROM` and starts each stage `FROM ${BASE_<ROLE>}`. The default equals the role file, so a plain `docker build` uses the same bases CI does.
- **Public defaults are redistributable.** In the public repositories every role names an image anyone may pull and redistribute, pinned by digest: `docker.io/library/rust`, `golang`, `node` and `python` official images, and `gcr.io/distroless/{cc,static}-debian13:nonroot`. Runtimes run as user 65532 with no shell, as before.
- **An environment overrides by file.** An overlay that wants other bases carries its own `ops/base-images/<role>/Dockerfile` for each role it changes. Its build reads them through `ops/base-images/base-images.py args`, which every image build calls (`_image-build.yml`, and the overlay's equivalent). Build and runtime roles that link glibc are overridden together, so a binary never meets an older glibc than it was built against.
- **The goat team's own goatnets keep Chainguard.** The team's environment repository carries Chainguard role files and builds every image its goatnets run from the assembled tree, under its own registry namespace. Images published under `ghcr.io/usareur-af` are built from the public defaults.
- **Refresh.** Dependabot refreshes each role file's digest (`.github/dependabot.yml`). `base-images.py sync` then rewrites the Dockerfile defaults, which also rebuilds exactly the images whose base moved.
- **Gate.** `base-images.py check` (workflow `base-images`) fails when a role is unpinned or on a subscription registry (`cgr.dev`, `registry1.dso.mil`), when a Dockerfile names a base outside a role or its default is out of sync, and when a role default or compose template names a subscription registry. The only waived references are listed with their reasons in `ops/base-images/vendor-refs.allow`, a list that only shrinks. Today it holds the two MinIO images: upstream MinIO no longer publishes them, so the public role default points at Chainguard's free image, which each user pulls from Chainguard.

## Alternatives considered

- **Keep Chainguard defaults in the public tree.** No published licence covers redistribution, the free catalogue only offers `latest`, and its scope has changed before. Rejected.
- **Rebuild Chainguard-equivalent images ourselves from Wolfi packages with apko.** apko and Wolfi are open source, so this is possible, and an environment may do it through the same role files. As the public default it would make goat a base-image publisher with its own rebuild and CVE duty. Rejected as the default; documented as an option.
- **Alpine as the default.** Alpine uses musl. goat's Rust binaries link glibc, so this would change how they build and run. Rejected.
- **The overlay carries its own copies of the Dockerfiles.** About thirty forked files would drift from the public ones on every change. Rejected.

## Consequences

**Positive.** The public images are reproducible (every base pinned by digest) and free of third-party licence questions. Anyone can match goat's own hardening by writing a few one-line files, with no fork. One script and one gate serve goatcore, goatstuff and every overlay.

**Negative.** The goat team's goatnets keep building their images in their overlay instead of pinning the public builds, so the team keeps a private image namespace for images built on Chainguard. A Dependabot refresh needs one `sync` push before it can merge. The public defaults and goat's own images differ in base, so a defect specific to one base family appears in only one of them.

## Assessor view (SCA-R)

SR-3 and SR-4 now hold per environment: the base images an environment runs are declared in that environment's own repository, digest-pinned and visible in each image's `org.opencontainers.image.base.name` label. CM-2 improves because no build floats on a mutable tag. The goat team's goatnets keep the Chainguard posture ADR 0700 describes. The residual risk is that an overlay forgets a role; the build then uses the public default, which the image label records.

## Trigger conditions for revisit

- Chainguard publishes terms that allow redistribution of images built on its free catalogue.
- A public, maintained MinIO image appears (the waiver list then empties).
- A goatnet needs a FIPS-validated base; that is an overlay role file, and this ADR needs no change unless the public tree must ship one.
