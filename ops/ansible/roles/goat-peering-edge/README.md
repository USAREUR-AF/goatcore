# goat-peering-edge

One strongSwan IPsec tunnel between two edge hosts belonging to different
sovereigns. Neither host joins the other's mesh; the tunnel carries this edge's
fabric port talking to the far edge's fabric port, and nothing else.

Design: `cross-sovereign-peering-transport.md`.
Names: [`federation-package-spec.md`](../../../../docs/design/federation-package-spec.md) §3.1.
FIPS: `fips-pqc-path.md` §"What the boundary node runs".

This role **configures**. It does not mint certificates, does not write secrets,
and does not open firewall ports.

## What it does

1. Refuses unless the peering declaration is complete, and checks the two
   peering identities against their derivation from the two `sovereign_id`
   values — a mistyped name matches no rule and is refused in silence at both
   ends, which reads as a broken link and costs days.
2. Installs `strongswan-swanctl` + `charon-systemd`, and stops the legacy
   `strongswan-starter` daemon if the distro pulled it in.
3. Observes what the host actually provides: `/proc/sys/crypto/fips_enabled`,
   the OpenSSL provider list, the strongSwan version, Ubuntu Pro attach state.
4. Writes the state file, **then** applies the FIPS gate below. The evidence is
   on the host whether the run proceeds or refuses.
5. Asserts the leaf certificate, its key, and the partner CA for this peering
   are already in place.
6. Creates an xfrm interface (`ipsec-<name>`) carrying the link subnet at a
   declared MTU, and renders the swanctl connection bound to it by `if_id`.

## The FIPS posture, and why it is conditional

**The strongSwan configuration is identical on a validated host and an
unvalidated one.** The suite is AES-256-GCM / SHA-384 / ECDSA P-384 with
certificate authentication either way. What differs is only whether the role
lets the run proceed, and what claim it records.

Three inputs decide it:

| | |
|---|---|
| `peering_edge_fips_required` | default **true** |
| observed | kernel `fips_enabled=1` **and** an OpenSSL FIPS provider loaded |
| claimed | observed **and** `peering_edge_cmvp_certificate` is non-empty |

`cmvp_certificate` is declared by an operator who read the number off the
public database at csrc.nist.gov. `fips-pqc-path.md` is explicit about this:
verify the certificate, do not trust a kernel that reports itself enabled. So
**an empty certificate is not a weaker claim, it is no claim** — a host showing
`fips_enabled=1` with nothing recorded still needs a waiver, and that is the
intended behaviour, not an oversight.

Four outcomes, and no fifth:

| required | claimed | waived reason | Outcome |
|---|---|---|---|
| true | yes | — | `proceed-claimed` |
| true | no | set | `proceed-waived` — configures, prints a deviation banner, records the reason in the state file |
| true | no | empty | **`refuse`** — fails, having written the observations first |
| false | — | — | `proceed-no-claim` — configures, records that no claim is made |

The refusal message distinguishes the two shapes it can have: validated crypto
running with no certificate recorded, versus a platform that cannot run
validated crypto at all. On an L4T host the second is expected and unfixable —
Canonical's FIPS kernel and NVIDIA's tegra kernel are mutually exclusive — so
that host runs this edge under a recorded waiver or not at all.

There is no post-quantum transform in the suite. `ke1_mlkem768` needs
strongSwan 6.x and a validated backend that is not available here, and the
design deliberately does not pursue PQC at the tunnel: the Zenoh TLS session
inside already negotiates the X25519MLKEM768 hybrid end to end, so PQC on the
outer layer would protect only the outer layer. The state file records
`post_quantum_kex: false` so the absence is a fact rather than an oversight.

## MTU

`peering_edge_mtu` defaults to **1380**, deliberately the same number the
WireGuard transport uses, so two teams agree one value regardless of which
transport the declaration selects.

Worst-case headroom on a 1500-byte physical path: 20 outer IPv4 + 8 UDP
(NAT-T) + 8 ESP header + 8 GCM IV + 16 ICV + up to 3 pad + 2 trailer leaves
about 1435. A partner behind PPPoE (1492) leaves about 1427. 1380 clears both
with margin.

**Raise it only against a measured path.** The cloud drill measured 8921
between two AWS regions, which is an AWS backbone number and does not
generalise to a partner on other infrastructure. Getting this wrong makes large
samples fragment or vanish while small ones work perfectly — the worst failure
shape, because the link tests fine.

**Interaction with WireGuard, which already runs on these hosts.** The ESP
packets leave on the physical interface, not through `wt0`; the peering
interface is a third interface and is on neither mesh. Two things to check
before applying:

- the partner's public address must not fall inside a NetBird network route,
  or the ESP would be sent into the mesh and the link would silently take a
  second layer of encapsulation it has no MTU budget for;
- `ipsec-<name>` must not be enrolled, advertised, or routed by NetBird.

MSS clamping is off by default. The edge terminates the fabric session itself,
so the traffic is locally originated and the kernel already derives MSS from
the interface MTU. Turn `peering_edge_clamp_mss` on the day something behind
this host routes across the link.

## Variables

Full list with comments in [`defaults/main.yml`](defaults/main.yml). The ones
without a usable default, which the role refuses without:

| Variable | What |
|---|---|
| `peering_edge_enabled` | off by default; nothing runs without it |
| `peering_edge_name` | short name for this peering, ≤ 11 chars (`ipsec-<name>` must fit `IFNAMSIZ`) |
| `peering_edge_our_sovereign_id` / `_their_sovereign_id` | lowercase ISO 3166-1 alpha-3, or a body's short name |
| `peering_edge_local_id` / `_remote_id` | `fed.<issuer>.<counterparty>.<ordinal>`; checked against the derivation |
| `peering_edge_local_addr` / `_remote_addr` | endpoints from bootstrap step 1 |
| `peering_edge_local_link_ip` / `_remote_link_ip` | the two addresses of the agreed /31 |

Worth knowing about the rest:

| Variable | Default | Note |
|---|---|---|
| `peering_edge_fips_required` | `true` | see above |
| `peering_edge_fips_waived_reason` | `""` | the only way past a refusal |
| `peering_edge_cmvp_certificate` | `""` | empty = no claim |
| `peering_edge_mtu` | `1380` | see above |
| `peering_edge_ike_proposals` | `aes256gcm16-prfsha384-ecp384` | |
| `peering_edge_esp_proposals` | `aes256gcm16-ecp384` | |
| `peering_edge_start_action` | `start` | set `trap` on a side behind NAT that must never initiate |
| `peering_edge_xfrm_if_id` | `1001` | local to this host; unique per peering, not agreed with the partner |
| `peering_edge_local_ts` / `_remote_ts` | the two `/32`s | see the note below on port scoping |

## Rotating the leaf (K6)

Off unless `peering_edge_rotation` names a rotation id
([federation-key-rotation.md](../../../../docs/design/federation-key-rotation.md) §8). The role still writes no
secret. The offline ceremony issues the new material and an operator stages it beside the live files:
`<name>-cert.next.pem` in `x509/`, `<name>-key.next.pem` in `private/` at mode 0600, and for an `ipsec-ca`
rotation our new CA as `<name>-local-ca.next.pem` in `x509ca/`. The role then:

1. checks what is staged before anything moves: the key is the certificate's, it chains to
   `peering_edge_local_ca_path` (or to the staged new CA for `ca`), its CN is `peering_edge_local_id`, its key
   is ECDSA P-384, and it is valid now and for `peering_edge_min_valid_days` more days (GB-289 when something
   is missing, GB-290 when something is wrong);
2. for `ca`, refuses unless `peering_edge_rotation_ca_readback` is the new CA's sha256 as the partner read it
   back (GB-303);
3. copies the live pair into `.previous-<id>/` (0700), once per id, and moves the staged pair over it;
4. runs `swanctl --load-creds` and `swanctl --rekey --ike <name> --reauth`, and waits up to
   `peering_edge_rotation_confirm_seconds` (the DPD timeout) for an established SA with an installed CHILD SA;
5. if none comes up, copies the old pair back, loads it, re-authenticates and stops (GB-302).

A rerun with the same id moves nothing, so a killed run finishes. `peering_edge_rotation_retire: true`, after
both sides pass phase 14, removes the set-aside key and keeps its certificate. During a partner's own CA
rotation, `peering_edge_remote_ca_next_path` makes `remote.cacerts` accept their new CA beside the current one.
`tests/test-rotation.sh` covers each step with a stand-in `swanctl` and certificates made at test time.

## Before you apply this to a host

Nothing below is done by the role.

- [ ] **Bootstrap steps 0 and 1 are complete.** Intent and trust level agreed;
      endpoints, link subnet and MTU exchanged out of band. Step 1 carries no
      trust material, so a failure at step 2 costs only time.
- [ ] **The host is the one the design targets, or the deviation is recorded.**
      An L4T host cannot run validated crypto and will refuse without
      `peering_edge_fips_waived_reason`.
- [ ] **The CMVP certificate number is read off csrc.nist.gov** for the module
      this host actually runs, and set in `peering_edge_cmvp_certificate`. Do
      not copy a number from another host's state file.
- [ ] **Leaf certificate and key are in place**, minted by the offline-CA
      ceremony with Subject CN = `peering_edge_local_id`, ECDSA P-384. The key
      must be **unencrypted**: `swanctl --load-all` runs from a handler with no
      terminal, and a passphrase-protected key makes it hang rather than fail.
- [ ] **The partner CA is installed for this peering only**, at
      `peering_edge_remote_ca_path`. It must not reach a core router's trust
      store: the access layer matches the common name and cannot see the
      issuer, so a foreign root in a core router *is* authorization.
- [ ] **The partner's public address is not inside a NetBird route** (see MTU,
      above), and `ipsec-<name>` is not enrolled in the mesh.
- [ ] **Firewall rules for bootstrap step 2 are written.** The role's traffic
      selectors are the two link addresses; they do not restrict the port. The
      design's step-2 assertion — that the only port reachable across the link
      is the far edge's fabric port — is a host firewall rule, and this role
      does not write one. Narrowing the traffic selectors to a port instead
      would make the two independently-built IKE stacks narrow differently,
      which fails as a silent no-match.
- [ ] **The cut procedure and contacts are agreed** (bootstrap step 7). The cut
      itself is `swanctl --terminate --ike <name>` plus
      `systemctl stop goat-peering-link@<name>` — local, immediate, unilateral.

## State file

`/var/log/goat-peering-edge-<name>-state.json`, sibling of
`/var/log/ubuntu-pro-fips-state.json` and collected the same way. It records
what was observed, not what was hoped for:

```json
{
  "timestamp": "2026-09-05T10:00:00Z",
  "hostname": "mgmt-zagros-01-thor",
  "peering": "xaa",
  "platform": "l4t-arm64",
  "kernel": "6.8.12-tegra",
  "decision": "proceed-waived",
  "identity": { "local_id": "fed.zagros.xaa.01", "remote_id": "fed.xaa.zagros.01" },
  "transport": { "link_ifname": "ipsec-xaa", "mtu": "1380", "fabric_port": "7447" },
  "negotiated_suite": {
    "ike_proposals": "aes256gcm16-prfsha384-ecp384",
    "esp_proposals": "aes256gcm16-ecp384",
    "post_quantum_kex": false
  },
  "observed": {
    "kernel_fips_enabled": false,
    "openssl_fips_provider": false,
    "strongswan_version": "strongSwan 5.9.13",
    "ubuntu_pro_attached": false
  },
  "fips": {
    "required": true, "observed": false, "claimed": false,
    "cmvp_certificate": "", "waived_reason": "example: platform cannot run validated crypto"
  }
}
```

`negotiated_suite` is the **configured** suite, not a reading of a live SA. S2
of the roadmap wants the established SA confirmed; that is
`swanctl --list-sas` at apply time, and it is not something this role can
assert on a host it has not yet reached.
