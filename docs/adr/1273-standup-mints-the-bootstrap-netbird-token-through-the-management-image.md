# ADR 1273 — Standup mints the bootstrap NetBird token through the management image's own command

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-27 |
| Deciders | operator |
| Repo scope | goat core (`ops/netbird/patches/service-user-token-bootstrap.patch`, `.github/workflows/netbird-mgmt.yml`, `ops/pins/fleet-pins.yaml`, `crates/goatbuilder/src/standup.rs`, `ops/scripts/goat-standup.sh`, `ops/scripts/netbird-service-identities.py`, `ops/ansible/dashboard-deploy.yml`), private overlays (a deployment that pins its own management image bumps that pin) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0301](0301-patched-netbird-management-image.md) (patched management image) · [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock upstream NetBird; every departure recorded with its retirement condition) · [ADR 0413](0413-no-orm-in-our-go-code.md) (raw SQL in our Go code) · [ADR 1204](1204-declared-secrets-posture-and-zero-saas-quickstart.md) (declared secrets posture) · [ADR 1257](1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md) (build targets) · design [`goatbuilder-build-targets.md`](../design/goatbuilder-build-targets.md) §4.1 · gate row J7 in [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) |

## TL;DR (for a layperson)

Setting up a new goatnet has one step that still needs a person: signing in to the mesh dashboard and creating the first administrator key, which every later step uses. We automate that step. The key is made by the mesh software's own code, in a version of it that we already build and run with a small, recorded change. The key is written to the goatnet's secrets store and read back before it is switched on, so there is never a working administrator key that nobody holds. It never appears on a screen or in a log, running setup again never makes a second key, and the mesh records that the key was made. If we had chosen to write the key into the mesh's database ourselves, every mesh upgrade could silently break it; if we had kept the step manual, every new goatnet would keep waiting for a person.

## Context

Standup phase 11 is manual today (`standup.rs`, the two `MANUAL` rows for LAN and internet sites). A person signs in to the NetBird dashboard with the bootstrap password, re-runs `dashboard-deploy.yml` so its post-task promotes them to admin, creates the service user `svc-<site>-netbird-infra` with role admin, creates its personal access token, and stores the token as `<SITE>_NB_API_TOKEN` under the declared secrets posture. Phase 12 then mints every other NetBird service identity with that token. Gate row J7 asks for phase 11 with no manual step.

The design ([`goatbuilder-build-targets.md`](../design/goatbuilder-build-targets.md) §4.1) chose a new `ops/mgmt-host-bootstrap` verb that writes the service user and token into NetBird's database with raw SQL. It rejected a route through the management service because that "would need a modified NetBird management service", which it took to conflict with ADR 1248. A trust review of that design found two things. First, the raw-SQL route writes NetBird's tables behind its back: in the version the fleet runs (upstream v0.71.4), `users.name` and `users.email` are encrypted at rest with a key the verb does not hold, and any NetBird upgrade can add a column, change the token hash or move a field. Second, the premise was wrong: the fleet's management image (`ops/netbird/Dockerfile`, ADR 0301) already applies `service-user-token-bootstrap.patch`, a recorded ADR 1248 departure that adds `management service-user bootstrap-token`. That command mints a service user and token through NetBird's own store code. It prints the plaintext token to stdout, mints another admin token on every run, and writes no activity event.

The operator chose, on 2026-11-27, to fix that command and use it for phase 11, with the conditions below.

## Decision

Standup phase 11 mints the bootstrap NetBird identity (`netbird-infra` in `ops/netbird/service-identities.yaml`) with the fleet management image's own patched command, not with a raw-SQL writer and not by hand. These five conditions are part of the decision; a build that does not meet each one is not this decision:

1. **Founder only.** Only a founder site's standup mints it (`goatnet_founder` true). A joining site never runs the mint: NetBird's `users` and `personal_access_tokens` tables replicate between sites (Spock), so a joiner's mint would add a second admin identity to the shared account.
2. **No token on stdout or in any log.** The command writes the plaintext only to a file it creates with `O_CREAT|O_EXCL`, mode 0600, in a root-owned 0700 directory under `/run`. No output stream, argument, environment variable, error message, trace span, Ansible variable, idempotency record or goatbuilder run record ever holds it. The operator side reads the file over SSH and pipes it into the store; it never becomes an Ansible variable.
3. **A re-run mints no second token.** A service user that already holds its token is left as it is, and the run exits as already applied. A service user that exists with no token is refused; making a new token for it is a separate, deliberate command, never a side effect of a re-run.
4. **An audit event.** The command writes NetBird's own activity events for what it does (`ServiceUserCreated`, `PersonalAccessTokenCreated`), as the dashboard path does, so the mesh's activity log shows the identity was made, by the standup, and when.
5. **A stored copy before the token is valid.** Making the token and switching it on are two steps. The first creates the service user and writes the token file, and saves no token row. The operator side stores the value under the declared posture (every posture, 1Password included), reads it back, and sends only the base64 SHA-256 hash of the read-back value to the host. The second step inserts the token row with that hash. A token that authenticates is therefore, by construction, the one the store returns. A store that keeps an old value, or a read-back that differs, is a failure, and no token row is written. The host file is removed after the read-back, before the token is checked against the API.

Two requirements apply to the build: its tests run against the fleet management image (the digest `ops/pins/fleet-pins.yaml` pins), not only against stock NetBird; and each change that implements it gets a hostile review before it is pushed.

**What it amends.** Two premises of earlier records change:

- [`goatbuilder-build-targets.md`](../design/goatbuilder-build-targets.md) §4.1 and §4.2: the route through `ops/mgmt-host-bootstrap` with raw SQL and "no NetBird fork". The route is now the management image's patched command. The patch was already a recorded departure (ADR 1248; `ops/upstream/deps/netbird.yaml`); its retirement condition is unchanged: it goes when upstream NetBird offers an offline, non-interactive way to mint the first admin token.
- The rule that a NetBird service identity is minted only by an operator running `netbird-service-identities.py`, never as a side effect of a playbook ([`live-state-guard-scheduling.md`](../design/live-state-guard-scheduling.md), `ops/netbird/service-identities.yaml`). For the one bootstrap identity, the standup mints it, and condition 4's activity events are its audit record. Every other identity keeps the rule.

With the manual row gone, nobody signs in to the dashboard during standup. The first person to sign in afterwards is promoted to admin by the existing promotion in `dashboard-deploy.yml`, run again after that sign-in, so who becomes the first human admin does not change.

## Alternatives considered

- **A raw-SQL verb in `ops/mgmt-host-bootstrap`** (the design's route). No NetBird code change. It writes NetBird's tables without NetBird's code: it cannot encrypt `users.name` without the store's key, so it must write it empty and get every other column's shape right by hand, and every NetBird upgrade can break it silently. Rejected for that schema risk.
- **Keep phase 11 manual.** No new trust change. J7 stays unbuilt, every new goatnet waits for a person at phase 11, and the patch stays in the image unused. Rejected by the operator.
- **The patch as it is.** Already ships. It prints the token, mints a new admin token on every run and records nothing. Rejected: it fails conditions 2, 3, 4 and 5.
- **One step that mints and saves the token, then stores it.** Simpler. A failure between the save and the store leaves a working admin token that no store holds. Rejected for condition 5.

## Consequences

**Positive.** A founder standup runs from phase 1 to 15 with no person present. The token is made by NetBird's own code at whatever version runs, so its encryption, hash and format follow NetBird. A working admin token never exists without a stored copy. A re-run is safe. The activity log shows the identity's creation.

**Negative.** goatbuilder now mints the goatnet's first NetBird admin credential with no person present. Root on the management host could already do this by hand (it holds the database superuser), so this is a change in automation and in the path the token travels, not a new privilege; it is still a trust change, and it is reviewed as one. The departure in the management image now carries more weight, and moving to a later upstream NetBird must carry the fixed command with it. The plaintext is on the host's `/run` (tmpfs, which can swap) for the seconds between the first step and the read-back, readable there by root and the `docker` group. With no sign-in during standup, the first human admin is whoever signs in first afterwards, and the bootstrap password file stays on the host until then.

## Assessor view (SCA-R)

This automates the issue of a privileged, non-person credential (IA-4, IA-5, AC-2): the identity is declared in a registry, minted by the vendor's own code path, stored under the declared secrets posture before it is usable, and its creation is recorded in the mesh's activity log (AU-2, AU-3, AU-12). The plaintext is confined to one root-only tmpfs file for seconds and never logged (IA-5(7), SC-28); the tests search every captured output, in encoded forms too. Least privilege is not improved: the identity is an admin, as it was when a person made it, because NetBird has no narrower role that can mint the other identities. The residual risk is the short host-file window and the automation itself; compensating controls are the founder-only gate, the refusal to mint a second token, and the activity record.

## Trigger conditions for revisit

- Upstream NetBird offers an offline, non-interactive way to mint the first admin token: retire the patch (ADR 1248) and move phase 11 to it.
- NetBird gains a role narrower than admin that can mint service users and tokens.
- A later NetBird changes how service users or tokens are stored, so the patched command no longer applies or its tests fail against the fleet image.
- A secrets posture is added that cannot read a value back.
