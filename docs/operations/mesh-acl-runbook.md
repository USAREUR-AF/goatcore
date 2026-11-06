# Mesh ACL — operational runbook

Operational procedures for the mesh ACL group taxonomy defined in
`docs/design/mesh-acl.md`. Use this when
enrolling a new peer, adding a new peer class, editing a rule, or
reviewing GUEST-group entries.

**Anchors.**
mesh-acl design doc ·
[ADR 0807 microsegmentation](../adr/0807-mesh-microsegmentation.md) ·
[setup-key rotation](setup-key-rotation.md) ·
F-035 default policy seed gap.

---

## Pre-flight: confirm the site has the day-1 floor

Before any peer enrollment, the site must have:

1. The 13 groups instantiated (12 role + GUEST + the implicit default
   `<SITE>`). Verify in the NetBird dashboard or via `netbird api groups list`.
2. Default-deny enabled at the policy root. Confirm "members of the
   same group can talk" is **off**.
3. The v1 day-1 matrix from
   `docs/design/mesh-acl.md`
   applied as ACL rules.
4. A test peer in `<SITE>-OBSERVER` can reach `isitup.<...>` but not
   the postgres VIP.

If the default policy + rule are missing entirely (peers register but
show `Peers count: 0/0 Connected`), see F-035
for the SQL workaround until the `mgmt-host-bootstrap --with-default-policy`
flag lands.

---

## Procedure A — enroll a peer into an existing role group

The most common case. Assumes the peer class already has a role group.

1. **Pick the role group** by peer class:
   - Mgmt-plane host → `<SITE>-MGMT`
   - Hosted-service peer → `<SITE>-SERVICES`
   - Operator laptop (read-only) → `<SITE>-OBSERVER`
   - Sensor → `<SITE>-SENSOR`
   - Off-site peer (rear ops, sponsor) → `<SITE>-OFFSITE`
   - Zenoh router → `<SITE>-ZENOH`
   - Snitch probe → `<SITE>-SNITCH`
   - **Doesn't fit** → Procedure B (GUEST + triage).
2. **Issue a role-scoped setup key** per
   [setup-key-rotation.md](setup-key-rotation.md). The setup key
   pre-assigns the peer to the role group on first contact, so the
   peer is *never* in the default group alone (which has no role-group
   ACLs and would dead-end).
3. **Enroll the peer.** Bootstrap-bundle path (production) or
   `netbird up --management-url ... --setup-key ...` (dev).
4. **Verify.** From the peer:
   - `netbird status` shows the role group in the assigned-groups list.
   - The peer can reach the destinations the v1 matrix allows for its
     role group, and only those.

---

## Procedure B — enroll a peer that doesn't fit (GUEST)

When a peer class is novel — first one-off of a class you haven't seen
before — enroll into GUEST. Forces a triage decision within 7 days.

1. **Issue a GUEST-scoped setup key** with the same role-key TTL bucket
   as the peer's expected lifetime (operator 30d / peer 7d). Setup key
   pre-assigns `<SITE>-GUEST` only.
2. **Enroll** as in Procedure A step 3.
3. **Note the candidate class** in
   `docs/project/active-work.yaml` under
   the `mesh-acl` track or as a new track if material:

   ```
   notes: |
     GUEST peer enrolled <date>: <hostname>, class candidate "<short-name>".
     Triage by <date+7d>.
   ```

4. **Triage within 7 days.** One of:
   - Fits an existing role group → reclass per Procedure D, remove from GUEST.
   - Genuinely novel class but only the first peer → keep in GUEST, log
     in active-work for the next cycle.
   - Third peer of the same novel class arrives → Procedure C.

---

## Procedure C — carve out a new role group (rule of three)

Triggered when the third peer of a novel class enrolls.

1. **Name the group.** Convention: `<SITE>-<ROLE>` where `<ROLE>` is the
   shortest unambiguous noun describing the peer class. Avoid mission
   names; the NetBird layer is echelon-based,
   not mission-based — see
   mesh-acl design doc §"Why no AIRDEFENDERS-C2".

2. **Update the design doc** (`docs/design/mesh-acl.md`):
   - Add the new group to the §"Group taxonomy (13 per site)" table.
   - Add a row + column to the §"v1 policy matrix" — copy the closest
     existing row as a starting point and adjust.
   - If the new group is a 14th role, update the TL;DR group count.
   The design-doc update lands in the same change as the API edits.

3. **Create the group via NetBird API.**

   ```
   netbird api groups create --name "<SITE>-<ROLE>"
   ```

4. **Add the matrix rules** for the new group via
   `netbird api access-control` — one rule per non-empty matrix cell.

5. **Reclass the existing GUEST peers** of this class into the new role
   group (Procedure D). Remove them from GUEST.

6. **Update CLAUDE.md** if the new group changes how the system is
   described at the architecture level (most carve-outs won't need this;
   they're just new role buckets).

7. **Commit + open PR.** Reviewers verify the design doc + ACL match
   and that the v1 matrix stays under the 100-rule budget
   (mesh-acl design doc §TL;DR).

---

## Procedure D — reclass a peer from one group to another

```
netbird api groups remove-peer --group "<SITE>-OLD-GROUP" --peer <peer-id>
netbird api groups add-peer    --group "<SITE>-NEW-GROUP" --peer <peer-id>
```

Verify the peer's `netbird status` reflects the new group within ~30 s.
The peer's existing WireGuard tunnels are not torn down by the reclass;
ACL evaluation re-runs on the next network-map push (~30 s).

---

## Procedure E — edit an ACL rule

**Today (pre Block 17A.5 policy engine):** the operator who owns the
change makes it directly via the NetBird API. Capture the change in
the design-doc matrix in the same PR.

```
netbird api access-control rules list
netbird api access-control rules update <rule-id> ...
```

**v2 (when [Block 17A.5 policy engine](../adr/0805-policy-engine-active-passive.md)
ships):** all ACL changes flow through the dual-path approval workflow
(normal: 1 role-holder + 24h SLA; fast: 2-of-N peer + immediate). The
reconciler pushes the approved delta to the NetBird API. Direct API
edits become a break-glass path that requires after-the-fact audit.

---

## Procedure F — weekly GUEST review

Once per week, an operator reviews every peer currently in
`<SITE>-GUEST`:

1. List GUEST peers:

   ```
   netbird api groups list-peers --group "<SITE>-GUEST"
   ```

2. For each peer, check `picked_up_at` against today. SLA breach (>7
   days in GUEST) is logged in active-work and a triage decision must
   be made.

3. Outcomes:
   - Fits an existing role group → Procedure D.
   - First / second peer of a novel class → leave in GUEST, refresh the
     active-work note.
   - Third peer of a novel class → Procedure C.
   - No longer needed → revoke setup key + remove peer.

When [Block 38 chaos probes](../design/continuous-architectural-chaos.md)
ship, this becomes an automated finding draft if any GUEST peer crosses
the 7-day SLA. The weekly manual review remains until the probe is
trusted.

---

## Procedure G — site stand-up (apply the v1 floor to a new site)

When a new site comes online:

1. **Pick a site code** — short uppercase identifier.
   Add it to the design-doc §"Site-prefix convention" table.
2. **Instantiate the 13 groups** under the new prefix via NetBird API.
3. **Apply the v1 matrix** — same shape, new prefix.
4. **Verify the day-1 floor** per the pre-flight checklist above.
5. **First peer of the site** is always the mgmt-plane host pair into
   `<SITE>-MGMT`. Then enroll the remaining peers in role order.

---

## Common gotchas

- **Setup key without role-group assignment.** If the setup key only
  carries the default `<SITE>` group and not the role group, the peer
  enrolls but reaches nothing (default group has no allow rules to
  anything except isitup baseline). Always issue role-scoped setup
  keys.
- **Bootstrap key in production.** The bootstrap key (1d TTL, used for
  the first mgmt-plane peer) is consumed once per site and revoked
  within 24h. Do not reuse for normal peers.
- **`Peers count: 0/0 Connected` after enroll.** Usually
  F-035
  — the default policy + rule weren't seeded. Apply the SQL workaround
  in F-035; the `mgmt-host-bootstrap --with-default-policy` flag will
  close this when it lands.
- **Forgetting the design-doc update on a new group carve-out.** The
  CI gate for Block 18A step 4
  will catch this when it ships; today it's a reviewer responsibility.
- **Mission-name groups.** Push
  back. The NetBird layer is echelon-based, not mission-based — mission
  slicing rides on app-layer authz (Ory claims + Zenoh key-expressions).
  See mesh-acl design doc §v2 differentiation.

---

## Cross-references

- `docs/design/mesh-acl.md` — taxonomy, matrix, lifecycle.
- [setup-key-rotation.md](setup-key-rotation.md) — role-scoped key TTLs.
- F-035 — default policy seed gap.
- [ADR 0805 policy engine](../adr/0805-policy-engine-active-passive.md) — v2 change-flow substrate.
