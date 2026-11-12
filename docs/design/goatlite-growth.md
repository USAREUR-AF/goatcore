# goatlite growth — a first goatnet grows into a full environment without a tear-down

| Field | Value |
|---|---|
| Status | **Designed.** Nothing in this document is built except where a row says so. Nothing in it has run on hardware. |
| Grows | [goatlite.md](goatlite.md) (the lite preset, ADR 1213) |
| Into | the full preset of the goatbuilder config ([schema](../../ops/goatbuilder/schema/goatbuilder-v1.schema.json)) and the procedure in [goatnet-standup.md](../operations/goatnet-standup.md) |
| Gate rows | G1 and G2 of the goatbuilder 1.0 gate |

## 1. What this is, and the rule that wins every conflict

A person who stood up a goatlite and wants a full goatnet keeps the one they have. They run one command, `goatbuilder grow`, and the same box, the same name, the same certificate authority and the same database carry on into a full environment. Nothing is torn down and nothing is stood up a second time.

The rule that decides every trade-off below: **a goatlite's first run does not get heavier to make growth easier.** The first run keeps its one file of four facts (`lite.name`, `lite.host`, `lite.ssh_user`, `lite.admin`; the schema's `lite.required`), asks no new question, prints no new prompt, needs no outside account and gains no phase. Everything growth needs — a second key holder, a secrets store, a public zone, more hosts — is asked for and provisioned when the learner chooses to grow, never in advance. Gate row G2 holds this with a test (§8).

## 2. The command

`goatbuilder grow` takes a running goatlite through numbered steps. `--dry-run` prints every step, the configuration it will write and the commands it will run, and changes nothing. Each step is idempotent and can be resumed with `--from`/`--to`, as `standup` and `join-node` are. Each step asks only for the facts that step needs. A step the learner does not want (an isolated goatnet needs no reach) is skipped as declared.

| Step | What changes | What stays | Status |
|---|---|---|---|
| 1 | Custody of the identity root: the CA key is re-encrypted to two or more holders (§3) | the root certificate, every issued certificate, every host | designed, not built |
| 2 | Reach: the `lite.internet` block, rendered and stood up from phase 5 ([goatlite.md §6](goatlite.md#6-growing-out-of-lite)) | the CA, the SPIRE trust domain, the ACL | render built; not stood up against real relays |
| 3 | Hand-over: `goatlite.yaml` is rewritten into a full configuration with the same values, and the rendered file set becomes the learner's (§6) | every rendered file's content | designed, not built (gate row G1) |
| 4 | Secrets store: values move from the env-file to the declared store (§6) | every value | designed, not built |
| 5 | Tier ceiling and promotion rungs: `site_deploy_tier_max` raised; rungs added | the running tier-1 services | designed, not built |
| 6 | Observability host: `join-node --class obs` | the core host | the verb is built; unmeasured on hardware |
| 7 | Second core: an intra-site standby, or a second site that joins this one (§5) | the first site's data | the verbs are built; unmeasured on hardware, and never run from a founder that began as a goatlite |

The order has two reasons. Custody comes first because reach exposes the goatnet to networks beyond the LAN, and the CA key should have a second holder before that. Reach comes before the hand-over because the lite renderer is the only tool that writes the internet surface, and it refuses any file that is not `preset: lite` (`ops/scripts/goatlite-render.py:124`).

## 3. The identity root

A goatlite's certificate authority is minted on the first run with one holder: the learner's own age key. Growth can raise that to two or more holders in place, roll over to a new root, or rebuild. The default is the in-place custody upgrade. Whether a root that began with one holder is acceptable for production is the owning team's declared posture, not something the tool decides; the tool records the fact so the team can decide.

### 3.1 What first run creates

Phase 4, or the render step before it, runs `ca-init --mode dev --allow-single-recipient` with the learner's age key as the single recipient (`ops/scripts/mint-site-certs.sh:207`). The private key is an ECDSA P-256 key in SEC 1 DER form, age-encrypted to the recipient list (`ops/enrollment/ca/age_ca.go:23`, `ops/enrollment/ca/age_ca.go:105`), stored at `~/.goat/state/<name>-ca/ca.key.age`. The root certificate is committed at `ops/enrollment/ca/<name>/root-cert.pem` and the public key at `ops/enrollment/public-keys/<ca_id>.pem`. A lite run mints no intermediate. SPIRE runs its own self-signed authority and does not chain to this root, so nothing below touches SPIRE.

Outside goatlite, `ca-init` refuses fewer than two recipients (`ops/enrollment/cmd/ca-init/main.go:160`, `ops/enrollment/cmd/ca-init/main.go:407`), and `ca-init --verify` reports a key with fewer than two as a problem (`ops/enrollment/cmd/ca-init/verify.go:55`).

### 3.2 The three options

| | (a) Custody upgrade in place | (b) Rollover | (c) Rebuild with the values carried over |
|---|---|---|---|
| What happens | decrypt the key once with the learner's identity, re-encrypt it to two or more recipients, replace the file, destroy the single-recipient copy | mint a new root (dev mode with two or more holders, or a YubiKey), trust both, re-mint every leaf under the new root, retire the old one | a new goatnet from the same declared configuration: new CA, new identities, every peer re-enrols |
| Possible today | the key format allows it: age encrypts one payload to any number of recipients. No tool does it: `ca-init` creates the key file with `O_EXCL` and has no re-wrap mode (`ops/enrollment/ca/age_ca.go:86`) | yes, by hand: the bundle validator trusts every key in `public-keys/` (`docs/design/offline-enrollment.md:355`), and the router can trust an old and a new root at once (`ops/ansible/roles/goat-zenoh/defaults/main.yml:23`) | yes: `goatlite-render.py --remove` then a new standup |
| Changes on hosts | none | every leaf certificate and every service that loads one | everything |
| Certificates re-issued | none | every leaf in the manifest (about twenty), every per-peer certificate the portal issued, every enrolment bundle, since a bundle carries one trust anchor (`ops/enrollment/bundle/bundle.go:152`); the signed pins intent is re-signed | all |
| Time | minutes | one bundle lifetime of overlap, 90 days, before the old root is retired (`docs/operations/yubikey-piv-ca-ceremony.md:164`) | a standup plus every peer's re-enrolment |
| Risk while it runs | none beyond the step itself | during the overlap the router accepts any name either root signs, because the Zenoh ACL matches the certificate name and not its issuer (`docs/adr/1164-per-site-spire-trust-domains.md:73`) | the goatnet is down until rebuilt |
| What it cannot do | move the key into hardware: a YubiKey generates its key on the card and the key never leaves it (`ops/enrollment/cmd/ca-init/prod.go:4`; cloning to a card is rejected in `docs/design/offline-enrollment.md:351`); undo the period with one holder: an old copy of the file still opens with the learner's key | change the goatnet's name (§4) | carry the data: the configuration is carried over, the database is not |

### 3.3 The default and what the others are for

**Default: (a), as growth step 1.** It closes the one risk of first-run custody that grows with time — a CA key with one holder is lost with that holder (`ops/enrollment/cmd/ca-init/main.go:71`) — and it touches no host and re-issues nothing. The step asks for the public key of each additional holder, re-encrypts, runs `ca-init --verify`, and writes into the goatnet's run record that the root was held by one key from its mint date to the growth date. `--verify` counts recipients and cannot prove that the holders are distinct people (`ops/enrollment/cmd/ca-init/verify.go:42`), so the step names who the holders are in the record, from what the learner declares.

**(b) Rollover** is for a team whose declared posture needs one of: a hardware-held root (YubiKey PIV, [yubikey-piv-ca-ceremony.md §9](../operations/yubikey-piv-ca-ceremony.md#9-rotation)); a root that was never held by one key; a root whose subject and id do not carry first-run provenance (`<name> goatlite CA` and `dev-<name>-ca-ecdsa-<date>`, §4). The ceremony runbook's rotation is this procedure; growth would run it with the dev root as the old root.

**(c) Rebuild** is for a goatnet that must change its name, or whose single-held key may have been exposed and whose team prefers a clean lineage over a rollover with revocation.

## 4. The name

The goatnet's name (`lite.name`) is written into identities on the first run and cannot be changed later without a rebuild. It matters in production once another party holds it. At growth time the learner is told one line, and at first run nothing.

| Stamped at first run | Value for `name: pasture` | Source | Changes when |
|---|---|---|---|
| SPIRE trust domain | `pasture.pasture.lan` | `ops/scripts/goatlite-render.py:987`; locked once minted (`ops/scripts/goatlite-render.py:256`) | rebuild only |
| URI SAN prefix in every certificate | `urn:goat:pasture:` | `ops/scripts/goatlite-render.py:749` | rebuild only |
| CA id | `dev-pasture-ca-ecdsa-<date>` | `ops/scripts/goatlite-render.py:246` | rollover |
| Root subject | `pasture goatlite CA` | `ops/scripts/mint-site-certs.sh:209` | rollover |
| Service zone | `pasture.lan` | `ops/scripts/goatlite-render.py:143` | reach (step 2) moves the services to the public zone; the trust domain keeps `.lan` |

**Whether it matters in production.** Inside one goatnet the name is a label; nothing in this repository resolves the trust domain as a DNS name. It starts to matter when another party records it: a trust domain and a URI SAN prefix appear in every federation agreement, and once a partner holds them, renaming needs that partner's work too (`docs/adr/1164-per-site-spire-trust-domains.md:53`). The `dev-` prefix and the word `goatlite` in the root subject show provenance in every chain dump and audit log (`ops/enrollment/README.md:68`); a team that does not want that runs the rollover. The join-node gate that refuses a site trusting the shared unnamed dev root accepts a goatlite CA id, because the id names its own goatnet (`crates/goatbuilder/src/joinnode.rs:205`).

**The one line, printed by `grow` before step 1:**

> pasture keeps its name: it is written into every identity this goatnet has issued (pasture.pasture.lan), so changing it later means a rebuild.

## 5. The database: one site to two

A second site can join a running single site. The procedure exists and treats the running site as the founder: the founder keeps its data, gains a replication node and two subscriptions, and is never re-seeded. It has not been run from a founder that began as a goatlite, and nothing of it has run on hardware through `goatbuilder`.

**What a goatlite already has.** Every mgmt host, a goatlite included, runs the `postgres-spock` image (`ops/pins/fleet-pins.yaml:16`), which ships the Spock extension, a configuration tuned for logical replication, and init scripts that load Spock in each replicated database (`ops/postgres/spock/Dockerfile:27`, `ops/postgres/spock/Dockerfile:30`). The goatlite renders itself as the founder (`goatnet_founder: true`, `ops/scripts/goatlite-render.py:553`) with replication off (`participates_in_spock: false`, `ops/scripts/goatlite-render.py:564`), and phase 3 already generates the cluster-shared replication password (`ops/scripts/goatlite-render.py:645`).

**The join.** `goatbuilder join-node` for a core node at a joiner site runs three passes on the joiner and one on the founder (`crates/goatbuilder/src/joinnode.rs:227` to `crates/goatbuilder/src/joinnode.rs:236`):

1. the joiner is bootstrapped with replication off, so the empty database gets its schema once;
2. `site-seed-from-peer.yml` copies the founder's identity and mesh rows into it, data only (`ops/ansible/site-seed-from-peer.yml:13`); the seed must come before any subscription (`ops/ansible/site-seed-from-peer.yml:40`);
3. the joiner is bootstrapped with replication on;
4. `spock-mesh-wire.yml` runs on the founder alone, creating the founder's node and its subscription to the joiner without re-running the founder's bootstrap (`ops/ansible/spock-mesh-wire.yml:14`).

The founder side is additive. Subscriptions are created with `synchronize_data := false` (`ops/ansible/roles/spock-mesh-peering/README.md:81`), so no table on the founder is truncated or reloaded; the role creates the founder's node when it is missing (`ops/ansible/roles/spock-mesh-peering/tasks/per-database.yml:269`); and the seed play refuses to empty a table on a node that already replicates (`ops/ansible/site-seed-from-peer.yml:32`).

**What growth must change on the goatlite first, all in its own group_vars:** `participates_in_spock: true` and an explicit topology naming the joiner, which `spock-mesh-wire.yml` requires before it acts (`ops/ansible/spock-mesh-wire.yml:22`); for the Ory databases, `ory_spock_enabled`, because a founder that never federated has no Ory replication node for the joiner to subscribe to (`ops/ansible/spock-mesh-wire.yml:29`). The joiner copies the founder's cluster-shared secrets and never generates its own (`ops/scripts/newsite-secrets.sh:6`). The two sites reach each other over the goatnet's mesh, so a joiner that is not on the founder's LAN needs reach (step 2) first.

**What it costs afterwards.** Spock replicates rows, not schema changes; with two sites, an image upgrade that migrates the schema is a coordinated two-site operation (`docs/design/pgbackrest.md:26`).

**The other second core: a standby at the same site.** An intra-site pair adds a B host that streams from the running primary; the role detects a healthy standby and never rebuilds the primary (`ops/ansible/roles/postgres-streaming-replication/README.md:5`). The primary then commits with `synchronous_commit = remote_write` (`ops/ansible/roles/postgres-streaming-replication/README.md:18`), so its writes wait on the standby. A goatlite's inventory already names its one host as router role A (`ops/scripts/goatlite-render.py:1157`).

The design document this repository's Spock role points to for the multi-site design, `ha-multi-site.md`, is not in this repository (`ops/ansible/roles/spock-mesh-peering/README.md:218`); the statements above rest on the plays and the role, which are.

## 6. The hand-over, and the secrets store

**The hand-over (step 3, gate row G1).** The full configuration is what `derive_lite` already computes from the four facts (`crates/goatbuilder/src/config.rs:411`): one environment, one founder site with class `core`, one promotion rung, the secrets store, and `repo` and `components` from the checkout. A unit test already shows that shape validating as a declaration in its own right (`crates/goatbuilder/src/config.rs:815`). `grow` writes it with `preset: full`, without the `lite:` block and with `lite.ssh_user` as `deploy.ssh_user`, to `ops/goatbuilder/goatbuilder.yaml`, where a full run reads it, shows the difference, and removes `goatlite.yaml`, so no later run takes the lite path. A four-fact file in a checkout with no config of its own has no `repo` or `components`; the lite preset takes them from the checkout's config, so `grow` asks for them then. The host, user, zone and administrator are not lost: the render already wrote them into the inventory and group_vars, and those files become the learner's. The renderer treats a file whose generated header is gone as the learner's and never overwrites it (`ops/scripts/goatlite-render.py:1844`), so the hand-over removes the headers. After the hand-over the renderer finds no lite file and refuses (`ops/scripts/goatlite-render.py:121`), which also stops `goatlite-render.py --remove` from deleting the CA material it would otherwise remove (`ops/scripts/goatlite-render.py:2150`).

A standup plan from the grown configuration changes nothing that exists. Against the lite plan it loses the two render gates (`crates/goatbuilder/src/standup.rs:375`) and the `--mint-ca-if-missing` flag (`crates/goatbuilder/src/standup.rs:414`); its remaining steps are the same commands, and each skips what already exists (a leaf certificate that exists is skipped, `ops/scripts/mint-site-certs.sh:243`). After the hand-over `--site` must be passed, because only a lite run derives it (`crates/goatbuilder/src/standup.rs:135`).

Three behaviours of a lite plan are decided today by the preset rather than by a declared value, so a plan from the grown configuration loses them; each needs a declared source before G1 passes:

| Behaviour | Today, under `preset: lite` | Declared source after the hand-over |
|---|---|---|
| No cloud account: every play runs with `GOAT_AWS_MODE=none` | `crates/goatbuilder/src/standup.rs:299` | an explicit declaration in the full configuration that this goatnet has no cloud account |
| The bootstrap owner is the declared administrator | `crates/goatbuilder/src/standup.rs:243` | `site_admins` in the site's group_vars, which the render wrote (`ops/scripts/goatlite-render.py:669`) |
| Phase 14 does not reach other goatnets' relay-health boxes | `crates/goatbuilder/src/standup.rs:591` | the site's `relay_health` absent row in `ops/goatnets/cross-env.yaml`, which the render wrote |

**The secrets store (step 4).** The values move; none is generated again. The NetBird encryption key encrypts rows already in the database, and the replication password is shared with any site that joins later. Phase 3 mints only what is missing from the declared store and never rotates what is there (`ops/scripts/newsite-secrets.sh:5`), so switching the store before the values are in it would mint a second set for a running goatnet. `ops/secrets/posture.py` has no command that copies values between stores today; the step needs one.

## 7. What is unproven

Everything on hardware. No step of `grow` has run on a machine: not the custody upgrade, not the hand-over followed by a standup, not a join to a founder that began as a goatlite, not an obs host or standby added to one. The reach step is rendered and gated but has not been stood up against real relays ([goatlite.md §6](goatlite.md#6-growing-out-of-lite)). `join-node` is built from a recorded run of the procedure and is unmeasured on hardware through `goatbuilder` (`crates/goatbuilder/src/joinnode.rs:39`). Growth proven on machines, step by step, comes after 1.0.

## 8. Gate rows

| Row | Proven by | Status |
|---|---|---|
| G1 | On the lite fixture `ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite`, `grow --dry-run` produces a full configuration that passes validation, and the standup plan from it, compared with the lite plan, runs no step that creates or replaces something that exists | not built |
| G2 | On the same fixture, the count of facts the learner must supply, of prompts and of phases is pinned at today's numbers and may not rise, and `docs/tutorials/00-goatlite.md` gains no step before the goatnet works | not built |

**The first-run budget G2 holds**, measured 2026-11-12 on the lite fixture and held as ceilings by `crates/goatbuilder/tests/goatlite_budget.rs`. A count that drops lowers its ceiling in the same change; a count that rises changes this table and the test together, with the reason.

| Measure | Today | How it is counted |
|---|---|---|
| Facts the learner supplies | 4: `name`, `host`, `ssh_user`, `admin` | the schema's `lite.required`; a file of only those validates |
| Stops for the learner | 1: phase 11, the first dashboard login on a LAN site | `MANUAL` steps in `goatbuilder standup pasture --dry-run -o json` |
| Phases | 12 of 14 (7 and 8 are internet-only) | distinct phase numbers in that plan |
| Steps | 36 | phase lines in that plan |
| Tutorial 00 before the goatnet works | 7 sections, 36 command blocks, 75 lines in them | `docs/tutorials/00-goatlite.md` before `## 8. Use it` |
