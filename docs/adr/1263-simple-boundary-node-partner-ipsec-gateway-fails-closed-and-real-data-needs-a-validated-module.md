# ADR 1263 — A simple boundary node meets a partner's IPsec gateway on the boundary host itself, fails closed, and real data across an organizational boundary needs IPsec on a validated module

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-26 |
| Deciders | operator |
| Repo scope | goat core: `ops/ansible/roles/simple-bn/` (IPsec tasks, templates, filter), `terraform/modules/boundary-host/`, design [`simple-boundary-node.md`](../design/simple-boundary-node.md) §4, §7, §8a; the private overlay records each interconnection |
| Amends | [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (its decision 3, encryption stages) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0103](0103-crypto-boundary-discipline.md) (cryptography at defined interfaces), [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the canonical boundary node), [ADR 1186](1186-credentials-at-rest-are-references-only.md) (keys as references), [ADR 1196](1196-equipment-and-site-identifiers-are-near-secrets.md) (addresses and positions are near-secrets), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) (only released data crosses) |

## TL;DR (for a layperson)

Some partners will only take our data through their own encrypted network link, an IPsec tunnel ending on their
firewall. The simple boundary node now builds that tunnel itself, on the same dedicated machine that sends the data, so
no extra appliance is needed. It is set up the same way every time: either side can start the tunnel, our machine
checks the partner's exact identity, only the partner's one address can reach it, and if the tunnel drops nothing is
sent unprotected. When a partner has no firewall of its own that can end an IPsec tunnel, goat hosts the whole
connection instead: the partner's receiving computer connects to our machine as an IPsec client, with a certificate we
sign and a tunnel address we hand it. Simulated data may use it as soon as it works. Real data may cross only when the encryption runs in a
government-validated cryptographic module and the partner is authenticated by certificate, or an authorizing official has
accepted a shared key in writing.

## Context

ADR 1253 gave the simple boundary node three encryption stages: 0, plain and synthetic only; 1, a WireGuard tunnel for
real data; 2, "the receiver's own protected transport when they mandate one". Stage 2 was one line. It has since been
built for IPsec and run against a live partner gateway, and that work settled questions the one line left open: who
starts the tunnel, how a partner behind NAT with no fixed address is admitted, what is encrypted when the partner's
gateway proposes `0.0.0.0/0`, what happens when the tunnel is down, and how the partner is authenticated.

A second question was left open in the other direction. ADR 1253 sends real data over stage 1. WireGuard's primitives
(ChaCha20-Poly1305, Curve25519, BLAKE2s) are not on the FIPS Approved list. The system's WireGuard waiver covers traffic
inside the mesh and excludes traffic across an organizational boundary, and the crypto posture already says that
cross-boundary traffic terminates on IPsec backed by a validated module. ADR 1253 and that posture disagree for any
receiver outside our own organization.

A boundary node is the system's interconnection with a partner. A security assessor needs to find, for each one, what
crosses it, how it is protected, how the partner is admitted and what happens when it fails. This record states those
answers once, so each interconnection only has to record its own values.

## Decision

1. **The tunnel ends on the boundary host.** Stage 2 for an IPsec gateway is IKEv2 with strongSwan (`charon-systemd`,
   `swanctl`) on the simple-bn host, installed and configured by the `simple-bn` role from `encryption.ipsec` in the
   declaration. No separate VPN appliance or gateway host is added. The host keeps ADR 1253's host rule: dedicated, in the
   goatnet, never a core-services machine.

2. **Initiator or responder, chosen by the partner's addressing.** When the partner's gateway has a fixed public address
   we initiate. When it sits behind NAT with no fixed address we are the responder: we never dial, never start a rekey,
   and hold child SAs to a hard lifetime a little above the partner's ESP lifetime, so the partner always rekeys first and
   an SA it failed to rekey is closed rather than kept. A trap (`start_action = trap`) is never used as a responder.

3. **Route mode when the partner's gateway is route-based, policy mode when it is policy-based.** In route mode the
   tunnel is an XFRM interface (`sbn-xfrm`) on a private point-to-point subnet agreed with the partner. Only what our
   routing puts into that interface is encrypted, and charon installs no routes of its own, so a partner proposing
   `0.0.0.0/0` selectors cannot pull other traffic into the tunnel. In policy mode the selectors are our public /32 and
   the receiver's inside address. Route mode is preferred where the partner can offer it, because what crosses is then
   decided by our routing table, which the role writes, not by the partner's proposal.

4. **Fail closed.** Nothing reaches the receiver in the clear while the tunnel is down. In route mode, datagrams from our
   tunnel address are looked up in their own routing table (4101), which holds the route into the tunnel and an
   unreachable default. In policy mode the receiver's address is unreachable outside strongSwan's table. A send then fails
   and the monitor shows `send-error`.

5. **Two independent admission checks.**
   - **Identity.** The partner's IKE identity is declared exactly (`remote_id`), and with the key or certificate it is what
     admits them. The role refuses a responder without it and refuses a `%` wildcard. The identity is copied from what the
     partner's gateway actually sends, as charon logs it on their first attempt, never from a description of it.
   - **Address.** The boundary host's firewall admits UDP 500 and 4500 only from the partner gateway's public egress
     address /32 (the `boundary-host` module's `ipsec_gateway_cidrs`). A partner that cannot name it in advance may be
     admitted from any address only until its first connection, with synthetic data only; the /32 seen on that connection
     then replaces it.

6. **Proposals.** IKE `aes256gcm16-prfsha384-ecp384`, ESP `aes256gcm16-ecp384` by default: AES-256-GCM, SHA-384 and
   ECDH over P-384. A partner's gateway may need others; they are set in the declaration and must stay on the FIPS
   Approved list.

7. **Authentication.**
   - **Certificates (`auth: pubkey`) are the default for real data.** Our private key is made on the boundary host and
     never leaves it; the partner's CA or certificate is placed beside it.
   - **A pre-shared key (`auth: psk`) is allowed when the partner's gateway requires it.** It is generated, at least 32
     random characters (the role refuses fewer than 16), held in the secret store, given to the partner through a
     one-time secure link, read into the environment only at deploy time, and written only into the root-only connection
     file. It never appears in a repository, a log or a message. A pre-shared key carries real data only after an
     authorizing official has accepted it for that interconnection in the system's plan of action and milestones.

8. **Real data across an organizational boundary uses stage 2 on a validated module.** This amends ADR 1253's decision 3:
   - Stage 0 stays synthetic only.
   - Stage 1 (WireGuard) carries real data only between goatnets of the same sovereign (a pseudo boundary node's
     `tunnel` mode) or where an authorizing official has accepted it for one interconnection in writing.
   - Real data to a receiver outside our organization uses stage 2, with strongSwan taking its cryptography from a
     CMVP-validated module: its `openssl` plugin against a FIPS-validated OpenSSL on the boundary host, recorded with the
     module's certificate number at authorization. A kernel reporting `fips_enabled` is not a certificate.
   - Post-quantum key exchange (ML-KEM) is not on this path. It needs a strongSwan backend that supplies ML-KEM inside a
     validated boundary, which the program has not procured.

9. **Each interconnection is recorded before real data crosses it.** The private overlay holds, per interconnection: the
   declaration and the 1Password item, a runbook following design §8a, an interconnection row in the system security plan,
   its ports in the ports and protocols register, an interconnection security agreement with the partner, and the
   evidence of its first run (our capture decoded with `ops/simple-bn/asterix_inspect.py`, and the partner's confirmation
   of receipt).

10. **Hosted variant: when the partner has no IPsec gateway, goat hosts the connection.** Some receivers have no
    firewall that terminates IPsec, or will not change it for one feed. Then the tunnel runs host to host: our end is
    unchanged (decisions 1 to 6: the boundary host, responder, route mode, fail closed, the firewall rule for their one
    address), and their end is an IPsec client on the receiving host itself, which dials our boundary host.
    - **Authentication is by certificate only.** A pre-shared key is not used in this variant: we write both ends'
      configuration, so no partner policy forces one. The receiving host makes its own key pair and sends a signing
      request; we sign it with a certificate authority used for this one interconnection and no other, whose key is held
      in the secret store and never on a host. Our boundary host's certificate comes from the same authority. The
      receiver's identity is the subject of the certificate we signed (decision 5).
    - **We hand out the tunnel address.** The receiving host gets its tunnel address from our end through the IKEv2
      configuration payload (a strongSwan address pool of one address), so the client needs no network set-up of its own,
      and a stock client works: strongSwan on Linux, the built-in IKEv2 client on Windows. The feeds go to that address and
      the port the receiver listens on.
    - **Revocation is local.** Ending the interconnection, or replacing a lost receiving host, is removing its certificate
      authority (or the one certificate) from our boundary host; the authority signs nothing else, so no other partner is
      touched.
    - **What we give the receiver** is one client sheet: the IKEv2 client to use, our public address and identity, the
      authority's certificate, their signed certificate, the proposals, and the port the feeds arrive on.
    - **The real-data rule is unchanged** (decision 8). The receiver's client is their system under their authorization;
      the interconnection security agreement records which client and cryptographic module it uses.
    - **Status.** A responder in route mode with certificate authentication, against a client that holds a static tunnel
      address, is what the role builds today. The address pool and the per-interconnection certificate authority are
      designed, not built.

## Security controls this provides

For an assessor, mapped to NIST SP 800-53 Rev. 5. The implementation lives in the role and module named in Repo scope;
each interconnection's own values and evidence are in the private overlay (decision 9).

| Control | How this decision meets it |
|---|---|
| AC-4 Information flow enforcement | One direction, one declared receiver, only declared feeds (ADR 1253); route mode encrypts only what our routing puts into the tunnel (decision 3) |
| CA-3 Information exchange | Each interconnection carries an interconnection security agreement and an SSP row before real data (decision 9) |
| CM-6, CM-7 Configuration settings, least functionality | The host runs only the sender, the monitor and the tunnel; every setting is rendered from a declaration and re-applied idempotently |
| IA-3, IA-5 Device identification, authenticator management | Partner admitted by exact IKE identity plus key or certificate; certificates are the default; a pre-shared key is generated, stored and delivered as decision 7 says; in the hosted variant, certificates only, from an authority used for that one interconnection (decision 10) |
| SC-7 Boundary protection | A dedicated boundary host; inbound IKE and NAT-T only from the partner's /32 (decision 5) |
| SC-8, SC-8(1) Transmission confidentiality and integrity | AES-256-GCM over IKEv2 (decision 6); fail closed, so no plaintext fallback (decision 4) |
| SC-12 Key establishment and management | IKEv2 with ECDH P-384; rekeys driven by one side only; private keys made on the host and never moved |
| SC-13 Cryptographic protection | Real data requires a CMVP-validated module under strongSwan (decision 8) |
| SI-4 System monitoring | The boundary monitor shows every feed's sends, refusals and send errors |

## Consequences

- A partner whose policy mandates IPsec can be fed without goat software, without an extra appliance and without a
  routable path from their network into ours: the boundary host accepts IKE only, and the tunnel carries traffic out.
- Each new IPsec partner costs one declaration, one secret-store item, one Terraform list entry and the records in
  decision 9. The method is design §8a; the private overlay keeps a runbook template.
- **Real data over stage 1 to a receiver outside our organization is out of line with this decision** until it moves to
  stage 2 or an authorizing official accepts it. Every such interconnection is listed in the private plan of action and
  milestones with its plan.
- The boundary-host image is stock Ubuntu 24.04. Real data under decision 8 needs FIPS-validated OpenSSL on that host
  (Ubuntu Pro FIPS), which the `boundary-host` module does not enable yet.
- The boundary host's outbound traffic is unrestricted at the cloud firewall; what leaves toward the partner is decided
  by the sender and the routing table, not by an egress rule.
- In the hosted variant goat operates a small certificate authority per interconnection and supports the receiver's
  client configuration. In exchange the receiver needs no firewall change, and the connection uses certificates from the
  first packet.
- Tunnel state (up or down, bytes) is not on the monitor page: the monitor runs unprivileged and reading strongSwan's
  state needs root. The role reports it on every run, and a down tunnel shows as `send-error` on the page.

## Alternatives considered

| Option | Why not |
|---|---|
| A separate VPN appliance or gateway host in front of the boundary node | One more machine to patch and authorize, and the hop between it and the sender would carry plaintext on our side |
| Always initiate | A partner behind NAT with no fixed address cannot be dialed |
| Policy mode only | A route-based gateway proposes `0.0.0.0/0`; as policy selectors that would encrypt, and route, everything the host sends |
| Certificates only, never a pre-shared key | Some partner gateways are configured for pre-shared keys by their own policy; refusing them would block simulated-data integration that carries no information |
| Keep WireGuard stage 1 for real data to any receiver | Its primitives are not FIPS Approved, and the system's waiver excludes cross-boundary traffic |
| Admit IKE from any address and rely on the identity and key | One check instead of two; the /32 costs one Terraform line once the partner's address is known |
| Hosted variant: place a goat-managed appliance on the partner's network as their tunnel end | A goat machine inside another organization's network needs custody, patching and physical access there, and a heavier agreement; a client on their own receiving host needs none of that |
| Hosted variant: a WireGuard client on their host | Its primitives are not FIPS Approved (decision 8) |
