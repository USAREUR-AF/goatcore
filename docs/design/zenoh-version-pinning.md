# Zenoh project-wide version pinning

| Field | Value |
|---|---|
| Status | Active (2026-05-18). Version-of-record `=1.9.0`. ADR 0858 step 5 retired the goat-maintainer image build in favor of upstream `eclipse/zenoh` + bind-mounted upstream rocksdb backend `.so`. |
| Scope | Every place zenoh appears in this tree: Rust crate deps, Cargo workspace pins, the Ansible role image pin (`eclipse/zenoh@sha256:...`), the role's rocksdb-backend artifact SHA256 + version, design-doc references that quote a specific version. |
| Related | [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) (upstream pivot, supersedes 0708), [ADR 0708](../adr/0708-build-zenoh-from-source.md) (build-from-source, retired), F-092, F-122, F-139, [`zenoh-trial-fabric.md`](zenoh-trial-fabric.md), `zenoh-data-fabric.md`, `panoscope.md` |

## Why this doc

zenoh's wire protocol is supposed to be stable across the 1.x line per
upstream, but in practice every place we touch zenoh has to agree on
the exact version. A 1.9.0 client talking to a 1.0.4 server **opens a
session** (looks like it works) and **then closes the subscription
channel** the instant the first sample tries to flow — silent failure
mode that costs an afternoon to diagnose. Same class of pain when:

- The router and a storage backend `.so` are built against different
  zenoh-plugin-trait versions → "Cannot spawn volume": Plugin
  compatibility mismatch at zenohd startup (the F-122 / F-136 class).
- Tag names drift from underlying source ref.

This doc names **the single project-wide version-of-record** and the
places that must update together.

## The version

**`=1.9.0`** — exact-pin, not range.

Image: upstream `eclipse/zenoh:1.9.0` pinned by content digest. Same
1.9.0 source tag applies to the upstream `eclipse-zenoh/zenoh-backend-rocksdb`
release whose prebuilt `.so` we bind-mount into the router (since
upstream's image does not ship the backend — F-139). The
image + backend are products of the same upstream pipeline at the same
release tag, so the plugin-trait ABI is aligned by construction.

## Where to pin

Every file below MUST agree on `1.9.0`. Bumping zenoh is a coordinated
change across all of them.

| File | What to pin |
|---|---|
| [`ops/panoscope/Cargo.toml`](https://github.com/usareur-af/goatstuff/blob/main/ops/panoscope/Cargo.toml) | `zenoh = "=1.9.0"` |
| [`ops/ansible/roles/zenoh/defaults/main.yml`](../../ops/ansible/roles/zenoh/defaults/main.yml) | `zenoh_image: "eclipse/zenoh:1.9.0@sha256:<digest>"`, `zenoh_rocksdb_backend_version: "1.9.0"`, `zenoh_rocksdb_backend_sha256_map.{aarch64,x86_64}` |
| [`ops/ansible/roles/goat-zenoh/defaults/main.yml`](../../ops/ansible/roles/goat-zenoh/defaults/main.yml) | same shape with `goat_zenoh_*` prefix |
| [`ops/zenoh/pqc-image/Dockerfile`](../../ops/zenoh/pqc-image/Dockerfile) | `ZENOH_TAG=1.9.0` + `UPSTREAM_IMAGE=eclipse/zenoh:1.9.0@sha256:<same digest as the role pin>`; on bump also re-apply `patches/zenohd-pqc-aws-lc-rs.patch` (git-apply fails loudly if upstream moves `main()`) and re-verify the three F-247 provider-install sites |
| [`ops/zenoh/pqc-image/Dockerfile.fips`](../../ops/zenoh/pqc-image/Dockerfile.fips) | `ZENOH_TAG=1.9.0` + `ROCKSDB_BACKEND_TAG=1.9.0` (matched-pair glibc plugin build); on bump re-apply `patches/zenohd-fips-aws-lc-fips.patch` and re-check the backend-traits `default-features` sed still matches the rocksdb manifest. **`AWS_LC_FIPS_SYS_VERSION=0.13.11` does NOT move on a zenoh bump** — it pins the CMVP #5298/#5314-validated AWS-LC FIPS 3.1.0 source and only changes when a newer module version is certificated (zenoh-pqc-tls.md §5) |
| `ops/ansible/group_vars/nova_hosts/vars.yml` | `zenoh_router_image: "ghcr.io/usareur-af/zenoh-pqc@sha256:<list digest>"` — the nova-a standing PQ canary (ADR 1079); a zenoh bump rebuilds zenoh-pqc on the new tag and this pin moves to the NEW list digest (+ deploy-history top row), not back to upstream |
| Any future zenoh-using crate | `zenoh = "=1.9.0"` |

## What NOT to use

| Pattern | Why not |
|---|---|
| `zenoh = "1"` or `"1.9"` | Cargo floats to the latest 1.x at lockfile resolution; runtime + clients can end up on different patch versions. |
| `eclipse/zenoh:latest` | Tracks upstream's `latest`, can leapfrog our pinned source. |
| `eclipse/zenoh:1.9.0` (tag-only, no digest) | Tags are mutable upstream. Pin by `@sha256:...` to lock the bytes. |
| `ghcr.io/usareur-af/zenoh:*` | Retired by ADR 0858 step 5. References fail the CI lint. |

## CI enforcement

The version of record and every place it must appear are declared in
[`ops/upstream/deps/zenoh.yaml`](../../ops/upstream/deps/zenoh.yaml)
([ADR 1262](../adr/1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md),
[`upstream-upgrade-process.md`](upstream-upgrade-process.md)).
[`.github/workflows/upstream-pins.yml`](../../.github/workflows/upstream-pins.yml)
runs `python3 ops/upstream/upstream.py check` on every pull request and fails when:

- a Cargo.toml names a zenoh crate with anything but an exact `=<version>` pin, or a
  Cargo.lock resolves one to another version;
- a pip, npm, Dockerfile, role-default, Go or HTML place names another version, digest,
  rocksdb checksum or builder toolchain than the manifest;
- a deploy config references the retired `ghcr.io/<org>/zenoh` image or `eclipse/zenoh`
  by tag without a digest;
- a patch file sits in `ops/zenoh/pqc-image/patches/` without a manifest entry.

`ops/ci/check-zenoh-pinning.sh` remains as an entry point and calls the same check.

## Bumping zenoh

```bash
python3 ops/upstream/upstream.py bump zenoh <new> \
  --set upstream_image_digest=sha256:<from docker buildx imagetools inspect eclipse/zenoh:<new>> \
  --set rust_toolchain=<upstream rust-toolchain.toml at the tag> \
  --set rocksdb_sha256_x86_64=<sha256 of the release zip> \
  --set rocksdb_sha256_aarch64=<sha256 of the release zip> \
  --lockfiles
python3 ops/upstream/upstream.py check zenoh
python3 ops/upstream/upstream.py patches zenoh --src <upstream zenoh checkout at <new>>
```

`bump` names each carried patch whose fix shipped in the new release; delete it, its lines in
both Dockerfiles and its manifest entry. A zenoh bump while an embargo touches the vendored
link crate, a lock file or a `[patch]` section goes to the embargo owner before its first
push. The candidate suite and the release ladder follow
([`upstream-upgrade-process.md`](upstream-upgrade-process.md) §1).

## Cross-references

- [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) — upstream-image
  pivot (current posture)
- [ADR 0708](../adr/0708-build-zenoh-from-source.md) — superseded
  build-from-source posture
- F-139 —
  rocksdb backend not in upstream image; engineering response shape
- F-122 —
  plugin-trait ABI tax that build-from-source was originally fighting
- [`zenoh-trial-fabric.md`](zenoh-trial-fabric.md) — nova deploy
- `panoscope.md` — first consumer that surfaced the
  version-skew failure mode
