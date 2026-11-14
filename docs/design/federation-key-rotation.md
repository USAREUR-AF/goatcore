# Federation key rotation

| Field | Value |
|---|---|
| Status | **Designed.** Nothing in this document is built except where a row says so. Explicit leaf re-issue (`peering_leaf_material.py --rotate`) is built and is run by hand; no `goatbuilder` path selects it. Gate row B3 of the goatbuilder 1.0 gate (`docs/design/goatbuilder-v1-gate.md`, in review) is not built |
| Gate row | B3: "Keys can be rotated. Every phase can be run again after a rotation and the peering keeps carrying data", proven by "a scenario that rotates each key in turn, re-runs the command, and checks data arrives throughout" |
| Companions | [federation-certificate-lifecycle.md](federation-certificate-lifecycle.md) (leaf reuse and `--rotate`), [federation-package-spec.md](federation-package-spec.md) §5 and §5.1 (the two bound values), [peering-directory-service.md](peering-directory-service.md), [peering-forwarder.md](peering-forwarder.md), [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) |
| Audience | Whoever builds rotation into `goatbuilder federate`, whoever writes the laboratory scenario that proves it, and the reviewer of gate row B3 |

## 1. What rotation means for a peering

A peering between two sovereigns uses eight kinds of key and certificate on our side, and the partner has its own. Two of ours, the peering leaves and the forwarder carrier keys, can be replaced without the partner doing anything, and SPIRE's CA rotates itself. The others are bound into what the partner holds, so replacing one is a two-party act. The design below gives each key one rotation procedure, run by one command, and says which phases run again, what the partner must do, and whether data keeps flowing while the two sides are out of step.

Throughout, "the command" is `goatbuilder federate <partner>`, whose fourteen phases are defined in `crates/goatbuilder/src/federate/mod.rs:303-361`: 1 preflight, 2 prepare (interface, keys and SPIRE bundle), 3 `init` (our package), 4 `lint`, 5 the exchange and confirmation 1, 6 `compose`, 7 `verify`, 8 mint the peering leaves, 9 `apply`, 10 admission on our core, 11 the transport, 12 the edge router and the outbound republisher, 13 SPIRE federation, 14 acceptance.

A resumed run skips a phase only when the phase is skippable, its journal record says it passed, and its recorded input and output digests still match; the first phase that must run again ends all later skipping (`crates/goatbuilder/src/federate/run.rs:412-419`). Only the MANUAL phases (5, 10) and phases 3, 6 and 9 are skippable (`run.rs:205-207`). Every other phase always runs. Rotation therefore needs to change the right digest, and the rest of the run follows from the journal.

## 2. The inventory

Each row names a key, where it is made, where it lives, who else holds it, and what rotation exists today.

| # | Key | Made by, phase | Stored | Also held by | Rotation today |
|---|---|---|---|---|---|
| K1 | **Sovereign signing key** (Ed25519 seed) | By hand, once. `init` will not invent one (`crates/goat-federation/src/verbs/init.rs:29-31`) | A path in `FED_SIGNING_KEY`, from `goat_federation_signing_key` in the edge group's vars (`crates/goatbuilder/src/federate/preflight.rs:403-407`) or `secrets.federation.signing_key` (`crates/goatbuilder/src/federate/declared.rs:79`) | The partner's keyring holds our verifying key: `<work>/keys.txt`, one `<sovereign_id>=<base64>` line per party (`run.rs:130`, `mod.rs:214`). The directory binds the name to it on first write ([peering-directory-service.md](peering-directory-service.md) line 96) | None. The specification requires the first-binding path again (`federation-package-spec.md:612-616`); the runbook says the same (`docs/operations/partner-federation-runbook.md:259-263`) |
| K2 | **Our CA** (the site root) | `ca-init`, through `ops/scripts/mint-site-certs.sh --mint-ca-if-missing` (`mint-site-certs.sh:187-212`); generated once, valid ten years ([internal-tls-ca.md](internal-tls-ca.md) lines 41-43) | Certificate `ops/enrollment/ca/<site>/root-cert.pem`, key `<state_dir>/<site>-ca/ca.key.age` (`ops/ansible/roles/goat-zenoh-portal/defaults/main.yml:215-216`) | Bound into the agreement as our party's `ca_fingerprint` (`crates/goat-federation/src/compose.rs:319-324`). The partner places it as their `partner-ca.pem`, which must hold exactly that one certificate (`crates/goatbuilder/src/federate/admit.rs:606-614`) | None for federation. `--rotate` does not replace `our-ca.pem` ([federation-certificate-lifecycle.md](federation-certificate-lifecycle.md) line 87) |
| K3 | **Peering leaves**: outward (presented to the partner), inward (presented to our core), republisher, optional core | Phase 8, `ops/federation/standup/mint-peering-leaves.sh` through `peering_leaf_material.py`; the core leaf only when `FED_CORE_SAN` is set (`mint-peering-leaves.sh:151-165`) | `<work>/pki/<name>.pem` and `<name>-key.pem`, key mode 0600. Installed by the edge role into `/etc/goat-zenoh-edge-<partner>/tls` when `goat_zenoh_edge_install_certs` is on (default off, `roles/goat-zenoh-edge/defaults/main.yml:62`), by the republisher role into `/etc/goat-fed-republisher-<partner>/` (`roles/goat-fed-republisher/defaults/main.yml:55-56`), and by admission onto our core (`admit.rs:687-692`) | Nobody. The partner trusts our CA by fingerprint and checks the leaf's name; it holds no leaf of ours | **Built:** `--rotate` ("What `--rotate` does today", below). Run by hand only: nothing in `crates/goatbuilder` passes it (`mod.rs:334`) |
| K4 | **WireGuard peering key** (`wg-fed0`) | Phase 2, `wg genkey` guarded by `creates:` (`roles/goat-peering-wg/tasks/transport-key.yml:47-50`, included by `prepare.yml:37-38`); phase 11 imports the same tasks | `/etc/goat-peering-wg/<if>.key` on the edge, never copied off it (`prepare.yml:35-36`) | The public half is our package's `transport_offer.public_key`, read live from the interface by phase 3 (`crates/goat-federation/src/derive.rs:830-859`). The partner's phase 11 reads it from our package, not from the agreement (`crates/goat-federation/src/render.rs:753-757`) | Built as a library and in the role (§7 rows 1c and 4): a rotation id passed to phase 2 sets the key aside once and makes a new one (`roles/goat-peering-wg/tasks/transport-key.yml`), and the link restarts onto it (`tasks/transport-live.yml`). Without an id, phase 2 never rotates it |
| K5 | **Forwarder carrier keys** (edge side and forwarder side), only with a forwarder | Phase 2, `wg genkey` with `creates:` on each host (`roles/goat-peering-wg/tasks/carrier.yml:23-28`, `roles/goat-peering-forwarder/tasks/main.yml:182-190`) | `/etc/goat-peering-wg/wg-carry0.key` and `/etc/goat-peering-forwarder/<if>.key`; the public halves on the controller as `edge-carrier.pub` and `forwarder-carrier.pub` (`crates/goatbuilder/src/federate/forwarder.rs:39-42`) | Nobody outside our own hosts | None |
| K6 | **IPsec leaf and partner CA** (`ipsec-fips` transport only) | By hand, through the offline CA ceremony; the role writes no secret material (`roles/goat-peering-edge/defaults/main.yml:135-138`) | `/etc/swanctl/x509/`, `/etc/swanctl/private/`, `/etc/swanctl/x509ca/` (`roles/goat-peering-edge/defaults/main.yml:164-167`) | The partner's swanctl trusts our IPsec CA, placed by hand | Session rekeying only (IKE 4 h, CHILD 1 h, `roles/goat-peering-edge/defaults/main.yml:116-122`); no credential rotation |
| K7 | **SPIRE server CA and its bundle** | The SPIRE server rotates its own CA within each `ca_ttl` of 2160 h (`roles/spire/defaults/main.yml:29-33`, `:41`); phase 2 exports the bundle to `<work>/ours.spiffe-bundle` (`roles/goat-peering-spire/tasks/export.yml:9-31`) | `/opt/spire/data/server/keys.json` (`roles/spire/templates/server.conf.j2:63-67`) | The partner's SPIRE server, which imports our bundle once from the exchanged file and then refreshes it from our bundle endpoint (`https_spiffe`) across the tunnel (`roles/goat-peering-spire/tasks/bundle-import.yml`, `tasks/main.yml:64-82`). A rerun of phase 13 imports the file again only when their server holds no bundle for us, or the file's `spiffe_sequence` is newer than the held one's | **Built:** automatic, while the bundle port is open through the tunnel; a closed port "reads as a partner who never rotated" (`goat-peering-spire/tasks/main.yml:29-34`) |
| K8 | **Directory entry key** | The same seed as K1: it signs the pointer, every upload and the withdraw (`ops/federation/standup/directory.py:481-488`, `:621-632`; `directory_withdraw.py:8-9`) | As K1 | The directory server binds the name to the first key that writes it | None in the client: uploads are signed with the current seed only (`directory.py:481-488`), and a `rotated` reply is only reported (`directory.py:678`) |
| K9 | **The partner's keys** (their K1, K2, K4, K7) | The partner | Ours: their verifying key in our keyring, their CA at `<work>/theirs-ca.pem`, their package at `<work>/theirs.canonical.json`, their bundle at `<work>/theirs.spiffe-bundle` | — | Receiving a new verifying key or CA fingerprint is **built** in part: `--partner-key` or `--partner-ca-fingerprint` makes phase 5 and every later phase run again (`run.rs:405-411`), and with the new key in the keyring the directory step moves aside a package on disk that does not verify under it, with the CA and bundle it vouched for (`directory.py:1010-1033`). A **new package under the same key** (their K4) is not received: the directory never replaces a package, CA or bundle already on disk (`directory.py:1320-1331`, which only warns "the file on disk is what composes"; [peering-directory-service.md](peering-directory-service.md) lines 294-301) |

### What `--rotate` does today

`peering_leaf_material.py --rotate` (`ops/federation/standup/peering_leaf_material.py:160-281`) replaces every leaf pair named with `--leaf`, and only those; `mint-peering-leaves.sh --rotate` passes it through (`mint-peering-leaves.sh:62`, `:220`). Without `--rotate`, a missing pair is issued, a valid pair is reused, and a pair that fails validation (including one valid for less than `--min-valid-seconds`, default 86400, `:167`) is refused with "inspect it before using --rotate" (`:212-222`). With it, every named pair is issued again. The steps are:

1. It takes a non-blocking `flock` on `<out>/.mint.lock` and refuses if another run holds it (`:190-198`).
2. It refuses to start while `<out>/.rotation-pending` exists (`:199-201`).
3. Because something is to be issued, it needs our CA key, and it checks that the key matches the CA certificate (`:228-234`).
4. It mints every pair into a private staging directory under `<out>` and validates each one before touching a target (`:235-242`).
5. When pairs already exist, it copies them into a new private `<out>/.previous-<random>/` (`mkdtemp`, `:243-247`).
6. Only then (a backup exists) it writes `.rotation-pending` as `{"backup": <the backup directory's name>, "leaves": [<names>]}`, syncs it to disk, and moves each staged file over its target with `os.replace` (`:248-259`).
7. On an `OSError` it copies the backup over what it had replaced, deletes newly installed files that had no previous version, removes the journal and re-raises (`:260-270`). On success it removes the journal, places `our-ca.pem` if its content differs (`place_our_ca`, `:127-141`), and prints where the previous material is kept (`:271-279`).

A crash between steps 6 and 7 leaves the journal, and every later run refuses until an operator restores or completes the set by hand ([federation-certificate-lifecycle.md](federation-certificate-lifecycle.md), "Explicit replacement"). New leaves are valid for `--days`, default 90 (`peering_leaf_material.py:166`, `FED_LEAF_DAYS` in `mint-peering-leaves.sh:56`).

## 3. Each key: what rotation does

For every key the procedure has the same shape:

1. Set the old material aside, never delete it.
2. Make the new material.
3. Run the command again from the first phase whose input changed.
4. Stop at the human confirmations the key requires.
5. End with phase 14.

The table gives the parts that differ. "Gap" means a window in which data does not flow.

| Key | Our side | The partner's side | Phases that run again | Overlap | If the partner has not acted yet |
|---|---|---|---|---|---|
| K3 leaves | Phase 8 with `--rotate`; phase 10 places the new core leaf (its inputs include the minted files, `run.rs:179-183`, so the review stops again); phase 12 installs the new edge and republisher leaves and restarts each once | Nothing | 8 to 14 run (a run from 8 skips nothing, `run.rs:277`); 10 stops for a new approval, since its inputs include the new leaves | **Not needed.** The partner accepts any leaf under the CA the agreement names with the expected name, so the new leaf is accepted on first use. The gap is the edge's and republisher's restart | Nothing breaks |
| K5 carrier keys | Set both carrier keys aside; phase 2 makes new ones and exchanges the public halves through the controller in its own sequence (prepare, keys, forwarder, keys, prepare: `crates/goatbuilder/src/federate/forwarder.rs:343-360`) | Nothing | 2, then the always-run phases | **No.** The edge's carrier holds exactly one peer (`roles/goat-peering-wg/tasks/main.yml:188-198`). The gap lasts from the edge's new key until the forwarder holds it, inside one phase 2 run | Nothing breaks |
| K4 WireGuard | Set the key aside; phase 2 makes a new one and restarts the link onto it, because phase 3 reads the key from the interface; phase 3 measures it into a new package; phase 5 sends that package to the partner | Phase 5 receives our new package; phase 11 replaces our peer key (`roles/goat-peering-wg/templates/goat-peering-wg-link.sh.j2:25-29` keeps exactly one peer) | Ours: 2 to 14. Theirs: 5 to 14; their phase 10 finds nothing new to admit and must not restart their core (§7 row 1b) | **No.** Each interface holds one peer, so the tunnel is down from our phase 2, when the link restarts onto the new key, until their phase 11. The agreement does not carry the key (`federation-package-spec.md:1113-1116`), so its digest is unchanged and no reconfirmation is needed for the key itself | The tunnel stays down, and every data path with it, until the partner's phase 11 runs with our new package. Today their phase 5 keeps our old package, because it still verifies under our unchanged signing key and the directory never replaces a package on disk (K9), so their phase 11 keeps the old peer key. §5 adds the confirmation that receives it |
| K1 signing key | Phase 3 signs our package with the new seed (phase 3's inputs are the seed's bytes, `run.rs:172`, so the run starts at 3); phase 5 shows the new verifying key to read aloud (`crates/goatbuilder/src/federate/confirm.rs:344-357`) | Confirmation 1 again: the partner binds our new key with `--partner-key` (`run.rs:405-411`); their phase 6 composes with it | Ours: 3 to 7, then the rest. Theirs: 5 onwards | **Not needed for data.** No data-path component reads K1: the agreement carries no verifying key (`compose.rs:319-345`), so the leaves, the transport and the admission are unchanged | Their next `compose` refuses our new package (`compose.rs:201-245`). The running peering keeps carrying data, and their term runs out at the agreement's `expires` (`compose.rs:264-272`) |
| K8 directory key | As K1. The directory requires an update to be signed by the key it replaces (`federation-package-spec.md:612-616`); the client must sign the first write under the new key with the old one as well | Nothing beyond K1 | As K1 | Needed: the old seed signs the hand-over | Not applicable |
| K7 SPIRE | Nothing: SPIRE rotates its own CA | Nothing, while our bundle endpoint is reachable through the tunnel | 13 and 14 on a rerun only | **Built** by SPIRE: the bundle carries the old and new authorities through `ca_ttl` | If our bundle port is closed, the partner's SPIRE keeps our old bundle and rejects our new SVIDs |
| K2 our CA | Phase 8 re-issues every leaf under the new CA; our edge trusts the new CA for the core (`our-ca.pem`); phase 3 measures the new `ca_fingerprint` | Confirmation 1 again with `--partner-ca-fingerprint`; the new CA placed as their `partner-ca.pem`; their phase 10 and 12 | Both sides from 3, through a new agreement | **Not possible today.** Every check accepts exactly one CA per party: admission (`admit.rs:606-614`), the edge role ([federation-certificate-lifecycle.md](federation-certificate-lifecycle.md) lines 165-169), and phase 5's single fingerprint (`confirm.rs:362-379`). See §4 | The partner's edge refuses every leaf we issue under the new CA |
| K6 IPsec | `rotate … ipsec-leaf` or `rotate … ipsec-ca`, from material the offline ceremony issued and an operator staged on the edge (§8) | For `ipsec-ca`: places our new CA beside the old one, then removes the old one (§8.3); for `ipsec-leaf`: nothing | 11, 14 | **For `ipsec-ca`, yes:** the partner trusts both of our CAs from the announce to the retire, so the switch has no gap beyond the IKE re-authentication. `ipsec-leaf` needs none: the CA is unchanged | `ipsec-leaf`: nothing breaks. `ipsec-ca`: the switch is refused until the partner has read back our new CA's fingerprint (§8.3) |

### Retiring the old material

Rotation in this design replaces material; it does not revoke it. A replaced leaf keeps the same name and the same trusted CA, and nothing on the path checks a CRL or OCSP (`roles/goat-peering-edge/defaults/main.yml:140-158` states this for the fabric's access layer), so the old leaf still authenticates until its `notAfter`. A leaked leaf key therefore stays usable for the rest of its validity, 90 days by default. The fabric leaves are issued by the site CA, which every peering and every service shares, so no step retires one peering's old leaf early. A shorter `FED_LEAF_DAYS` shortens the window. The IPsec transport differs: its partner CA file is per peering, so rotating that CA retires exactly that partner's leaf (same lines). For WireGuard, the old key is retired the moment the partner's phase 11 runs, because each interface holds exactly one peer.

## 4. The CA overlap

Rotating our CA (K2) cannot keep data flowing with today's checks, because every check accepts exactly one CA per party. The design makes the agreement name a party's next CA during a rotation, and makes each check accept exactly the set the agreement names. The cutover then becomes three steps in which data keeps flowing:

1. **Announce.** Our package carries `ca_fingerprint` and `ca_fingerprint_next`. Both values are read aloud at confirmation 1. The partner composes, and admission and the edge role place both certificates in `partner-ca.pem`. The partner now trusts both CAs for us. Our own `our-ca.pem` holds both as well.
2. **Switch.** Phase 8 `--rotate` re-issues our leaves under the new CA, and phases 10 and 12 install them. Both sides already trust the new CA, so data keeps flowing; the only gap is the edge restart (as for K3).
3. **Retire.** Our package drops the old fingerprint, and `ca_fingerprint_next` becomes `ca_fingerprint`. A new agreement is composed, and each side's checks remove the old CA. Every leaf of the old CA stops being accepted at that moment.

This changes the package specification (§5.1 binds one fingerprint per party) and the agreement schema, so it needs an ADR that supersedes those lines before it is built. That ADR must also settle version skew, the normal case between two sovereigns. A reader on an older goatcore reads only the members it requires (`parse_package`, `crates/goat-federation/src/package.rs:137-230`), so it would ignore `ca_fingerprint_next` and keep trusting only the old CA, and the switch would cut it off. The overlap is therefore announced only to a partner whose own package says it reads the member, and `rotate … ca` refuses before the switch otherwise. Rotating the site root affects every service under the site CA, not only federation ([internal-tls-ca.md](internal-tls-ca.md) lines 365-366 name the larger ceremony). The federation part designed here is the follow-through on a site CA rotation that has already happened.

## 5. The command

A rotation is one command with one decision: which key. This follows [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) decision 2 (one command, one decision) and keeps its two human confirmations (decision 3) exactly where a key requires them.

```text
goatbuilder federate rotate <partner> <key>      key: leaves | carrier | transport | signing | ca
```

`ca` is refused until the overlap of §4 is built. `rotate` is a stage word, like `deploy`, `test`, `status`, `allocate` and `backout` (`crates/goatbuilder/src/main.rs:440-444`). For the named key it:

1. **Takes the peering's lock** and refuses while a rotation of this peering is unfinished, using a `.rotation-pending` journal of the same shape as the leaf helper's.
2. **Sets the old material aside** in a private `.previous-<stamp>/` beside it. For `leaves` this is the helper's own `--rotate`. For `transport` and `carrier` it moves the key files on the edge (and on the forwarder) aside, so the `creates:` guards in phase 2 make new ones.
3. **Records the rotation in the run journal**, so the first changed digest starts the run at the right phase. It writes `<work>/rotation.json` (the key and a nonce), which phase 3 reads as an input when the file exists. Today phase 3's inputs are only the signing key (`run.rs:172`), so a new WireGuard key would let a resumed phase 3 be skipped and leave a stale package. The record is read only when present, so a journal written before it existed keeps its digest and an upgrade reruns nothing; the measured public key itself cannot be an input, because it is known only by measuring the edge, which is phase 3. The CA certificate joins the inputs with the CA overlap (§4), the only rotation that changes it.
4. **Runs the command again from there.** It stops only at the confirmations the key requires:
   - **Confirmation 1** for `signing`, `ca` and `transport`: the partner needs a new package. For `transport` it is a package delivery, not a re-binding.
   - **Admission** for `leaves` and `ca`: the core's leaf changes. The approval digest covers the staged files, so an old approval does not carry over (`admit.rs:820-833`).
5. **Ends with phase 14.** It prints which side still has to act, with the partner's exact command.

The partner side needs no new verb. A partner answering our rotation runs the ordinary command. For `signing` or `ca` it passes the new value with `--partner-key` or `--partner-ca-fingerprint`, which already makes phase 5 and everything after it run again (`run.rs:405-411`).

For `transport` the partner runs `goatbuilder federate <partner> --from 5`. A resumed run would skip phase 5, whose inputs (our package, the partner's package on disk and the partner's CA) have not changed (`run.rs:174`, `run.rs:205-207`), and `--from` turns skipping off (`run.rs:277-285`). Their phase 5 must then receive our new package, which today it does not: it verifies under the key they already hold, and the directory step never replaces their copy (K9). The same holds on the hand-exchange path (`FED_NO_DIRECTORY=1`), where a person copying our new file over `<work>/theirs.canonical.json` changes phase 5's input and nothing asks anyone; the agreement digest does not change, because the transport key is not in the agreement (`federation-package-spec.md:1113-1116`), so confirmation 2 does not stop either.

**Proposed** ([ADR 1229](../adr/1229-a-partners-new-package-is-confirmed-within-confirmation-1.md), amending [ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) decision 3): **a partner's new package under the same key is confirmed within confirmation 1.** When the directory, or the hand exchange, offers a package that verifies under the bound key and differs from the one on disk, phase 5 shows which members changed (for a transport rotation, `transport_offer.public_key`) and the new package's digest. The two sides read that digest aloud on the call, as confirmation 1 already does for keys, and a person confirms it (`yes` at the terminal, or `--accept-partner-package <digest>`). Only then is the old package moved aside to `<work>/theirs.canonical.json.previous-<UTC time>` and the new one placed. Without the confirmation the run stops and names the digest (GB-192's family, as for a package held back from their pointer, `confirm.rs:491-609`), and `--yes` never confirms it. It is journaled with confirmation 1, bound to the digest. Placing a package because it verifies was rejected: a package's `issued` is a date only (`federation-package-spec.md`, line 50), so two packages of one day are not ordered, and a directory could replay an older one.

`rotate` never runs on its own initiative. Phase 8's existing rule stands: an existing leaf that fails validation is refused with "inspect it before using --rotate" (`peering_leaf_material.py:216-220`). The comment in `run.rs:25-26` said phase 8 "renews them when needed", which the helper does not do; build item 1 corrects it.

## 6. The proving scenario

The scenario proves gate row B3 in the federation test environment: each key is rotated in turn on a running peering, the command is run again on both sides, and a continuous stream of samples arrives throughout. It belongs in `ops/federation-e2e/scenarios/catalog.yaml`, beside `rerun-every-phase` (`catalog.yaml:173-196`). That scenario's check `rotation-explicit-only` (its step `rotation-only-on-request`, `catalog.yaml:195`) needs the explicit request path of §5 before it can pass.

**Unit and setup.** The `forwarder` unit (`catalog.yaml:633-635`): a pair of sovereigns (`xaa`, `xab`) peered over WireGuard through a forwarder, so that K5 is covered. Strict acceptance passes. A publisher on each side writes one numbered sample every second for the whole scenario, and a reader on the other side records each number it receives with its arrival time: `ops/federation-e2e/lib/flow.sh` (`flow_start … reader`, the receiving core's REST subscription) and `ops/federation-e2e/lib/samples.py` (`record`, and `check` for `samples-<key>`), driven by `ops/federation-e2e/units/rotation.sh` (§7 row 7).

**Steps, in order.** The first side rotates each key, then the second side rotates the same key. After each rotation:

- `goatbuilder federate rotate <partner> <key>` runs on the rotating side.
- The ordinary command, with the new value where §5 says, runs on the other side.
- Phase 14 runs on both sides.

The keys go in this order: `leaves`, `carrier`, `transport`, `signing`, then a forced SPIRE CA rotation (`spire-server localauthority x509 prepare`, then `activate`, on the pinned SPIRE 1.11.2, `ops/pins/fleet-pins.yaml:65`; then a phase 13 rerun). `ca` joins the order once §4 is built.

**Checks** (one per rotated key and side unless stated):

| Check id | Passes when |
|---|---|
| `rotated-<key>-<side>` | The material differs from the material set aside: the certificate serials for leaves, the public key for WireGuard and carrier, the verifying key for signing |
| `kept-<side>` | Every key not named by the step is byte-for-byte unchanged |
| `old-retired-<key>` | The old material no longer works where the design retires it: the old WireGuard public key is in no peer list on the partner's interface; the old carrier key is in no peer list on the forwarder. For leaves the check is the opposite: a session presenting the old leaf still succeeds until `notAfter`, recorded as the exposure the design states (§3, "Retiring the old material") |
| `acceptance-<key>-<side>` | Strict phase 14 exits 0 on both sides, including the SPIRE check (`acceptance.sh:1142-1181`), which needs `FED_SPIRE_OURS` and `FED_SPIRE_THEIRS` set |
| `samples-<key>` | No sample number is missing at either reader across the step. For `transport` and `carrier`, which have a gap by design (§3), the check passes when the missing numbers are re-delivered after the gap; if the fabric does not re-deliver them, the check fails and names the gap's length in seconds |
| `no-pending` | No `.rotation-pending` file remains on either side |
| `rerun-after-<key>` | The ordinary command run again with nothing changed skips 3, 6 and 9 and rotates nothing |

Whether samples published during the WireGuard gap arrive late or are lost depends on the persistent reception that gate row B4 proves. The `samples-transport` check records which, and the result decides whether `transport` needs the two-link overlap that §3 does not design.

## 7. What is to be built

Each piece is one pull request, with its test written first. The Files column names where each change is made.

| # | Piece | Files |
|---|---|---|
| 1 | Correct the phase 8 comment (`run.rs:25-26`), and make phase 3 read `<work>/rotation.json` as an input when it exists. Built. Without the file, all fourteen phase digests are what earlier releases recorded. A stronger form, not designed for now: phase 2 writes the edge's public key to a file in `<work>` (as `roles/goat-peering-wg/tasks/carrier.yml` does for `edge-carrier.pub`), and phase 3 reads that, so any key change reruns phase 3; it costs one phase-3 rerun per peering after the upgrade that adds it | `crates/goatbuilder/src/federate/run.rs` |
| 1b | Phase 10 by the command makes no change when the admission it would make is the one already approved and placed: same approval digest and byte-identical files on the core. It then places nothing and restarts nothing, so a partner's `--from 5` after our transport rotation does not restart their core. **Built**: the "same approval digest" is the digest of what is placed, without the plan hash, recorded only after the restart passed; a target changed by hand or a failed restart is admitted again (`tests/federate_admit.rs`, [goatbuilder.md](goatbuilder.md), confirmation 2). Not run on a live core | `crates/goatbuilder/src/federate/admit.rs` |
| 1c | Requirement on every `rotate` of a key our package carries (`transport`, `signing`, `ca`): write `<work>/rotation.json` once, before the run from 3, and never rewrite or delete it after the new half has been exchanged. Phase 3 hashes the file's bytes, so changing it forces yet another half and another exchange. A later rotation writes a new record in its place, and that is what starts the next rotation. **Built for `transport`** as a library: `rotation::transport_rotation` writes `{"key":"transport","nonce":"<id>"}` before the run from 2, rewrites it only when it does not name the rotation under way (so a resumed rotation leaves its bytes), and never deletes it | `crates/goatbuilder/src/federate/` (the `rotate` stage) |
| 2 | `federate rotate <partner> leaves` (no `rotation.json`: nothing in our package changes): the stage word, the lock and journal, `--rotate` passed to phase 8, the run from 8, the closing print. Built as a library (`crates/goatbuilder/src/federate/rotation.rs`); the `main.rs` stage word is a separate change | `crates/goatbuilder/src/federate/`, the `main.rs` stage word |
| 3 | The edge's own certificate install on by default (`goat_zenoh_edge_install_certs`), so a rotated leaf reaches the edge without a hand copy ([federation-certificate-lifecycle.md](federation-certificate-lifecycle.md), "Installing on the edge") | `roles/goat-zenoh-edge`, the e2e harness |
| 4 | `rotate … carrier` and `rotate … transport`: moving the key files aside on the edge and the forwarder, then the run from 2. **Built for `carrier`** as a library and in the roles: `rotation::carrier_run` runs from phase 2 with the rotation's id kept in `<work>/.rotation-carrier` until phase 2 passes; phase 2 passes the id to its plays, and each role's `tasks/carrier-key.yml` moves the carrier key in place, once, to `.previous-<id>/` (0700) and makes a new one, on the forwarder for the named peering only (`roles/goat-peering-wg/tests/test-rotation.sh`, unit tests in `rotation.rs`). **Built for `transport`** the same way: `rotation::transport_run` runs from phase 2 with the id kept in `<work>/.rotation-transport` until phase 2 passes; goat-peering-wg's `tasks/transport-key.yml` sets the peering key aside and makes a new one, and `tasks/transport-live.yml` restarts the link while the interface holds a key other than the one on disk (`roles/goat-peering-wg/tests/test-rotation.sh`, unit tests in `rotation.rs` and `mod.rs`). Neither is on the command line until the `rotate` stage word is wired. The partner receiving the new package is row 5a's (§5, ADR 1229), not built | `roles/goat-peering-wg`, `roles/goat-peering-forwarder`, `crates/goatbuilder/src/federate/` |
| 5 | `rotate … signing`: the run from 3, and a directory write under the new key also signed by the old one | `crates/goatbuilder/src/federate/`, `ops/federation/standup/directory.py` |
| 5a | The partner's phase 5 taking a newer package under the same key within confirmation 1 (§5, ADR 1229): the directory step reports a newer verifying package in our slot without placing it; phase 5 asks, moves the old one aside and places the new one | `ops/federation/standup/directory.py`, `crates/goatbuilder/src/federate/confirm.rs` |
| 6 | Check that phase 13's `bundle set` from the exchanged file does not replace a newer bundle fetched from the partner's endpoint, and fix it if it does. **Checked and fixed:** it did replace it, on every phase 13, because `bundle set` overwrites the held bundle. The older copy then dropped the partner's new authorities until the next refresh, and that refresh authenticates the endpoint with the held bundle. Phase 13 now reads the bundle held (`bundle list -id`) and imports the file only when none is held, or both carry a `spiffe_sequence` and the file's is greater; `peering_spire_bundle_replace: true` imports it over any held bundle, for a partner whose SPIRE server was rebuilt (`roles/goat-peering-spire/tasks/bundle-import.yml`, `tests/test-bundle-import.sh` with a stand-in `docker`). Not run against SPIRE: whether `bundle list -format spiffe` prints a federated bundle's `spiffe_sequence` on the pinned SPIRE 1.11.2 is not measured; without it the held bundle is always kept, which is the import-once the design intends | `roles/goat-peering-spire` |
| 7 | The scenario of §6, with the numbered once-a-second publisher and the arrival-recording reader on each side. **Built, not measured**: the publisher and reader (`lib/flow.sh`, `lib/samples.py`), and in `units/rotation.sh` the checks `rerun-after-<key>-<side>`, `old-retired-<key>` (WireGuard and carrier; the old leaf's exposure reads BLOCKED) and `samples-<key>`, judged by `samples.py check` (unit-tested in `tests/test_harness.py`). A key the goatbuilder under test refuses (GB-189, GB-282) has its checks marked BLOCKED with the refusal's words, and the unit goes on to the next key. Not driven yet: the forced SPIRE CA rotation (`spire-ca-rotation`, BLOCKED). Not run in the federation test environment; the rotation steps need `rotate` on the command line | `ops/federation-e2e/` |
| 8 | The CA overlap of §4: the ADR superseding §5.1's single fingerprint, then the package and agreement fields, admission, the edge role and phase 5 | `docs/adr/`, `crates/goat-federation`, `crates/goatbuilder/src/federate/admit.rs`, `confirm.rs`, `roles/goat-zenoh-edge` |
| 9 | IPsec credential rotation (K6), §8: the role's staged-material checks, set-aside and switch with roll-back, the second partner CA during an overlap, the `ipsec-leaf` and `ipsec-ca` stage keys, and the tests of §8.6. **Built in the role** (`roles/goat-peering-edge/tasks/rotate.yml`, `tests/test-rotation.sh`); the stage keys and the controller's journal are not built | `roles/goat-peering-edge`, `crates/goatbuilder/src/federate/` |

## 8. IPsec credential rotation (K6)

The role part is built: `roles/goat-peering-edge/tasks/rotate.yml` checks the staged material, sets the live pair aside once per id, switches, re-authenticates and rolls back (§8.3 for `ipsec-leaf` and the switch of `ipsec-ca`), and accepts a partner's next CA beside the current one, tested with a stand-in `swanctl` and certificates made at test time (`roles/goat-peering-edge/tests/test-rotation.sh`). Not built: the `rotate … ipsec-leaf` and `rotate … ipsec-ca` stage keys in goatbuilder and the controller's journal (§8.4, GB-298). Nothing is measured on a running link. This section covers a peering whose agreement chose `ipsec-fips`, which phase 11 stands up with `peering-edge.yml` and `roles/goat-peering-edge`. A `rotate … ipsec-leaf` or `rotate … ipsec-ca` on any other peering is refused before anything runs.

### 8.1 What is rotated

The IPsec transport authenticates with certificates, not with anything the package or the agreement carries. Three items belong to one peering:

| Item | Where it lives on our edge | Who makes it | Who else holds it |
|---|---|---|---|
| **Our IPsec leaf**, certificate and private key, Subject CN = `peering_edge_local_id` | `/etc/swanctl/x509/<name>-cert.pem` and `/etc/swanctl/private/<name>-key.pem` (`roles/goat-peering-edge/defaults/main.yml:165-166`), presented as `local.certs` (`templates/swanctl.conf.j2:43`) | The offline CA ceremony | Nobody: the partner checks it against our CA |
| **Our per-peering IPsec CA**, the certificate only | Not on our edge today | The offline CA ceremony | The partner, as their `<name>-partner-ca.pem` |
| **The partner's per-peering IPsec CA** | `/etc/swanctl/x509ca/<name>-partner-ca.pem` (`defaults/main.yml:167`), the only CA the connection accepts (`templates/swanctl.conf.j2:52`) | The partner | The partner |

The role treats the partner CA file as the revocation instrument: nothing on the path checks a CRL or OCSP, so a replaced leaf stays valid until its `notAfter`, and only removing the CA that issued it retires it (`defaults/main.yml:140-158`). Rotation therefore has two kinds:

- **`ipsec-leaf`**: a new leaf from the same CA. Only our side acts. The old leaf is **not** retired; it still authenticates until its `notAfter`. This is renewal before expiry, not a response to a leaked key.
- **`ipsec-ca`**: a new CA and a new leaf issued by it. Both sides act. When the partner removes our old CA, our old leaf stops authenticating at once. This is the response to a leaked key, and the only rotation that retires anything.

Receiving the partner's new CA, when the partner rotates theirs, is the same procedure with the sides swapped (§8.3, "When the partner rotates").

### 8.2 Who runs it, and where the material comes from

The role writes no secret material and refuses without it (`roles/goat-peering-edge/tasks/main.yml:304-323`), and rotation keeps that rule. The ceremony issues the new material. An operator stages it on the edge beside the live files, under the names the role reads with `.next` before the extension (`<name>-cert.next.pem`, `<name>-key.next.pem` at mode 0600, and for `ipsec-ca` the new CA certificate as `/etc/swanctl/x509ca/<name>-local-ca.next.pem`). The operator then runs:

```text
goatbuilder federate rotate <partner> ipsec-leaf
goatbuilder federate rotate <partner> ipsec-ca  --partner-holds-ipsec-ca <sha256>
```

The command never mints, copies or prints a private key. It checks the staged material on the edge, sets the live material aside, switches, re-authenticates and runs phase 14. For both kinds, a new role variable, `peering_edge_local_ca_path`, names our current IPsec CA certificate on the edge (public material) so the role can check the chain. Without it, a rotation is refused (GB-289).

Generating the new key on the edge, with only a signing request leaving it, would remove the step in which a private key travels to the edge. That changes the role's rule that it writes no secret, so it is left for a later decision and is not part of this design.

### 8.3 The order, and the overlap

**`ipsec-leaf`**, our side only:

1. **Check what is staged**, on the edge and before anything moves: the key matches the certificate; the certificate chains to `peering_edge_local_ca_path`; its Subject CN is `peering_edge_local_id`; its key is ECDSA P-384, which `peering_edge_ike_proposals` requires (`defaults/main.yml:112`); it is valid now and for at least `peering_edge_min_valid_days` more (default 30). Any failure is refused (GB-289 for missing or unreadable files, GB-290 for the rest) and nothing is moved.
2. **Set the live pair aside** into `/etc/swanctl/.previous-<id>/` (0700), once per rotation id, as `roles/goat-peering-wg/tasks/carrier-key.yml` does for the carrier key.
3. **Switch**: move the staged pair over the live names, run `swanctl --load-creds`, then `swanctl --rekey --ike <name> --reauth`. A rekey without `--reauth` keeps the authentication of the old IKE SA, so the new certificate would not be presented until the next re-authentication.
4. **Confirm**: wait up to `peering_edge_dpd_timeout` (150 s by default, `defaults/main.yml:124`) for an established IKE SA and an installed CHILD SA (`swanctl --list-sas --ike <name>`). If neither appears, **roll back**: move the set-aside pair back, load it, re-authenticate, and stop with GB-302. During `ipsec-leaf` the old leaf is still valid under an unchanged CA, so the roll-back restores the link.
5. Phase 14.

**`ipsec-ca`**, three steps, both sides. The overlap runs from step 1 to step 3: during it, the partner accepts leaves under our old and our new CA.

1. **Announce.** We deliver our new CA certificate to the partner, as the old one was delivered, and both sides read its sha256 fingerprint aloud on the call, as confirmation 1 does for keys. The partner places it with a new role variable, `peering_edge_remote_ca_next_path`. When that variable is set, the role renders `remote.cacerts` as the current file and the next one, comma-separated, which strongSwan accepts as a list. The partner runs `goatbuilder federate <us> --from 11` and reads back the fingerprint of the file they placed.
2. **Switch.** We run `rotate <partner> ipsec-ca --partner-holds-ipsec-ca <sha256>`. Without the flag, or with a fingerprint other than our staged CA's, the switch is refused (GB-303) before anything moves: we have no way to read the partner's trust store, so the read-back is the only evidence that they will accept the new leaf. The staged checks are those of `ipsec-leaf`, except that the leaf must chain to the staged new CA. Then steps 2 to 5 of `ipsec-leaf` run. The roll-back works here too, because the partner still trusts the old CA. The new CA then becomes our `peering_edge_local_ca_path`.
3. **Retire.** After phase 14 passes on both sides on the new leaf, the partner moves our new CA over their current file, unsets `peering_edge_remote_ca_next_path` and runs `--from 11` again. Our old leaf now fails at their edge: that is the revocation. Our next run of the command removes the set-aside private key and keeps the set-aside certificate, so a retired key does not stay on the edge.

**Compromise.** When our old leaf's key is believed leaked, the overlap is itself the exposure. `--no-overlap` skips step 1: the partner replaces our CA directly, the link is down from their phase 11 until our switch, and the switch checks the read-back as before. The roll-back is then impossible, because the partner no longer trusts the old CA, and the command says so before it switches.

**When the partner rotates.** We are the partner side. Their new CA goes into `peering_edge_remote_ca_next_path` and we run the ordinary command from 11, then read the fingerprint back to them. At their retire, we move it over `peering_edge_remote_ca_path`, unset the variable and run from 11 again. No new verb is needed.

### 8.4 How a killed run finishes

Each rotation writes `<work>/ipsec-rotation.json` on the controller before anything moves on the edge: the kind, the rotation id, the sha256 of the staged certificate (and of the staged CA for `ipsec-ca`), the partner's read-back fingerprint, and the last step that completed (`checked`, `set-aside`, `switched`, `confirmed`, `retired`). The set-aside directory `.previous-<id>/` on the edge is the second marker. A resumed run takes the peering's lock, reads the journal, and redoes each step only when its effect is missing:

- **Set aside**: only when `.previous-<id>/` does not exist, so a second run never sets aside the new pair as if it were the old one.
- **Switch**: only when the live certificate's sha256 is not the journal's. A run killed after the move but before `--load-creds` finds the files switched, and charon still holding the old credentials in memory. It loads and re-authenticates without moving anything.
- **Re-authenticate and confirm**: always repeated, which is harmless.
- **Roll back**: a run killed during the roll-back finishes it. The set-aside pair is the target, and a live file whose sha256 matches the set-aside one is already done.

A journal whose staged sha256 differs from the `.next` files now on the edge (someone staged other material during a stopped rotation) is refused (GB-298). The operator either restores the files the journal names, or finishes the rotation with `--abandon <id>`, which moves the set-aside pair back if it is not live and removes the journal. No path deletes the set-aside pair before the retire step.

### 8.5 Refusal codes

Five codes, GB-289, GB-290, GB-298, GB-302 and GB-303, reserved for this design. Each prints the host, the file and the fix:

| Code | Refused when |
|---|---|
| GB-289 | A staged file, or `peering_edge_local_ca_path`, is missing or unreadable, or the staged key is not mode 0600 |
| GB-290 | The staged certificate does not match its key, does not chain to the CA it must chain to, names another CN, uses another key type than the proposals require, or is not valid now and for the minimum days |
| GB-298 | A resumed rotation whose journal names material other than what is staged, or another rotation of this peering is unfinished |
| GB-302 | The IKE SA did not come up on the new leaf within `peering_edge_dpd_timeout`; the old pair was restored (or, with `--no-overlap`, could not be) |
| GB-303 | `ipsec-ca` without `--partner-holds-ipsec-ca`, or with a fingerprint other than the staged CA's. The role checks it too, from `peering_edge_rotation_ca_readback` |

A `rotate … ipsec-*` on a peering whose agreement is not `ipsec-fips` is refused with the stage's existing usage code, GB-189.

### 8.6 Tests

Written before the code, as for the other keys:

- **Role, with a stand-in `swanctl` and test certificates made at test time** (as `roles/goat-peering-wg/tests/test-rotation.sh` makes its keys): a key that does not match its certificate, a wrong CN, a certificate from another CA, an RSA key, and one expiring within the minimum days are each refused, and nothing on disk changes. A good pair is set aside once per id, and a second run with the same id moves nothing. A run stopped after the set-aside, and one stopped after the move, each finish without a second move and with exactly one `--load-creds` and one `--reauth` after the resume. A stand-in that never reports an established SA makes the run restore the old pair and exit with GB-302. The retire step removes the set-aside key and keeps the certificate.
- **Template**: without `peering_edge_remote_ca_next_path`, `swanctl.conf` renders byte for byte as today; with it, `remote.cacerts` lists the current file and then the next.
- **goatbuilder unit and binary tests**: the stage keys parse, and are refused for a WireGuard agreement with GB-189. `ipsec-ca` is refused without the read-back and with a wrong one (GB-303). The journal resumes at the right step, and a mismatched journal is GB-298. A peering with no rotation renders phase 11 exactly as before.
- **Not covered by this design's tests**: the federation test environment has no `ipsec-fips` unit (§6 uses the `forwarder` unit), and strongSwan in a container needs the `xfrm` interface and privileges the environment does not grant. So the samples-throughout check for K6 needs a new unit, which is not designed here. Not measured on the pinned strongSwan: whether `--reauth` makes the new IKE SA before the old one is deleted (strongSwan's `make_before_break`), which decides whether the switch has any gap at all.
