# Upstream upgrade process: one path for every load-bearing dependency

| Field | Value |
|---|---|
| Status | **Built:** the tool (`check`, `bump`, `patches`, `watch`), `check` in CI, manifests for Zenoh, NetBird and Ory, the Zenoh mixed-version candidate row. **Written, not yet run:** the private weekly watcher and the Zenoh candidate workflow. **Designed, not built:** NetBird and Ory candidate suites, manifests for the other dependencies in the inventory |
| Decision | [ADR 1262](../adr/1262-one-upgrade-process-for-load-bearing-upstream-dependencies.md) |
| Tool | [`ops/upstream/upstream.py`](../../ops/upstream/upstream.py) |
| Manifests | [`ops/upstream/deps/`](../../ops/upstream/deps/) |
| CI | [`.github/workflows/upstream-pins.yml`](../../.github/workflows/upstream-pins.yml) |
| Ladder | [`release-graduation.md`](release-graduation.md) (ADR 1120) |

## 1. What this is

**Plain.** A new upstream version of Zenoh, NetBird, Ory or another dependency goat cannot run
without becomes, by a fixed sequence of steps, a tested candidate that a platform admin can
roll out. Every dependency goes through the same steps; only its manifest and its candidate
tests differ.

**Detail.** The steps, where each runs, and who approves:

| Step | Trigger | Runs | Where | Gate | Approves |
|---|---|---|---|---|---|
| 1. Manifest | — | `ops/upstream/deps/<name>.yaml` holds version, facts, rules, patches, moves-with | goatcore, goatstuff | `upstream.py check` in CI on every pull request | goat-owners review |
| 2. Watch | Weekly schedule or by hand | `upstream.py watch --json`: newer releases of the dependency and its companions, keyword hits in release notes, carried patches whose upstream pull request has shipped | Private overlay (scheduled workflow, one private issue per dependency) | None; a notice | — |
| 3. Assess | The issue | An engineer reads the notes against our findings and patches and writes go or no-go in the issue | Private issue | Written decision | One platform admin |
| 4. Bump | Go | `upstream.py bump <name> <version> --set fact=value --lockfiles`; deletes patches whose fix shipped; `patches --src` proves the rest still apply | A branch in goatcore (and goatstuff) | `check` passes; the image builds apply every remaining patch | goat-owners |
| 5. Embargo stop | Before first push | The private public-push scan refuses a push that changes a file an open embargo's patch changes | Operator laptop and session hooks | Fails closed | Embargo owner, in writing |
| 6. Build | Candidate branch pushed | The dependency's existing image workflows, dispatched on the branch; when the tree pins images built from itself (Zenoh's router images), a second `bump --set` points those pins at the candidate builds | goatcore CI | Plugin or service boot test, FIPS probe where applicable | — |
| 7. Candidate suite | Build green | `upstream-candidate-<name>.yml` (§4), dispatched with the candidate image; reusable rows in `ops/upstream/candidate/<name>/` | Private overlay CI on throwaway runners (finding rigs and advance builds are private) | Every row passes; results attached to the candidate record | — |
| 8. Ladder | Suite green | `graduate.py` moves the pin up the dev, preprod and prod rungs with a soak per rung; clients and partner text follow the server via `coupled_with` | Private overlay | ADR 1120 gates, rollout preflight (ADR 1211) | A platform admin per rung, by explicit instruction. Never automated (ADR 1177) |

## 2. Dependencies in the process

**Plain.** A dependency is load-bearing when a goatnet stops working without it and moving it
needs more than a digest bump: a schema migration, clients that must move too, or patches we
carry. Those get a manifest. Everything else stays with Dependabot.

**Detail.** Version, patches and companions per dependency, read from the tree on 2026-11-26:

| Dependency | Ours | Upstream latest | Carried on top | Moves with it | Manifest |
|---|---|---|---|---|---|
| Zenoh (router, rocksdb plugin, Rust, Python, TypeScript clients) | 1.9.0 | 1.10.2 | 7 patch files in `ops/zenoh/pqc-image/patches/` (F-253, F-268, F-524, F-614, PQC, FIPS, one backport); vendored `zenoh-link-tls` copies | Every fabric service, partner clients, partner-facing portal text | [`zenoh.yaml`](../../ops/upstream/deps/zenoh.yaml) |
| NetBird (management built here as netbird-mgmt; dashboard; stock fallbacks) | 0.71.4 | 0.83.2 | 7 patch files in `ops/netbird/patches/`; goat-client embeds a fork branch | Dashboard, host agents, goat-client, goat-relay | [`netbird.yaml`](../../ops/upstream/deps/netbird.yaml) |
| Ory (Kratos, self-service UI, Hydra) | 26.2.0 (Kratos, UI and Hydra on one calendar version) | 26.2.0 | 5 patch files in `ops/ory-consent/patches/` | Consent image, goat-id-console, every sign-in | [`ory.yaml`](../../ops/upstream/deps/ory.yaml) |

Further rows are added as each dependency's manifest lands; the full inventory with every
pinned location is kept beside the fleet pins in the private overlay, because it names where
each goatnet runs what.

### Upgrade plans for the three named dependencies

**Zenoh 1.9.0 → 1.10.2.** 1.10.2 fixes the QUIC admission stall (upstream #2719, fixed by
#2805) and ships the querier deadlock fix we backported (#2635), so that patch drops. The F-253,
F-268, F-524, PQC and FIPS patches apply unchanged to the 1.10.2 source; the zenoh-link-tls
stream change (F-614) is re-vendored at 1.10.2 and its router patch rebased after F-253.
Upstream main carries the fixes for F-253 (#2919) and the link-tls defect (#2910); the first
release containing both drops two more patches and every vendored copy. goatstuff moves after
goatcore: three of its workspaces take `panoscope-core` from goatcore by git ref.

**NetBird 0.71.4 → 0.83.2.** Four patches rebase (`skip-migrations`, `auto-peer-name`,
`service-user-token-bootstrap`, `setupkey-user-id-stamp`); `peer-id-bound-relay-tokens` drops in
favour of upstream #8493 when that ships; the two ADR 1081 client patches leave the management
build, which compiles neither into a shipped binary. Before the bump: from 0.83.0 the
`peers.peer_status_*` columns are no longer written (live state moves to `peer_connections`),
so every direct SQL reader of them must move first; the Spock repset list needs a decision for
`peer_connections`, `management_instances` and the agent-network tables; rollback is a
pgBackRest restore. 0.84.0 waits until goat-relay accepts the relay v2 auth envelope.

**Ory 26.2.0 (bumped from Kratos 1.3.1 / Hydra 2.3.0).** 25.4.0 was skipped (the only release
hit by the pagination-token SQL injection advisories, fixed in 26.2.0). All five consent patches
apply to the 26.2.0 compiled UI and pass `verify-patched.sh`. Kratos 26.2.0 adds columns and a
backfill on replicated tables, so it moves a whole goatnet in one window,
[`ory-kratos-upgrade-window.yml`](../../ops/ansible/ory-kratos-upgrade-window.yml); the `ory` role
refuses that move per site. Hydra's migrations are site-local and move site by site through the
ordinary path. Procedure, refusal and restore: [ory-identity-upgrades.md](ory-identity-upgrades.md).
Retest: `/admin/sessions` loses `x-total-count`, the verified-address login behaviour, the
login-form double-submit guard.

## 3. The manifest

**Plain.** One YAML file per dependency is the only place its version is written by hand.
Every other place either matches it or fails CI.

**Detail.** Keys (see [`zenoh.yaml`](../../ops/upstream/deps/zenoh.yaml) for a full example):

| Key | Holds |
|---|---|
| `version` | The version of record, quoted |
| `upstream` | `github` repo (or `pypi`, `npm`, `docker`), `companions` released in lockstep, `watch_keywords` matched against release notes |
| `facts` | Values that change with a release: image digest, plugin checksums, toolchain. `bump --set NAME=VALUE` rewrites each; `series` (major.minor) follows the version by itself |
| `rules` | Where the version appears. Kinds: `cargo` (every Cargo.toml is an exact `=X` pin, every Cargo.lock agrees), `pip`, `npm`, `gomod`, `text` (named files, a regex with named groups `v` and fact names), `regex` (the same across globs), `forbid` (a shape that must never appear: a retired image, a tag without a digest) |
| `lockfiles` | Commands that regenerate lock files; `bump` prints them or runs them with `--lockfiles`. Lock files are never text-edited |
| `patches` | Each carried patch: `file`, `finding`, `purpose`, `upstream_pr` or `upstream_issue`, `fixed_in` (first release containing the fix, or null). `bump` names every patch whose `fixed_in` is reached. `patch_dirs` makes an unlisted patch file fail `check` |
| `moves_with` | What follows the server up the ladder, in order |
| `candidate_suite` | The workflow that tests a candidate |

A line carrying `upstream-pin: allow` is skipped by every rule; use it for a historical
mention, never for a pin. `<name>.private.yaml`, present only in an assembled tree, is merged
into the manifest and carries rules and patches that must not be described publicly. Its
top-level `exclude` (path globs) removes paths from every rule: private places that keep a
version on purpose, such as deploy records, frozen snapshots and values the release ladder moves.

## 4. Candidate suites

**Plain.** Before a new version goes anywhere near a goatnet, CI starts it on throwaway hosts
and runs the checks that our past failures with that dependency taught us.

**Detail.** Common rows, then the rows each dependency adds. Status: **designed, not built.**

| Row | Applies to | Pass bar |
|---|---|---|
| Service or plugin boots from the candidate image | All | Healthy within its start timeout; plugins load (exact compiler match, F-122) |
| Old clients against the new server, new clients against the old server | Zenoh, NetBird | Publish, subscribe, query and liveliness round-trip both ways; NetBird peers connect through management, signal and relay at mixed versions |
| Access-control proofs | Zenoh, Ory | `ops/ansible/roles/goat-zenoh/tests/isolation-proof.sh` and `rr3p-live-proof.sh` pass; Ory permission checks return the recorded answers |
| Schema migration forward on a copy | Ory, NetBird management, PostgreSQL | Migrations complete on a restored copy; services start on the result; replication keeps applying |
| Finding rigs | Zenoh | F-253 accept-loop repro, F-268 and F-524 wedge rigs, M-119 slow-subscriber rig: each meets the bar recorded in its finding |
| Advanced Pub/Sub catch-up | Zenoh | A late subscriber receives history within the configured window |

## 5. Public and private

**Plain.** Everything generic is public; anything that would show which fixes we carry for an
undisclosed problem, or where our goatnets run what, is private.

**Detail.**

| Piece | Lives in | Why |
|---|---|---|
| `upstream.py`, public manifests, CI check, reusable candidate rows | goatcore (goatstuff carries its own manifests) | Generic; reveals nothing beyond the public tree |
| Candidate workflows, finding rigs | Private overlay | The rigs reproduce findings, and candidate builds during an embargo are private |
| Weekly watcher schedule and its issues | Private overlay | The issue lists every patch and its upstream state |
| `<name>.private.yaml` | Private overlay, merged by `assemble.sh` | Embargoed patches and their paths; `exclude` for private places that keep a version on purpose |
| Embargo stop | Private public-push scan | Already refuses a public push touching an open embargo's files |
| Ladder records, `coupled_with`, inventory with fleet pins | Private overlay | Name goatnets and their pins |

## 6. Adding a dependency

1. Write `ops/upstream/deps/<name>.yaml`: version, upstream, facts, rules, patches.
2. Run `python3 ops/upstream/upstream.py check <name>` until it passes, tightening rules
   until every load-bearing place is matched (the count is printed).
3. Add reusable rows under `ops/upstream/candidate/<name>/` and an `upstream-candidate-<name>.yml` workflow in the private overlay with the rows from §4 that apply.
4. Add the dependency to the private watcher's list and its `coupled_with` links in the
   fleet pins.
