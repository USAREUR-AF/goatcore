# ADR 1220 — Base images are named by role: the public tree defaults to Chainguard's free catalogue, and an environment's overlay supplies its own

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-11-06, revised 2026-11-07 |
| Deciders | operator |
| Repo scope | Multi (goatcore, goatstuff, goatrodeo; every overlay that builds images) |
| Amends | [ADR 0700](0700-chainguard-first.md) (a private Chainguard catalogue belongs in an environment's overlay, never in the public tree's defaults) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0703](0703-pinned-digests-and-pin-regression-ci.md) (digest pins), [ADR 1197](1197-public-core-versioned-dependency-private-overlay-repo.md) (public core, private overlay), [`../design/base-images.md`](../design/base-images.md), [`../operations/build-on-your-own-base-images.md`](../operations/build-on-your-own-base-images.md) |

## TL;DR (for a layperson)

Every goat container image is built on a few standard starting images: one for compiling Rust, one for Go, one small image the finished program runs in. The public repositories start from Chainguard's free images, which anyone can download without an account. An organisation that wants other starting images, such as hardened images from a paid Chainguard catalogue or its own mirror, writes one line per starting image in its own configuration, and every image it builds uses them. Paid images never appear in the public project, and nobody has to fork a Dockerfile to choose differently.

## Context

Chainguard publishes two kinds of image. Its free catalogue, `cgr.dev/chainguard/*`, is public: anyone pulls it anonymously, and it is tagged only `latest` and `latest-dev`. A paid catalogue lives under the subscriber's own organisation, `cgr.dev/<org>/*`, needs credentials, carries versioned tags and FIPS variants, and its terms do not let an open-source project redistribute images built on it. goat's public Dockerfiles named the free catalogue directly in their `FROM` lines, thirteen of fourteen on a floating `:latest`, so no build was reproducible and an environment that wanted its paid catalogue had to fork every Dockerfile.

goat builds an environment's images from an assembled tree: the goatcore checkout with the environment's overlay copied over it ([ADR 1197](1197-public-core-versioned-dependency-private-overlay-repo.md)). A file in the overlay at the same path replaces the public one, which makes the overlay the natural place for an environment's choice of base images.

## Decision

- **Roles.** A base image is named by its role, not its vendor. Each role is one file, `ops/base-images/<role>/Dockerfile`, holding one digest-pinned `FROM` line. goatcore defines `rust-build`, `go-build`, `runtime-cc`, `runtime-static` and `runtime-shell`; goatstuff adds `node-build`; goatrodeo uses `go-build` and `runtime-static`. A role file shared by two repositories is byte-identical in both, which the assembly enforces.
- **Dockerfiles take bases as build arguments.** A Dockerfile declares `ARG BASE_<ROLE>=<default>` before its first `FROM` and starts each stage `FROM ${BASE_<ROLE>}`. The default equals the role file, so a plain `docker build` uses the same bases CI does.
- **Public defaults are Chainguard's free catalogue.** `rust:latest-dev`, `go:latest`, `node:latest-dev`, `glibc-dynamic:latest`, `static:latest` and `wolfi-base:latest`, each pinned by digest. The free catalogue tags only `latest`; the digest is what makes a build reproducible, and Dependabot moves it when Chainguard rebuilds.
- **An environment overrides by file.** An overlay that wants other bases carries its own `ops/base-images/<role>/Dockerfile` for each role it changes. Its build reads them through `ops/base-images/base-images.py args`, which every image build calls (`_image-build.yml`, and the overlay's equivalent). Build and runtime roles that link glibc are overridden together, so a binary never meets an older glibc than it was built against.
- **A private catalogue lives only in an overlay.** The goat team's environment repository carries its own role file for every role and refuses to build when the public tree has a role it lacks, so its images build on the images it chooses, including a private Chainguard catalogue once it uses one.
- **Refresh.** Dependabot refreshes each role file's digest (`.github/dependabot.yml`). `base-images.py sync` then rewrites the Dockerfile defaults, which rebuilds exactly the images whose base moved.
- **Gate.** `base-images.py check` (workflow `base-images`) fails when a role is unpinned or on a subscription registry (a private Chainguard organisation `cgr.dev/<org>/`, or `registry1.dso.mil`), when a Dockerfile names a base outside a role or its default is out of sync, and when a role default or compose template names a subscription registry. Exceptions are listed with their reasons in `ops/base-images/vendor-refs.allow`, a list that only shrinks; today it is empty.

## Alternatives considered

- **Keep the free catalogue in `FROM` lines.** Every environment that wants a paid catalogue forks every Dockerfile, and nothing pins a digest. Rejected.
- **Neutral non-Chainguard defaults (Debian official images, distroless).** Moves every public image off the hardened base goat's own goatnets run, with no licence gain: the free catalogue is public. Rejected.
- **Rebuild Chainguard-equivalent images ourselves from Wolfi packages with apko.** Possible, and an environment may do it through the same role files. As the public default it would make goat a base-image publisher with its own rebuild and CVE duty. Rejected as the default; documented as an option.
- **The overlay carries its own copies of the Dockerfiles.** About thirty forked files would drift from the public ones on every change. Rejected.

## Consequences

**Positive.** Public images and the goat team's images start from the same hardened bases. Every base is pinned by digest, so a rebuild of a commit is reproducible. Moving to a private catalogue is a role-file change in the overlay, with no fork and no public trace.

**Negative.** A Dependabot refresh needs one `sync` push before it can merge. A free-catalogue digest is reproducible only while Chainguard keeps serving it; an old digest that stops resolving is refreshed, not pinned forever.

## Assessor view (SCA-R)

SR-3 and SR-4 hold per environment: the base images an environment runs are declared in that environment's repository, digest-pinned and visible in each image's `org.opencontainers.image.base.name` label. CM-2 improves because no build floats on a mutable tag. The residual risk is an overlay that forgets a role; the goat team's build refuses that case, and any build records the base it used in the image label.

## Trigger conditions for revisit

- Chainguard stops publishing an image the roles use in its free catalogue, or requires an account for it.
- A goatnet needs a FIPS-validated base: that is an overlay role file naming a private catalogue, and this ADR needs no change.
