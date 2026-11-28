# Bootstrap bundle — cloud-held stateless architecture artifact

> **Reconciled (2026-11-28):** how this design relates to `goatbuilder standup`, the site cache and the pull converger, and what is kept from it, is in [cold-start-designs-reconciled.md](cold-start-designs-reconciled.md) and [ADR 1277](../adr/1277-one-signed-site-bundle-is-the-cold-start-artifact.md) (Accepted, not built).

Design for a sovereign-key-encrypted, cloud-stored **bootstrap bundle**
that contains everything needed to stand up a fresh goat site
or restart the whole system after total loss — and nothing that carries
mission data, identity, secrets, or runtime state.

**Status (2026-04-22):** design locked. Implementation tracked in
`../project/phase3-logistics.md`.

## Table of contents

- [1. Purpose and scope](#1-purpose-and-scope)
- [2. What is in the bundle](#2-what-is-in-the-bundle)
- [3. What is not in the bundle](#3-what-is-not-in-the-bundle)
- [4. Encryption + signing](#4-encryption--signing)
- [5. Storage](#5-storage)
- [6. Build + refresh cadence](#6-build--refresh-cadence)
- [7. Use cases](#7-use-cases)
- [8. Relationship to other artifacts](#8-relationship-to-other-artifacts)
- [9. Open questions](#9-open-questions)

---

## 1. Purpose and scope

The pgBackRest design ([`pgbackrest.md`](pgbackrest.md)) covers
operational-state backup, kept per-site and sovereign. It deliberately
does not survive total-loss of every site. The bootstrap bundle
handles the other half of the recovery story: **"we lost every site;
can we rebuild the *system*, even if we can't rebuild its *state*?"**

The bundle is the shortest path from "operator with a laptop and cloud
credentials" back to "a running goat mgmt-plane." It is not a
backup. It carries zero operational data — every piece of state is
regenerated fresh at provisioning.

Two scenarios the bundle serves:

1. **Rapid new-site provisioning.** New hardware lands at a site;
   pulling the bundle is faster and more reliable than cloning the git
   repo and re-fetching container images from upstream registries under
   tactical-time pressure. Steady-state (post-bootstrap) image
   distribution rides the OCI image distribution substrate
   per ADR 0838;
   the bundle carries the **bootstrap-set** (Forgejo + registry:2 +
   minio + openobserve + otel-collector — see Block 75J in
   `implementation-plan.md`)
   so an Orin-C can come up without network reach to T1, then the
   substrate handles the canonical-set tail.
2. **Total-loss restart from scratch.** Every site destroyed. Operator
   with a YubiKey, a laptop, and internet can recover the system's
   ability to exist — not its history.

The bundle being in cloud is explicitly acceptable per the
sovereignty rule: cloud is OK for encrypted-in-flight transit and for
stateless artifacts; data-at-rest with identity/mission content stays
sovereign on our hardware.

## 2. What is in the bundle

Everything needed to deterministically stand up a fresh mgmt-plane.
**All items are either public information, compile artifacts, or
stateless templates.**

- **Git repository snapshot at a signed tag.** The full goat
  repo at a specific commit — design docs, infra-as-code, Ansible
  roles, scripts, config templates. Captured as a git bundle file
  (`git bundle create`) so restoration is a single `git clone` away.
- **Container image mirror + manifests.** Full image tarballs for every
  service in the mgmt-plane stack (postgres-spock, netbird-management,
  signal-server, Traefik, CoreDNS, Caddy, pgbackrest, chrony,
  node-exporter, Prometheus, Alertmanager, etc.), plus the manifest +
  digest index so content integrity is verifiable. Images are either
  Chainguard-based or built from source per the repo's supply-chain
  policy.
- **Terraform modules + frozen provider versions.** `.terraform.lock.hcl`
  + the Terraform binary appropriate to the repo's required version.
- **Ansible playbooks, roles, and inventory templates.** Including the
  `hosts.ini` template with `TBD_*_MESH_IP` placeholders per the
  bootstrap-vs-refresh pattern documented in CLAUDE.md.
- **First-boot scripts.** `deploy.sh`, `ops/relay/deploy-relay.sh`,
  per-env bootstrap playbooks, NVMe format + mount scripts.
- **Host OS cloud-init / provisioning templates.** Ubuntu 24.04
  cloud-init snippets (x86) + Ubuntu-for-Tegra 22.04 cloud-init snippets
  (Orin), LUKS setup scripts, OP-TEE key-custody helpers for Orin targets
  (Tegra SE-backed; see [`orin-os-decision.md`](orin-os-decision.md)).
- **Offline-CA public certificate.** Ed25519 public key used to verify
  device-held offline enrollment bundles
  ([`offline-enrollment.md`](offline-enrollment.md)). The *private* key
  stays on the operator's YubiKey and never touches the bundle.
- **Empty-state DB schema seeds.** SQL files that initialize the three
  mgmt-plane databases (`netbird`, `netbird_idp`, `netbird_events`) to
  a fresh-install state — same content an operator would see after a
  clean `docker compose up` on day zero.
- **Fresh-secret-generation scripts.** Helpers that generate new
  WireGuard keypairs, new setup keys, new Dex client secrets, new
  Postgres superuser + replicator passwords, new pgBackRest cipher
  passes — at provisioning time, not stored in the bundle.
- **README_RECOVERY.md.** Operator runbook that walks the recovery
  sequence end-to-end from bundle decrypt to first peer enrollment.
- **Pre-built peer-side static binaries** (added 2026-05-04, Block 61).
  The binaries that mesh peers run as systemd services on first boot
  rather than fetching from the network: today
  [`snitch probe`](../../ops/snitch/) (Go static, multi-arch — amd64
  + arm64). Distribution shape: same age-encrypted bundle, no
  separate trust path. Peer-onboarding ansible roles consume the
  binary from the unpacked bundle on the operator workstation, not
  via `curl <collector>/install`. Per-device probe API keys are
  separate — they ride the per-device offline-enrollment bundle as
  the new `snitch_probe_key` field per
  [`offline-enrollment.md`](offline-enrollment.md) §11.6a; the
  bootstrap bundle carries only the binary, not any device-bound
  credential.

## 3. What is not in the bundle

**Zero identity data, zero operational state, zero bearer credentials.**
If any of the following would be captured by copying the bundle, the
copy mechanism has a bug.

- No Postgres dump. No pgBackRest archive. No WAL.
- No `setup_keys` values. No `personal_access_tokens`. No `users` rows.
  No `peers` rows. No `group_peers`. No policies.
- No WireGuard private keys for any host or peer. Keys are generated
  fresh per host at provisioning.
- No OIDC client secrets. No Dex state tables with any rows.
- No pgBackRest cipher keys. Those are per-host and generated fresh.
- No AWS credentials, no 1Password vault exports, no operator SSH
  keys.
- No offline-CA private key. That stays on the operator's YubiKey PIV.
- No TLS private keys. Let's Encrypt DNS-01 re-issues at first boot.
- No per-site hostnames or mesh IPs. Those are chosen fresh at
  provisioning.
- No activity log (`netbird_events`) content.

The test for "should this go in the bundle" is: **if this artifact was
leaked in plaintext, does it reveal anything about real peers, real
users, real mission data, or real network state?** If yes, it does not
go in.

## 4. Encryption + signing

### 4.1 Encryption — age multi-recipient

Built bundle tarball → encrypted with
[age](https://age-encryption.org/) in multi-recipient mode. Each
authorized operator has their own X25519 keypair, private key stored
on a YubiKey PIV slot. Encryption targets all authorized recipients:

```
age \
  -r <alice-pubkey> \
  -r <bob-pubkey> \
  -r <charlie-pubkey> \
  -r <1password-last-resort-pubkey> \
  -o bundle.tar.age \
  -e bundle.tar
```

Any authorized recipient can decrypt with their private key alone. No
quorum required — the threat model for this bundle is availability (key
loss during tactical time) not coercion (the bundle has no identity
data worth coercing operators over).

**Last-resort recipient.** A fourth recipient key lives in the
sovereign 1Password vault (not on a YubiKey) as an operator-of-last-
resort fallback. If every YubiKey is lost or destroyed, the 1Password
vault credential unlocks the bundle.

**Pubkey roster.** Operator public keys are stored in git under
`ops/bootstrap/recipients.txt` — public information, no secrecy
required, audit trail via git log of who had access when.

**Rotation.** Adding or removing an authorized operator requires
re-encrypting the bundle to the new recipient list. Per the manual
refresh cadence (§6), operators add/remove recipients at the next
scheduled bundle rebuild. Emergency revocation (operator terminated
under duress) requires an ad-hoc rebuild + the old bundle's cloud
object is deleted.

### 4.2 Signing — Ed25519 via the offline CA

Before encryption, the bundle tarball is signed with the existing
offline Ed25519 CA on a YubiKey PIV slot — the same CA that signs
device offline-enrollment bundles
([`offline-enrollment.md`](offline-enrollment.md) §2). One key
ceremony, two uses.

```
ssh-keygen -Y sign -f <yubikey-piv-identity> -n bootstrap-bundle \
  bundle.tar > bundle.tar.sig
```

The operator verifies the signature with the CA's public key *before*
decrypting:

```
ssh-keygen -Y verify -f <ca-public-key-allowed-signers> \
  -n bootstrap-bundle -s bundle.tar.sig < bundle.tar.age.decrypted
```

Signature verification catches any tampering of the ciphertext (e.g. an
attacker who gains S3 write access and replaces the object). Signing
does not use up key material — the CA is set up to sign arbitrary
content with a namespace tag (`-n bootstrap-bundle`).

The CA public key is baked into the recovery runbook (README_RECOVERY.md
inside the bundle) so the chain is self-contained: operator needs the
bundle ciphertext, their age private key, and the CA's public key (also
distributed out-of-band to every operator as a known-good artifact).

## 5. Storage

**Sovereignty framing (revised 2026-05-07):** the bundle is age-encrypted
ciphertext signed by the offline CA. **Sovereignty lives in the cipher
chain** (age recipients + Ed25519 signature + offline-CA root + operator
YubiKey), **not in the cloud tier.** Both AWS and Cloudflare are
commercial entities outside our sovereign domain — they are delivery
vehicles for already-signed-and-encrypted ciphertext. A leaked bundle
discloses nothing operational; a tampered bundle fails signature
verification at pull time.

This means the S3 / R2 / sovereign-substrate tiers are **peer mirrors**,
not a "primary + cache" hierarchy. Distinguish them by purpose:

- **Cloudflare R2** — rapid CDS-level forward-staged delivery (free
  egress, edge presence at tactical-edge sites, S3-compatible API). The
  hot path for new-site bringup pulls.
- **AWS S3** — secondary commercial mirror. Convenient because the prod
  AWS account exists; provides an additional Object-Lock-protected
  integrity tier with a different attack surface than Cloudflare.
- **TBD sovereign substrate** (per [ADR 0220](../adr/0220-multi-cloud-multi-substrate-posture.md))
  — availability hedge against simultaneous AWS + Cloudflare failure.

**Bucket layout:**

- **Cloudflare R2 (`image-mirror`)** — image artifacts, served on a custom domain. Bundle ciphertext optionally mirrored here in
  v2 (small artifact, edge benefit marginal).
- **AWS S3 `example-bootstrap-bundle`** — bundle ciphertext +
  signature; Object Lock GOVERNANCE 30d + versioning.
- **AWS S3 `example-image-mirror`** — image artifacts mirror; same
  hardening.

**Object naming:** `<bucket>/bootstrap/<tag>.tar.age` +
`<bucket>/bootstrap/<tag>.tar.sig`. One object per tagged release.

**Retention:** Keep the latest 12 tagged bundles. Never delete the
signed-tag lineage — if anyone pulls an old bundle by tag, it must
still verify.

**Access control:** AWS IAM grants `s3:GetObject` + `s3:PutObject` only
to an operator role that requires MFA. Cloudflare R2 served via
custom-domain GET (no anonymous write). Object Lock GOVERNANCE 30 days
on S3 (protects against accidental `terraform destroy`); R2 has
equivalent Object Lock since 2024.

**Durability:** Each tier's native cross-region durability is
sufficient on its own. The bundle is cheap to rebuild; cross-cloud
mirror is a defense-in-depth posture (different attack surfaces),
**not** a sovereignty posture.

## 6. Build + refresh cadence

Manual refresh, with a visible drift indicator so "we forgot to rebuild"
is a noisy failure instead of a silent one.

- **Trigger:** operator runs `ops/bootstrap/rebuild.sh` on their
  workstation. Script tags the current `main` branch as
  `bundle-vYYYYMMDD`, builds the tarball, signs it with their YubiKey,
  encrypts with the current recipient roster, uploads to S3.
- **Drift counter.** A CI check on every `main` push counts commits
  since the last `bundle-v*` tag. When the count crosses a threshold
  (default 25 commits or 14 days, whichever first), a soft "bundle
  drift" CI check fails and posts a notice to gatewatch. This is a
  *reminder*, not a gate — merges continue, only the notice fires.
- **Expected cadence:** roughly every 2–4 weeks during active
  development, quarterly during steady-state.
- **Emergency rebuild:** required when revoking an operator's
  recipient key (ad-hoc; not waiting for cadence).

No automated-rebuild-on-push. Rationale: the bundle is a
security-critical artifact; the one human who signs it should see
every rebuild as a deliberate act, not a CI side effect.

## 7. Use cases

### 7.1 Rapid new-site provisioning

New mgmt-host pair hardware at a new site.
Operator workflow:

1. Pull latest bundle from S3 to the operator laptop.
2. Verify signature + decrypt (age private key on YubiKey).
3. `scp` the unpacked bundle to the new host out-of-band (USB, LAN
   transfer, etc.) — or `git clone` from the bundle's embedded git
   bundle file.
4. Run `ops/ansible/<env>-bootstrap.yml` against the new host.
5. Host comes up fresh; generate a new setup key at the existing mesh
   mgmt-plane; enroll new host; Spock-subscribe to a peer site for
   initial sync.
6. Host joins the mesh as a new Spock node.

Value: the bundle removes the "can I reach GitHub and every upstream
registry right now under tactical-time constraints" question from the
provisioning critical path.

### 7.2 Total-loss restart

Every site destroyed or otherwise unreachable. Operator with a laptop
+ YubiKey + cloud credentials at a new location:

1. Pull latest bundle from S3.
2. Verify signature, decrypt, unpack.
3. Terraform + Ansible new infrastructure from scratch (new relays or
   re-use surviving ones, new mgmt-host hardware).
4. Deploy the stack fresh; every peer re-enrolls — surviving devices
   use their pre-distributed offline-enrollment bundles per
   [`offline-enrollment.md`](offline-enrollment.md) to come back onto
   the fresh mesh.
5. Accept that historical mesh state (peers, policies, activity
   logs) is lost. Operational posture restarts fresh.

Expected RTO: ~4–8 h (dominated by hardware provisioning and Spock
initial sync, which on a single-node fresh cluster is effectively
instant). RPO: effectively "everything since the system last stood
up" — the bundle is not a state-restore mechanism.

## 7.3 Pre-baked image companion artifact (Block 68, added 2026-05-06)

The bundle is the long-lived sovereign archive. For tactical-edge bringup
in the 1 h single-Orin / 2 h three-Orin window, a derived **pre-baked
bootable image** companion artifact is built per hardware class from each
signed-tag bundle. Image contents are identical-in-substance to the
bundle (open-source software, IaC, image cache, schema seeds, offline-CA
public cert) — the difference is delivery shape (bootable media that
boots the OS, lays down LUKS, pre-loads Docker images, and runs
`site-bootstrap.yml` against localhost in one pass). A leaked image
reveals nothing operational. Same Ed25519 offline-CA signature; T2
Cloudflare R2 edge-cache mirror serves the image at tactical-edge
bandwidth; T1 S3 + T3 sovereign mirror remain authoritative.

The image is a strict superset of "what the bundle would produce after
operator-driven extraction" — it pre-stages every step of §7.1 so the
critical-path budget shrinks from ~4–8 h to a measured 1 h / 2 h.
Drill discipline (§9 of the rapid-bringup design)
keeps the budget honest.

Full design + procedure + scenario split (S1 total-loss vs S2
Spock-peer-join) at `rapid-site-bringup.md`.
Implementation tracking under Block 68.

## 8. Relationship to other artifacts

| Artifact | What it holds | Storage rule |
|---|---|---|
| **Bootstrap bundle** (this doc) | Architecture + code + image mirror + empty-state seeds. **No identity data.** | Cloud, ciphertext, sovereign age keys, Ed25519 signed. |
| **pgBackRest archives** ([`pgbackrest.md`](pgbackrest.md)) | Full mgmt-plane DB content — identity, credentials, peer state. | Per-site NVMe only. No cloud. Island-mode compliant. |
| **Offline-enrollment bundles** ([`offline-enrollment.md`](offline-enrollment.md)) | Per-device pre-shared WG config + signed ACL capability. Tiny, device-scoped. | Baked into device image; not in either bundle above. |
| **Closed-LAN bootstrap pack** (`nova-lan-instance.md` § Internal CA + DNS; layout at [`ops/bootstrap/nova-lan/pack-layout.md`](../../ops/bootstrap/nova-lan/pack-layout.md)) | `nova-lan-ca` root cert + resolver IP. No mesh enrollment, no images, no identity. | USB-supplied (physical media). No cloud tier — closed network. Sibling to this bundle, distinct shape. |
| **1Password sovereign vault** | Operator YubiKey backups, age last-resort recipient key, pgBackRest cipher passes, infra secrets. | Operator workstations + 1Password cloud (outside project cloud accounts). |

The closed-LAN bootstrap pack is **structurally distinct** from this bundle even though it shares the name. This bundle is cloud-held, encrypted-for-sovereign-keys, and carries the full architecture + image mirror needed to stand up a WAN-connected mgmt-host. The closed-LAN pack carries only the trust-anchor + resolver-IP needed by Tier-T *clients* of an already-running closed-LAN mgmt-host — distributed exclusively via USB, no encryption layer, no image content. See `nova-lan-instance.md` § Internal CA + DNS for the design rationale and [`ops/bootstrap/nova-lan/README.md`](../../ops/bootstrap/nova-lan/README.md) for the mapping of which Block 68 conventions the closed-LAN flow reuses vs diverges from.

Together, pgBackRest + bootstrap bundle + offline-enrollment cover the
full resilience envelope:

- Single-host loss → pgBackRest (local), or paired-host pgBackRest
  restore.
- Single-site full loss → Spock replication + rebuild from peer site.
- Pre-existing corruption → pgBackRest PITR on any local stanza.
- **Every-site loss** → bootstrap bundle (system recovery) +
  offline-enrollment (device re-entry).
- Compromised cloud → everything sovereign stays sovereign; only the
  cloud-held bundle is at risk, and it's ciphertext with no identity
  data.

## 9. Open questions

1. **Prod AWS account provisioning timeline.** Phase 3 logistics row
   currently says "future prod AWS account." Need a concrete
   creation date before the first bundle can be uploaded.
2. **Bundle size ceiling.** Full image mirror could run to several GB
   depending on how many services we include. Budget for ~5–10 GB
   initially; revisit after first build.
3. **Cross-region replication for the S3 object.** Single-region is
   cheaper and simpler. Only add cross-region if a specific scenario
   shows up (e.g., operator is geographically pinned away from the
   bundle's region during tactical-time provisioning).
4. **Bundle test harness.** Every built bundle should be decrypt +
   signature-verify smoke tested in CI against a clean recipient
   before the tag is considered published. Design CI step in a
   follow-up.
5. **Rotation ceremony.** Operator key rotation / revocation is
   hand-waved as "next scheduled rebuild" — needs a concrete runbook
   for the emergency-revocation case.
