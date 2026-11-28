# goatlite growth — a first goatnet grows into a full environment without a tear-down

| Field | Value |
|---|---|
| Status | **Designed**, with step 3 (the hand-over, gate row G1) built as `goatbuilder grow`. Nothing else in this document is built except where a row says so. Nothing in it has run on hardware. |
| Grows | [goatlite.md](goatlite.md) (the lite preset, ADR 1213) |
| Into | the full preset of the goatbuilder config ([schema](../../ops/goatbuilder/schema/goatbuilder-v1.schema.json)) and the procedure in [goatnet-standup.md](../operations/goatnet-standup.md) |
| Gate rows | G1 and G2 of the goatbuilder 1.0 gate |

## 1. What this is, and the rule that wins every conflict

A person who stood up a goatlite and wants a full goatnet keeps the one they have. They run one command, `goatbuilder grow`, and the same box, the same name, the same certificate authority and the same database carry on into a full environment. Nothing is torn down and nothing is stood up a second time.

The rule that decides every trade-off below: **a goatlite's first run does not get heavier to make growth easier.** The first run keeps its one file of four facts (`lite.name`, `lite.host`, `lite.ssh_user`, `lite.admin`; the schema's `lite.required`), asks no new question, prints no new prompt, needs no outside account and gains no phase. Everything growth needs — a second key holder, a secrets store, a public zone, more hosts — is asked for and provisioned when the learner chooses to grow, never in advance. Gate row G2 holds this with a test (§8).

## 2. The command

`goatbuilder grow` takes a running goatlite through numbered steps. What is built today is step 3 alone: `goatbuilder grow` runs the hand-over and nothing else, and the numbered steps, `--from` and `--to` are designed (§6). `--dry-run` prints every step, the configuration it will write (as its difference from `goatlite.yaml`, leaf by leaf), each file it will hand over and the commands it will run, exactly as the real run runs them, and changes nothing. Each step is idempotent and can be resumed with `--from`/`--to`, as `standup` and `join-node` are. Each step asks only for the facts that step needs. A step the learner does not want (an isolated goatnet needs no reach) is skipped as declared.

| Step | What changes | What stays | Status |
|---|---|---|---|
| 1 | Custody of the identity root: the CA key is re-encrypted to two or more holders (§3) | the root certificate, every issued certificate, every host | designed, not built |
| 2 | Reach: the `lite.internet` block, rendered and stood up from phase 5 ([goatlite.md §6](goatlite.md#6-growing-out-of-lite)) | the CA, the SPIRE trust domain, the ACL | render built; not stood up against real relays |
| 3 | Hand-over: `goatlite.yaml` is rewritten into a full configuration with the same values, and the rendered file set becomes the learner's (§6) | every rendered file's content | built: `goatbuilder grow` (gate row G1); no run on hardware |
| 4 | Secrets store: values move from the env-file to the declared store (§6) | every value | designed, not built |
| 5 | Tier ceiling and promotion rungs: `site_deploy_tier_max` raised; rungs added | the running tier-1 services | designed, not built |
| 6 | Observability host: `join-node --class obs` | the core host | built; no run on hardware |
| 7 | Second core: an intra-site standby, or a second site that joins this one (§5) | the first site's data | built; no run on hardware, and never run from a founder that began as a goatlite |

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

**Whether it matters in production.** Inside one goatnet the name is a label; nothing in this repository resolves the trust domain as a DNS name. It starts to matter when another party records it: a trust domain and a URI SAN prefix appear in every federation agreement, and once a partner holds them, renaming needs that partner's work too (`docs/adr/1164-per-site-spire-trust-domains.md:53`). The `dev-` prefix and the word `goatlite` in the root subject show provenance in every chain dump and audit log (`ops/enrollment/README.md:68`); a team that does not want that runs the rollover. The join-node gate that refuses a site trusting the shared unnamed dev root accepts a goatlite CA id, because the id names its own goatnet (`crates/goatbuilder/src/joinnode.rs:495`).

**The one line, printed by `grow` before step 1:**

> pasture keeps its name: it is written into every identity this goatnet has issued (pasture.pasture.lan), so changing it later means a rebuild.

## 5. The database: one site to two

A second site can join a running single site. The procedure exists and treats the running site as the founder: the founder keeps its data, gains a replication node and two subscriptions, and is never re-seeded. It has not been run from a founder that began as a goatlite, and nothing of it has run on hardware through `goatbuilder`.

**What a goatlite already has.** Every mgmt host, a goatlite included, runs the `postgres-spock` image (`ops/pins/fleet-pins.yaml:16`), which ships the Spock extension, a configuration tuned for logical replication, and init scripts that load Spock in each replicated database (`ops/postgres/spock/Dockerfile:27`, `ops/postgres/spock/Dockerfile:30`). The goatlite renders itself as the founder (`goatnet_founder: true`, `ops/scripts/goatlite-render.py:553`) with replication off (`participates_in_spock: false`, `ops/scripts/goatlite-render.py:564`), and phase 3 already generates the cluster-shared replication password (`ops/scripts/goatlite-render.py:645`).

**The join.** `goatbuilder join-node` for a core node at a joiner site runs three passes on the joiner and one on the founder (`crates/goatbuilder/src/joinnode.rs:527` to `crates/goatbuilder/src/joinnode.rs:536`):

1. the joiner is bootstrapped with replication off, so the empty database gets its schema once;
2. `site-seed-from-peer.yml` copies the founder's identity and mesh rows into it, data only (`ops/ansible/site-seed-from-peer.yml:13`); the seed must come before any subscription (`ops/ansible/site-seed-from-peer.yml:40`);
3. the joiner is bootstrapped with replication on;
4. `spock-mesh-wire.yml` runs on the founder alone, creating the founder's node and its subscription to the joiner without re-running the founder's bootstrap (`ops/ansible/spock-mesh-wire.yml:2`).

The founder side is additive. Subscriptions are created with `synchronize_data := false` (`ops/ansible/roles/spock-mesh-peering/README.md:81`), so no table on the founder is truncated or reloaded; the role creates the founder's node when it is missing (`ops/ansible/roles/spock-mesh-peering/tasks/per-database.yml:269`); and the seed play refuses to empty a table on a node that already replicates (`ops/ansible/site-seed-from-peer.yml:32`).

**What growth must change on the goatlite first, all in its own group_vars:** `participates_in_spock: true` and an explicit topology naming the joiner, which `spock-mesh-wire.yml` requires before it acts (`ops/ansible/spock-mesh-wire.yml:22`); for the Ory databases, `ory_spock_enabled`, because a founder that never federated has no Ory replication node for the joiner to subscribe to (`ops/ansible/spock-mesh-wire.yml:29`). The Ory replication roles find the Kratos and Hydra containers by the names a goatlite renders, `ory_spock_kratos_container` and `ory_spock_hydra_container`, which are the names its compose gives them (`mgmt-host-kratos-1`, `mgmt-host-hydra-1`); a joining site that copies the founder's group_vars carries them too. For the NetBird account, both sites declare `spock_mesh_databases` with the `netbird` database in the `netbird_mgmt` replication set, whose tables are `spock_mesh_netbird_mgmt_tables` (`ops/ansible/roles/spock-mesh-peering/defaults/main.yml:102`): ADR 1084 D1 makes management-layer replication opt-in, and without it the subscriptions pull the empty default set and replicate nothing, while the seed and both seed-current checks compare exactly those tables, so the two sites can only drift (measured in a VM founder run: both netbird subscriptions read `replicating` and no netbird table was in any replication set). The joining site's group is added to the goatnet's members in the goatlite's pin sidecar (`ops/pins/lite-pins.yaml`): a goatlite's goatnet is declared only there, and a group that is a member of no goatnet has every service undecided in the end-state model, and its own site named as goatbuilder's environment, so an acceptance row runs for a service the goatlite declares absent and the deploy-ledgers row reads an environment nothing declares (measured in the same run). The joining site declares the same. `goatbuilder join-node` refuses a node or a founder that lacks any of them, the `netbird_mgmt` set with the seed's own tables included, before phase 13 changes anything (GB-417, `ops/ansible/spock-join-replication-declared.yml`). The joiner copies the founder's cluster-shared secrets and never generates its own (`ops/scripts/newsite-secrets.sh:6`). The two sites reach each other over the goatnet's mesh, so a joiner that is not on the founder's LAN needs reach (step 2) first.

**What it costs afterwards.** Spock replicates rows, not schema changes; with two sites, an image upgrade that migrates the schema is a coordinated two-site operation (`docs/design/pgbackrest.md:26`).

**The other second core: a standby at the same site.** An intra-site pair adds a B host that streams from the running primary; the role detects a healthy standby and never rebuilds the primary (`ops/ansible/roles/postgres-streaming-replication/README.md:5`). The primary then commits with `synchronous_commit = remote_write` (`ops/ansible/roles/postgres-streaming-replication/README.md:18`), so its writes wait on the standby. A goatlite's inventory already names its one host as router role A (`ops/scripts/goatlite-render.py:1157`). A goatlite declares no `intra_site_pair`, so growth declares it, with the values the role lists, before the B host is joined; `join-node` refuses a core at the founder's site without it (GB-440), because the node would otherwise come up as a second primary.

The design document this repository's Spock role points to for the multi-site design, `ha-multi-site.md`, is not in this repository (`ops/ansible/roles/spock-mesh-peering/README.md:218`); the statements above rest on the plays and the role, which are.

## 6. The hand-over, and the secrets store

**The hand-over (step 3, gate row G1).** The full configuration is what `derive_lite` already computes from the four facts (`crates/goatbuilder/src/config.rs:411`): one environment, one founder site with class `core`, one promotion rung, the secrets store, and `repo` and `components` from the checkout. A unit test already shows that shape validating as a declaration in its own right (`crates/goatbuilder/src/config.rs:823`). `grow` writes it with `preset: full`, without the `lite:` block and with `lite.ssh_user` as `deploy.ssh_user`, to `ops/goatbuilder/goatbuilder.yaml`, where a full run reads it, shows the difference, and removes `goatlite.yaml`, so no later run takes the lite path. A four-fact file in a checkout with no config of its own has no `repo` or `components`; the lite preset takes them from the checkout's config, so `grow` asks for them then. The host, user, zone and administrator are not lost: the render already wrote them into the inventory and group_vars, and those files become the learner's. The renderer treats a file whose generated header is gone as the learner's and never overwrites it (`ops/scripts/goatlite-render.py:1844`), so the hand-over removes the headers. After the hand-over the renderer finds no lite file and refuses (`ops/scripts/goatlite-render.py:121`), which also stops `goatlite-render.py --remove` from deleting the CA material it would otherwise remove (`ops/scripts/goatlite-render.py:2176`).

A standup plan from the grown configuration changes nothing that exists. Against the lite plan it loses exactly the two render gates (`crates/goatbuilder/src/standup.rs:465` to `crates/goatbuilder/src/standup.rs:473`) and the `--mint-ca-if-missing` flag (`crates/goatbuilder/src/standup.rs:507`), and it runs the acceptance gate (phase 13) that a goatlite skips by default; a gate reads and changes nothing. Its remaining steps, gates included, are the same commands, and each skips what already exists (a leaf certificate that exists is skipped, `ops/scripts/mint-site-certs.sh:243`). After the hand-over `--site` must be passed, because only a lite run derives it (`crates/goatbuilder/src/standup.rs:139`).

Three behaviors of a lite plan were decided by the preset rather than by a declared value, so a plan from the grown configuration would have lost them. The grown configuration now declares each in three optional `deploy` fields, which `config::grow` writes and the standup reads only when they are declared (`crates/goatbuilder/src/config.rs:474`):

| Behavior | Declared by the grown configuration | Read at |
|---|---|---|
| No cloud account: on the LAN profile every play runs with `GOAT_AWS_MODE=none` (a goatlite on the internet profile reaches AWS) | `deploy.cloud: none` (`ops/goatbuilder/schema/goatbuilder-v1.schema.json:170`) | `crates/goatbuilder/src/settings.rs:51`, `crates/goatbuilder/src/standup.rs:389` |
| The bootstrap owner is the declared administrator | `deploy.owner`, from `lite.admin` (`ops/goatbuilder/schema/goatbuilder-v1.schema.json:171`) | `crates/goatbuilder/src/settings.rs:58`, `crates/goatbuilder/src/standup.rs:315` |
| Phase 14 does not reach other goatnets' relay-health boxes | `deploy.cross_goatnet: false` (`ops/goatbuilder/schema/goatbuilder-v1.schema.json:172`) | `crates/goatbuilder/src/settings.rs:65`, `crates/goatbuilder/src/standup.rs:717` to `crates/goatbuilder/src/standup.rs:721` |

A configuration that declares none of the three plans exactly as before. `crates/goatbuilder/tests/full_plans_unchanged.rs` holds this for every fixture site without a `lite:` block. A pull request that changes the record (`crates/goatbuilder/tests/fixtures/full-plans.golden.json`) on a base older than the newest change on `main` to `crates/goatbuilder/src/standup.rs`, `ops/goatbuilder/fixtures/` or the record itself is refused by `ops/ci/check-golden-base-current.py` (docs-guards) until it is rebased and the record regenerated (built).

**The command, as built.** `goatbuilder grow` reads `goatlite.yaml` at the checkout root (`crates/goatbuilder/src/grow.rs`). Before it changes anything it shows:
- the full configuration's difference from that file, leaf by leaf;
- the configuration file it will write;
- each file the hand-over will take the generated header off, which it lists by running the same renderer command with `--check`;
- the removal of `goatlite.yaml`.

The renderer command in that plan is the one the real run executes, verbatim. `--dry-run` stops there and writes nothing. Without `--dry-run` it is a mutation. At a terminal it shows that plan and asks, and anything but a typed yes stops it with nothing changed. It asks only when the question can be answered, with stdin and stderr both a terminal. Otherwise it takes `--yes` (C10, GB-008), including a terminal whose stderr goes to a file (`goatbuilder grow 2>file`), and with `--yes` it does not ask. It then hands the goatlite over in this order: it writes `ops/goatbuilder/goatbuilder.yaml` (through a file beside it and a rename), runs `goatlite-render.py --hand-over`, which takes the generated header off every whole file the render wrote (`ops/scripts/goatlite-render.py`, `hand_over`), and removes `goatlite.yaml` last. Marker blocks in shared files keep their comments: once `goatlite.yaml` is gone the renderer refuses to run, so nothing rewrites them. A run that stopped part way is run again: a `goatbuilder.yaml` that already holds the grown configuration is kept and the run carries on, and a failed hand-over keeps `goatlite.yaml`. Run again after a finished hand-over (no `goatlite.yaml`, and `ops/goatbuilder/goatbuilder.yaml` carrying grow's header), it says the goatlite was already handed over and exits 0 with nothing changed. It refuses with a catalog code and changes nothing when `goatlite.yaml` is missing (GB-244) or not YAML (GB-245), is not a goatlite (GB-304: another preset, or no `lite:` block), when `ops/goatbuilder/goatbuilder.yaml` holds another configuration (GB-305), or when the grown configuration does not pass the schema and the cross-field rules (GB-306, which names `repo` and `components` for a four-fact file). A YAML file that parses but does not convert to JSON (a key that is not a string, for one) goes through config's load path like any configuration: today that refusal carries no code and exits 2, and once the configuration loader codes it, it is GB-025, exit 1. When the checkout already has its own `ops/goatbuilder/goatbuilder.yaml`, the lite file takes `repo` and `components` from it, and that file is also where grow writes: grow keeps it when it already holds exactly the grown configuration ("already holds the grown configuration; kept") and otherwise refuses with GB-305. A write or the removal failing is GB-254. The renderer failing, or refusing to read the goatlite (its `--check` exits 1 for both "would change" and a refusal, so grow reads its report's heading), is GB-258, with the renderer's own words. It does not move secrets (step 4) and does not touch a host. Because it changes only files in the checkout, a finished `grow` is undone with git before it is committed: restore `goatlite.yaml` and the handed-over files, and delete `ops/goatbuilder/goatbuilder.yaml`. When `ops/goatbuilder/goatbuilder.yaml` carries grow's own header but no longer holds what `goatlite.yaml` grows into, GB-305 says it is grow's file from an earlier run, since edited.

| Run | Shows | Writes |
|---|---|---|
| `goatbuilder grow --dry-run` | the difference, the config to write, each file to hand over, the renderer command, the removal | nothing |
| `goatbuilder grow` at a terminal (stdin and stderr) | the same, then asks | on a typed yes: the config, the headers taken off, `goatlite.yaml` removed |
| `goatbuilder grow --yes`, or no terminal (or stderr not a terminal) with `--yes` | the difference and each step as it runs | the same, without asking |

A grown goatlite is stood up with `goatbuilder`. `ops/scripts/goat-standup.sh` does not read these three fields, and the script route is not supported for a grown goatlite.

**The secrets store (step 4).** The values move; none is generated again. The NetBird encryption key encrypts rows already in the database, and the replication password is shared with any site that joins later. Phase 3 mints only what is missing from the declared store and never rotates what is there (`ops/scripts/newsite-secrets.sh:5`), so switching the store before the values are in it would mint a second set for a running goatnet. `ops/secrets/posture.py` has no command that copies values between stores today; the step needs one.

## 7. What is unproven

Everything on hardware. No step of `grow` has run on a machine: not the custody upgrade, not the hand-over followed by a standup, not a join to a founder that began as a goatlite, not an obs host or standby added to one. The reach step is rendered and gated but has not been stood up against real relays ([goatlite.md §6](goatlite.md#6-growing-out-of-lite)). `join-node` is built from a recorded run of the procedure and is unmeasured on hardware through `goatbuilder` (`crates/goatbuilder/src/joinnode.rs:63`). Growth proven on machines, step by step, comes after 1.0.

## 8. Gate rows

| Row | Proven by | Status |
|---|---|---|
| G1 | On the lite fixture `ops/goatbuilder/fixtures/goatnet-from-scratch/00-goatlite`, `grow --dry-run` produces a full configuration that passes validation, and the standup plan from it, compared with the lite plan, runs no step that creates or replaces something that exists | built: `goatbuilder grow` (`crates/goatbuilder/src/grow.rs`) on `config::grow`; proven by `crates/goatbuilder/tests/grow.rs` (the dry run on the fixture), `grow::tests` (the hand-over, the re-run and the refusals), the plan test in `crates/goatbuilder/src/config.rs`, `crates/goatbuilder/tests/full_plans_unchanged.rs` and `tests/test_goatlite_render_hand_over.py`; no run on hardware |
| G2 | On the same fixture, the count of facts the learner must supply, of prompts and of phases is pinned at today's numbers and may not rise, and `docs/tutorials/00-goatlite.md` gains no step before the goatnet works | built: `crates/goatbuilder/tests/goatlite_budget.rs` |

**The first-run budget G2 holds**, measured 2026-11-12 on the lite fixture and held as ceilings by `crates/goatbuilder/tests/goatlite_budget.rs`. A count that drops lowers its ceiling in the same change; a count that rises changes this table and the test together, with the reason.

| Measure | Today | How it is counted |
|---|---|---|
| Facts the learner supplies | 4: `name`, `host`, `ssh_user`, `admin` | the schema's `lite.required`; a file of only those validates |
| Stops for the learner | 1: phase 11, the first dashboard login on a LAN site | `MANUAL` steps in `goatbuilder standup pasture --dry-run -o json` |
| Phases | 12 of 14 (7 and 8 run on the internet profile only) | distinct phase numbers in that plan |
| Steps | 36 | numbered phase rows in that plan |
| Titled rows | 38: the 36 steps and the two image-pull rows, pinned by name | every `=== … ===` row in that plan |
| Commands | 33 rows holding 65 shell command separators (59 until 2026-11-16, when each of the six plays limited to the founder gained one `&&` for its GB-355 check, which refuses a limit that selects no host instead of passing; no step, prompt or fact was added) | command rows in that plan, and the `&&`, `\|\|`, `;` and `\|` inside them, so work appended to an existing step counts |
| Tutorial 00 before the goatnet works | 7 numbered sections, 18 headings, 36 command blocks with 75 lines, 158 inline code spans | `docs/tutorials/00-goatlite.md` before `## 8. Use it`: every heading at any level, numbered or not; fenced blocks, indented ones included |

**What the budget cannot see.** A prompt a play or script asks during a real run is not in the dry run, which runs nothing; nor are the pages the tutorial links the learner to. Both are held by review until a run on a machine records them.
