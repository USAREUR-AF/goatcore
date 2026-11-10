# Build goat images on your own base images

goat's public images are built on Chainguard's free images, which anyone pulls without an account, each pinned by digest. Your environment can build every goat image on different bases instead: a private Chainguard catalogue (versioned tags, FIPS variants), your organisation's mirror, or images you build yourself. You write one short file per base in your environment's repository; you change no Dockerfile. The goat team's own goatnets build this way, from role files in their environment repository. The mechanism is [`../design/base-images.md`](../design/base-images.md), the decision [ADR 1220](../adr/1220-public-base-images-by-role-overlay-supplies-its-own.md).

## What you need

- An environment repository shaped like [`usareur-af/goatenv`](https://github.com/usareur-af/goatenv): it is copied over a goatcore checkout to make the tree you build and deploy from.
- Pull access to the images you choose, and a registry you can push to.
- `docker buildx`, `python3` and `crane` (or `docker buildx imagetools`) on the machine that builds.

## 1. See the roles

In a goatcore checkout at your `goatcore.ref`:

```bash
python3 ops/base-images/base-images.py list
```

```
go-build        BASE_GO_BUILD        cgr.dev/chainguard/go:latest@sha256:…
runtime-cc      BASE_RUNTIME_CC      cgr.dev/chainguard/glibc-dynamic:latest@sha256:…
runtime-shell   BASE_RUNTIME_SHELL   cgr.dev/chainguard/wolfi-base:latest@sha256:…
runtime-static  BASE_RUNTIME_STATIC  cgr.dev/chainguard/static:latest@sha256:…
rust-build      BASE_RUST_BUILD      cgr.dev/chainguard/rust:latest-dev@sha256:…
```

goatstuff has its own set: `rust-build`, `node-build` and `runtime-cc`; goatrodeo uses `go-build` and `runtime-static`.

## 2. Write a role file for each base you replace

A role file is `ops/base-images/<role>/Dockerfile` in your environment repository, holding one `FROM` line pinned by digest. Get the digest with `crane digest <image>:<tag>`. For a private Chainguard catalogue under the organisation `example-org`, the files look like this; pick the tags and variants your organisation subscribes to:

| File | Line |
|---|---|
| `ops/base-images/rust-build/Dockerfile` | `FROM cgr.dev/example-org/rust:1.92-dev@sha256:<digest>` |
| `ops/base-images/go-build/Dockerfile` | `FROM cgr.dev/example-org/go-fips:1.26-dev@sha256:<digest>` |
| `ops/base-images/node-build/Dockerfile` | `FROM cgr.dev/example-org/node:22-dev@sha256:<digest>` |
| `ops/base-images/runtime-cc/Dockerfile` | `FROM cgr.dev/example-org/glibc-dynamic-fips:latest@sha256:<digest>` |
| `ops/base-images/runtime-static/Dockerfile` | `FROM cgr.dev/example-org/static-fips:latest@sha256:<digest>` |
| `ops/base-images/runtime-shell/Dockerfile` | `FROM cgr.dev/example-org/wolfi-base:latest@sha256:<digest>` |

Rules your bases must meet, because the Dockerfiles rely on them:

| Role | Must provide |
|---|---|
| `rust-build`, `go-build`, `node-build` | the toolchain on `PATH`, a shell, and a user that may write the work directory (the Rust and Node stages switch to `USER root`) |
| `runtime-cc` | glibc at least as new as `rust-build`'s, libgcc, CA certificates at `/etc/ssl/certs/ca-certificates.crt`, user 65532 |
| `runtime-static` | CA certificates, user 65532 |
| `runtime-shell` | `sh`, `apt-get` or `apk`, glibc 2.39 or newer |

Replace `rust-build` and `runtime-cc` together: a binary built against a newer glibc does not start on an older one.

Your environment repository is private or public as you choose. Naming a vendor's image in it redistributes nothing; whoever builds pulls the image from the vendor under their own access. A build that pulls from a private catalogue logs in to it first (`docker login cgr.dev`, or the CI's equivalent with a pull token).

## 3. Assemble and check

Copy your environment's `ops/` over goatcore, as you do to deploy, and confirm the build sees your files:

```bash
git clone --branch "$(cat my-env/goatcore.ref)" https://github.com/usareur-af/goatcore.git tree
cp -R my-env/ops tree/
python3 tree/ops/base-images/base-images.py list
```

`list` now shows your images for the roles you replaced and the public defaults for the rest.

## 4. Build and pin

Build an image by the stem of its workflow in `.github/workflows/` (`goat-relay-image`, `goat-zenoh-portal-image`, …):

```bash
cd tree
python3 ops/base-images/base-images.py build --unit goat-relay-image \
  --tag registry.example.org/goat/goat-relay:local-1 \
  --platform linux/amd64,linux/arm64 --push
crane digest registry.example.org/goat/goat-relay:local-1
```

Put the digest in your environment's `ops/pins/fleet-pins.yaml` in place of the public image, and deploy as usual. `docker buildx imagetools inspect` shows the label `org.opencontainers.image.base.name`, the base the image's final stage started from.

For a goatstuff image, do the same in a goatstuff checkout at your `goatstuff.ref` with your environment copied over it.

In CI, call `python3 ops/base-images/base-images.py args --dockerfile <file> --github-output` before `docker/build-push-action` and pass `build-args: ${{ steps.<id>.outputs.build_args }}`, as goatcore's `.github/workflows/_image-build.yml` does.

## 5. Keep the digests fresh

Add each of your role directories to your repository's `.github/dependabot.yml` under the `docker` ecosystem:

```yaml
version: 2
updates:
  - package-ecosystem: docker
    directories:
      - /ops/base-images/rust-build
      - /ops/base-images/runtime-cc
    schedule:
      interval: weekly
```

Dependabot then opens a pull request whenever a pinned tag has a newer build; merging it and rebuilding moves your images to the new base.

## Other ways to get hardened bases

- **Build your own from Wolfi.** [apko](https://github.com/chainguard-dev/apko) assembles a minimal image from [Wolfi](https://github.com/wolfi-dev) packages, both open source. Push the result to your registry and name it in the role file.
- **Your organisation's mirror.** Name the mirrored image and digest in the role file; nothing else changes.
