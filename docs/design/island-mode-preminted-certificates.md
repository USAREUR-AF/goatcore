# Island mode: a pre-minted certificate batch for a site that holds no signing key

**Status:** designed, not built. Nothing in this note is in code. It answers one question for
[ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md) (island mode): does
pre-minting fabric certificates fit the portal's slot-pool mode? **Answer: it fits with changes**
(§6). Builds on [`island-mode-multi-writer.md`](island-mode-multi-writer.md) §3 (slot ranges,
designed, not built) and [ADR 1193](../adr/1193-revocations-are-absorbing-on-replicated-mesh-and-identity-tables.md)
(revocations are absorbing; Proposed). Measured against `USAREUR-AF/goatcore` main `144b2e5f`,
2026-11-13.

A site of a goatnet that is cut off must keep admitting people and enrolling machines
([ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md)). Admitting someone to the
fabric means handing them a certificate the routers accept. Some sites should not hold a key that
can sign one. A phone is the clearest case: it is easy to lose, and the key that signs fabric
leaves signs for the whole goatnet. This note designs the alternative the operator chose for such
a site: while connected, the hub mints a fixed batch of certificates for slots the site owns. While
cut off, the site hands out certificates from that batch and signs nothing.

The examples use the synthetic goatnet `zagros`: hub site `zagros-a`, and joiner site `zagros-c`,
a phone.

---

## 1. How slot-pool minting works today

**Plain:** every slot's name is fixed in advance and every router already accepts it. Issuing a
slot means claiming a free row and signing a certificate whose name is that slot. Only the signing
needs the goatnet's CA key.

| Step | What happens | Where |
|---|---|---|
| Pool | `InitializeSlotPool` inserts `zenoh_slot_pool_size` rows once; each slot id (the anchor) is `sha256(salt:k)[:32]` | `ops/goat-zenoh-portal/internal/registry/slots.go:75` |
| Router | Every router pre-renders every anchor of the pool into its `cert_common_names` subjects, so a new tenant needs no ACL edit and no restart | `ops/ansible/roles/goat-zenoh/templates/config.json5.j2:1566` |
| Claim | `allocateSlot` takes the lowest free `idx` and clears every earlier stamp on the row, the certificate stamps included | `slots.go:227` |
| Sign | `certmint.Minter.Mint` runs `cert-sign` with the goatnet CA key (`<ca-bundle-dir>/ca.key.age`, decrypted by the portal's age identity). The leaf's CN is the anchor; the key is ECDSA P-256 so the leaf can also be a grant-crypto recipient | `ops/goat-zenoh-portal/internal/certmint/certmint.go:344`, `:368` |
| Lifetime | One year. `Minter.Validity` defaults to 365 days and no flag sets it | `certmint.go:173` |
| Profile | `bundle-create` signs a goat-cli profile (`.cbor`) with the same CA key. It carries the tenant's portal API key, the portal URL and the router endpoint | `certmint.go:424` |
| Callers | People: `POST /api/orgs` and collection (`ops/goat-zenoh-portal/internal/api/collect.go:178`). Machines: enrolment, with `serverAuth` added, because the pod router also listens with the leaf (`ops/goat-zenoh-portal/internal/api/machine_enroll.go:302`) | |

What the routers check is the CN, and nothing else: there is no URI-SAN matcher. Zenoh has no CRL
or OCSP ([ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md), "no CRL/OCSP"). A leaf
therefore works at every router of the goatnet until it expires. Cutting a tenant off before that
happens at the mesh (NetBird) or at the identity provider (Kratos), or by re-rendering the routers
without that anchor and restarting them.

The pool has been batch pre-minted once before, on one goatnet. The stamps that pass left behind
broke collection (F-313), which is why `allocateSlot` now clears certificate stamps on every claim.
Pre-minting is not new to the pool. What is new here is using the batch while cut off.

---

## 2. What a pre-minted batch holds

**Plain:** a batch is a set of finished credentials for the slots the site owns, made by the hub.
A credential stays unused until the site assigns its slot to someone.

The batch is minted by the hub (ADR 1205's highest-echelon site) while it is connected to the site,
and only for slots in the site's own range (`slot_ranges(site, lo, hi)`, multi-writer §3.1). One
entry per slot:

| Field | Purpose |
|---|---|
| `slot_id`, `idx` | The anchor, inside the site's range |
| `kind` | `person` (clientAuth) or `machine` (clientAuth + serverAuth). These are separate, because the two leaves differ and a person leaf must not be able to listen |
| leaf certificate, private key | Signed by the goatnet CA with CN = anchor; a short validity (§3) |
| API key and `.cbor` | Pre-generated portal API key, and the goat-cli profile signed for it. The `vendors` row and its key hash are written only when the slot is assigned, so until then the key authenticates to nothing (`resolvePrincipal` finds no row) |
| `batch_id`, `not_after`, `minted_at` | Which batch, and when it stops being usable |

Storage:

- A site-local table, never replicated: the private keys and API keys are secrets
  (multi-writer §3.2 already keeps secret columns local). The batch is encrypted at rest with the
  site's own key, not stored in the clear.
- Not in `slots.cert_minted_at`. The claim clears that column (`slots.go:227`), and pool-era stamps
  there already stranded a cohort once (F-313). The slot row learns of the batch only when it is
  assigned.
- The public leaf of each entry may go into the recipient-cert store when it is assigned, as `Mint`
  does today (`persistRecipientCert`).

Size: enough for the expected admissions in the longest partition the site plans for, and no more.
Each unused entry is exposure (§5). The hub tops the batch up while connected.

---

## 3. Lifetime and revocation

**Plain:** a batch certificate is short-lived and is replaced after the site reconnects. Taking a
batch back is a revocation that no merge can undo, and it costs a router restart.

- **Validity.** Each leaf's `notAfter` is set when the batch is minted. It counts from the mint, not
  from the issue, so it must cover the partition the site plans for plus a margin. This needs a
  validity setting per mint (`Minter.Validity` has none, §1). After heal, the hub re-issues each
  batch-issued tenant a normal leaf in place ([ADR 1044](../adr/1044-credential-rotation-in-place-reissue.md)).
  A batch-issued leaf is therefore never the long-term credential.
- **Revoking a batch** (a lost phone, or a site the hub severs under ADR 1205) is a class 3 write:
  a ledger row of a new kind, `portal.slot_batch`, keyed on the `batch_id` and listing its anchors.
  As in ADR 1193, the ledger only grows, and "revoked if any site revoked" decides without a clock.
  Any assignment the site made from that batch, arriving after heal, is revoked by the ledger like
  any other: the tenant's NetBird peer and Kratos identity are cut through the existing subject
  revocation (inventory X5).
- **Denying the anchors at the routers.** The ledger alone does not stop a leaf, because the routers
  have no CRL. The router render needs a deny list of anchors, which the template does not have
  today. It filters those anchors out of every slot subject. Applying it is a planned restart of
  each router, as for any `access_control` change (ADR 1025). Until that restart, a thief who
  can reach a router over the mesh can use a stolen leaf.
- **Reuse.** A batch slot, used or not, is not reused while any leaf for it can still be valid. This
  is the inventory's X2 rule (an anchor is the certificate's identity) extended to unused entries:
  the slot returns to the pool only after `notAfter`, or after the anchor is on the router deny
  list.

---

## 4. What a cut-off site can and cannot do with it

| Can | Cannot |
|---|---|
| Admit a person or enrol a machine to the fabric, from its own range, while cut off: claim the slot (class 2, owner = the site) and hand over the batch entry's leaf, key and `.cbor` | Issue anything once the batch is empty, or for a slot outside its range |
| Record the assignment, the identity link and the custody, as class 2 writes that replicate on heal | Choose a certificate's lifetime, add a SAN, or issue a leaf of the other kind |
| Create the tenant's portal row with the pre-generated API key's hash, so the tenant can use that site's portal | Re-issue or rotate a leaf (that needs the CA key); a lost credential waits for heal |
| Revoke what it issued: a class 3 ledger row, and the mesh and identity cuts it can make locally | Remove an anchor from any router. That is a hub render and a restart |
| Run its own workloads, if it has its own SPIRE trust domain ([ADR 1164](../adr/1164-per-site-spire-trust-domains.md)) | Get workload SVIDs from a batch. A batch holds fabric leaves only; SPIRE is a separate issuer |

The tenant also needs a NetBird setup key and a Kratos identity, which the site creates as class 1
writes (multi-writer §3). The batch covers only the fabric certificate.

---

## 5. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| A stolen batch (lost phone) | Every unused entry is a working fabric identity, at any router, until its `notAfter` or until the routers are restarted with those anchors denied. The thief still needs a path to a router over the mesh | Short validity; a small batch; the batch encrypted at rest; batch revocation (§3) with the router deny list; revoking the phone's own mesh peer. API keys in the batch are inert until assigned |
| Overlap on heal: two sites assign the same anchor | One certificate identity held by two tenants | Not possible while ranges are disjoint and a batch covers only its owner's range. The hub mints batches only for the range's owner, and claims filter on `owner_site` (multi-writer §3.1) |
| Overlap on heal: the hub took over the range while the site was cut off (ADR 1205) | The hub could re-assign anchors the site has already issued from its batch | A takeover burns every unused entry of that site's batches: none of their slots is reused until `notAfter` (§3, Reuse) |
| The site assigned from a batch the hub revoked during the partition | People admitted at the site are cut off after heal | Intended: the revocation is absorbing. The site's admin sees the cut in the resolution queue (multi-writer §4) |
| A batch leaf outlives the partition | A long-lived identity issued without the hub | Re-issue in place after heal (§3), and short validity as the backstop |
| The batch state is mixed with pool stamps | Collection refuses its owner (F-313) | A separate table (§2) |

---

## 6. Recommendation: fits, with changes

The slot pool already has what pre-minting needs. The anchors are fixed in advance, and every
router admits them without a restart. A certificate is just the anchor plus a signature, so it can
be made before anyone owns the slot. The pool has been pre-minted before. The changes:

| # | Change | Status |
|---|---|---|
| 1 | Per-site slot ranges and `owner_site` claims (multi-writer §3.1, step B) | designed, not built. Prerequisite |
| 2 | A site-local, encrypted batch table, separate from `slots` (§2) | designed, not built |
| 3 | A batch source in `certmint`. With no CA material at the site, `Mint` takes the batch entry for the claimed slot instead of running `cert-sign` and `bundle-create`. The claim picks only slots with an entry of the right kind | designed, not built |
| 4 | A validity per mint, and a hub command that mints and tops up a site's batch | designed, not built |
| 5 | Ledger kind `portal.slot_batch`, and reuse held until `notAfter` | designed, not built |
| 6 | A deny list of anchors in the router render | designed, not built |
| 7 | Re-issue of batch-issued leaves after heal (ADR 1044 path) | designed, not built |

It does not need any change to the router's matching, the anchor derivation, the CA or the key type.

This answers the operator's choice for phones. A fixed site's own signing key, subordinate to the
goatnet CA, is a separate design. The routers keep only the CN, so a site-issued name would have to
carry the site, and no way exists to withdraw a subordinate CA from routers that have no CRL.
