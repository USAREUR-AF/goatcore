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

[`ops/ci/check-zenoh-pinning.sh`](../../ops/ci/check-zenoh-pinning.sh)
greps the tree and fails CI when:

- A Cargo.toml uses a range instead of `=1.9.0`.
- A deploy-target config references the retired `ghcr.io/usareur-af/zenoh`
  image.
- A deploy-target config references upstream `eclipse/zenoh:` by tag
  only (no `@sha256:...` digest pin).

Runs on every PR via the existing ansible-syntax-check workflow.

## Bumping zenoh

Coordinated change-set when upstream cuts a new release and we decide
to upgrade:

1. Bump the image digest pin in both role defaults (resolve the new
   multi-arch manifest digest from `docker buildx imagetools inspect
   eclipse/zenoh:<new>`).
2. Bump `*_rocksdb_backend_version` in both role defaults.
3. Resolve the new per-arch SHA256 for the rocksdb backend artifact
   from the GitHub release API (`api.github.com/repos/eclipse-zenoh/
   zenoh-backend-rocksdb/releases/tags/<new>`, asset `.digest`).
   Update `*_rocksdb_backend_sha256_map` in both roles.
4. Bump every `zenoh = "=<old>"` to the new version in any crate.
5. Run `bash ops/ci/check-zenoh-pinning.sh` locally; CI re-runs it.
6. Smoke: panoscope subscribes against the new router; storage
   spawn-on-restart works (per F-139 smoke).

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
