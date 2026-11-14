# Phones and tablets on a goatnet

| | |
|---|---|
| Status | **Designed 2026-09-10. Built the same day, not deployed: the guide's phone section (§2.3), the handset key end to end (§2.2, §2.6). Measured: M-116, iOS resolves mesh names. Not built: the handset DNS group (§2.4, conditional on Android).** Plan and phase gates: `mobile-and-help-desk-plan.md`. Decision record: ADR 1175. |
| Reader | The person adding the phone section to the welcome guide; the platform admin who mints keys; anyone asked "can I put my phone on the goatnet". |
| Related | [three-stage-candidate-onboarding.md](three-stage-candidate-onboarding.md) · machine-enrollment.md · [peer-identity-resolution.md](peer-identity-resolution.md) · dns-service-discovery.md · [goat-client.md](goat-client.md) · help-desk-service-pattern.md · track `netbird-groups-governance` (PR #991) |

A phone or tablet joins a goatnet with the NetBird app from the App Store or Play Store and a setup key, the same way a laptop does with the command-line client. It is a person's device: it holds one person role, it is another peer on that person's identity, and it is not a machine in the sense of ADR 1096. Three things are missing today and this document adds them: the live welcome guide says nothing about phones, a phone enrolled on the person's only key inherits whatever substrate groups the person holds, and nobody has measured whether a phone resolves mesh service names.

## 1. What is true today

**The client is the stock NetBird app, and it works against the fork.** The fork is seven patches under `ops/netbird/patches/`. The only wire-protocol delta is one additive protobuf field on `RelayConfig` (`peer-id-bound-relay-tokens.patch`), which a stock client ignores, and the relay dual-accepts both token forms by default (`docs/design/relay-architecture.md:69`). Setup-key enrolment needs no NetBird user and no OIDC (`docs/design/netbird-feature-usage.md:47`). The management name of each goatnet is public through the relays with a publicly trusted certificate (`docs/design/relay-ingress-dispatch.md:147-152`), which matters because a store app cannot be handed a private trust anchor.

**The live welcome guide has no phone section.** `ops/goat-id-console/internal/welcome/welcome.go:690-724` emits install and enrol lines for macOS, Linux and Windows only. The only written phone procedure in the repo is `ops/ansible/templates/operator-onboard/bundle.md.j2:40-50`, which belongs to the enroll leg that has been fail-closed since July 2026 (`ops/ansible/operator-onboard.yml:460`). The guide the person actually receives cannot tell them how to put a phone on.

**A phone on the person's only key gets the person's groups.** The key minted at onboarding carries the manifest's `netbird.auto_groups` and a reuse count `netbird.usage_limit` (default 5, `ops/goat-id-console/web/static/admin/onboard-intake-form.json:99`). A second device on that key is a peer with identical groups. For an OBSERVER that is fine. For a PLATFORM-ADMIN it puts a handset in the substrate-authority set that D5 alarms on and that the group-management runbook forbids ("A person's phone or tablet is a person device and takes a person role, usually OBSERVER", `docs/operations/netbird-group-management.md:28`).

**Mesh names resolve on an iPhone; Android is unmeasured.** Measured 2026-09-10: the operator's iPhone on iOS 26.6.1 with NetBird 0.75.0 opened `isitup.zagros.netbird.zagros.example` with nothing but the pushed group below, so the iOS client applies match-domain nameservers despite the vendor's stated limit. Android has not been tried. Each goatnet pushes one NetBird nameserver group to every peer, pointing at the site's CoreDNS replicas for the mesh zone only: `domains: [<zone>]`, `primary: false` (`ops/ansible/goat-coredns.yml:238-256`). NetBird's DNS documentation states that nameservers with match domains are supported only on macOS, Windows and Linux with systemd-resolved, and that Android additionally needs Private DNS switched off. Before M-116 no measurement in the repo recorded a phone opening a `*.netbird.<zone>` name. The name-fallback resolver (`name-fallback-resolver.md` §9) has no handset tier because it needs `goat-clientd` running as root.

**goat-client does not ship for phones.** The iOS and Android shells are in-tree and never released, blocked on store signing (`docs/design/goat-client.md:11,258`). A phone cannot run goat-client and the stock NetBird app at the same time. So for now the handset client is the stock app, and the day goat-client mobile ships, the guide's install step changes and nothing else in this design does.

## 2. Decisions

### 2.1 A handset is a person device on a person role, never a machine

A phone is another peer under the person's own identity, on a person role, OBSERVER unless the sponsor says otherwise. It gets no slot, no Zenoh certificate, no node class and no custody edge, because those belong to system principals minted through machine enrolment (ADR 1096), and a phone that browses services is not one. A handset never holds an infra tier, a node class or P2P; the group-management runbook's decision procedure (`docs/operations/netbird-group-management.md:92-100`) stops at step 1 for it.

### 2.2 Each person gets a separate handset key

The onboarding runner mints two setup keys per person instead of one:

| key | groups | reuse | peer name | who uses it |
|---|---|---|---|---|
| device key (exists today) | the manifest's `netbird.auto_groups` | `netbird.usage_limit`, default 5 | `peer-<handle>[-<org>]-<site>-<date>-<hostname>` | laptops and workstations, command-line client |
| handset key (new) | `<GOATNET>-OBSERVER` only | `netbird.handset_limit`, default 1, clamped 0–5 | same prefix, the app appends the device name | phones and tablets, NetBird app |

The manifest carries the new field beside the existing one:

```yaml
netbird:
  auto_groups: [PLATFORM-ADMIN, DEVELOPER]
  usage_limit: 3
  handset_limit: 1        # 0 = no handset key minted
```

The intake form asks "phones or tablets you will put on the goatnet (0–5)" beside the existing "setup-key reuse" question. Both are capacity, not authority (`machine-enrollment.md:333-348` draws that line for `usage_limit`; the handset key never grants more than OBSERVER, so a candidate cannot self-grant anything by answering it). The sponsor can raise the handset role to DEVELOPER at approval for someone who will run a fabric client on a tablet; nothing above DEVELOPER is offered.

Both keys are minted by the same `goat admin onboard --native` run (`crates/goat-cli/src/cmd/admin.rs`, `mint_handset_key`; NetBird key name `<key_name>-handset`, `auto_peer_name` the unsuffixed `key_name`), share the person's TTL, and are revoked together by the existing revoke sweep, which matches keys by the peer-name prefix. The runner scrapes the printed `handset key:` line beside `setup key:`, the console stores it in `onboard_proposals.handset_key`, and the welcome guide prints both, each labelled with what it is for. Built 2026-09-10, not deployed.

### 2.3 The welcome guide gets a phone section

`welcome.go` gains a "Phone or tablet" section after the desktop install block, generated from the same manifest, with the handset key and the goatnet's management URL. Its content, in the order the person does it:

1. Install NetBird from the App Store or Google Play.
2. Open the app, choose "Change Server", enter the management URL (`https://netbird.<public zone>`).
3. Enter the setup key labelled *handset* from this guide.
4. Connect and accept the VPN prompt. iOS and Android run one VPN at a time; turn any other VPN off first.
5. Android only: Settings → Network & internet → Private DNS → Off, or mesh names will not resolve.
6. The peer appears on the goatnet as `peer-<handle>-<site>-<date>-<device name>`. It reaches the same web services as a laptop, through the same login. It cannot run the `goat` command, publish to the fabric, or receive files.

The retired bundle's text at `bundle.md.j2:40-50` is the seed. Built 2026-09-10: `renderPhoneBlock` in `ops/goat-id-console/internal/welcome/welcome.go`, asserted by `TestRender_phoneBlockUsesTheHandsetKey`, `TestRender_phoneBlockFallsBackToTheDeviceKeyWithAWarning` and `TestRender_phoneBlockAbsentWhenNoHandsetDeclaredAndNoKey` in the same package (no browser e2e covers the guide; the intake form is the only e2e'd surface).

### 2.4 Mesh names on handsets: measure, then one nameserver group per goatnet if needed

Phase 0 of the plan is one measurement on an enrolled phone on ZAGROS: open `https://guide.zagros.netbird.zagros.example/public/onboarding-guide` in the phone's browser, and on Android repeat with Private DNS off. If the page loads, the pushed match-domain group works on phones despite the vendor's stated limit, and nothing changes.

iOS passed on 2026-09-10, so this section now applies to Android only. If it does not load there, each goatnet gets a second nameserver group, `<site>-handset-dns`, carrying the same CoreDNS replicas as `<site>-mesh-dns` but `primary: true` with no match domains, distributed to the group `<GOATNET>-HANDSET`. The handset key's `auto_groups` becomes `[OBSERVER, HANDSET]`. CoreDNS already forwards everything outside the mesh zone to public resolvers (`ops/ansible/roles/mesh-coredns/templates/Corefile.j2:27-28`), so a phone with the mesh resolver as its primary still reaches the internet; the cost is that all of the phone's DNS crosses the tunnel while connected, which a VPN client's user expects.

`HANDSET` is a device-attribute group that appears in no ACL rule: it exists to distribute DNS and nothing else. It is declared in `ops/ansible/netbird_acl_taxonomy.yml` (suffix `HANDSET`, in `_default` and `zagros`) so that the daily reconcile does not report it as undeclared and `netbird-acl-bootstrap.yml` creates it.

**Built 2026-09-10, default-off.** `goat-coredns.yml` provisions `<site_id>-handset-dns` only when the goatnet's group_vars set `handset_dns_enabled: true`, and refuses if the HANDSET group is missing; the mint (`mint_handset_key`) adds `<prefix>-HANDSET` to the handset key's groups once that group exists on the management, and says so when it does not. Enable per goatnet after the Android measurement, with one `goat-coredns.yml` run.

Desktops keep the match-domain group. Making the existing group primary for everyone would send every laptop's DNS through the mesh and leave it without DNS when the mesh is down, which the name-fallback resolver exists to prevent.

### 2.5 Attribution of a handset rides the peer name, as for every other peer

A handset enrolled on a handset key is named by the `auto_peer_name` on that key with the device's own name appended, which is the deterministic shape `peer-identity-resolution.md` resolves. The person behind a phone is therefore known to `goat who`, netvis and the surface-visit records the same way the person behind a laptop is. A phone enrolled on a hand-minted key is named after the device alone, `iPhone`, and is unattributable; that is the F-338 defect, closed for handsets by 2.2.

A person's identity on a web surface comes from the Ory door, not from the peer: a phone browser logs in with the same password or passkey, and the upstream sees `X-Forwarded-User` (ADR 1163). Nothing about that is phone-specific.

### 2.6 Revocation, loss and expiry

- Revoking the person revokes both keys and cuts every peer, handset included, on the existing path (`credential-revocation-and-offboarding.md`).
- A lost phone is one peer cut, `goat admin peer cut <peer-name>`, plus revocation of the handset key and a fresh mint through the console, so the finder cannot re-enrol on the remaining reuse count. Built 2026-09-10 as `goat admin onboard <handle> --env <env> --native --reissue-handset-key`, which revokes the person's live handset keys and mints a fresh one, and as a console button on the onboarded panel. The button queues a `reissue_handset` row in `onboard_actions`, a follow-up queue beside the proposals queue; the runner claims it exactly once in its poll loop, runs the verb, and writes the new key back, and the console stores it on the proposal and re-sends the welcome package with it. A second click while one is pending queues nothing. The same queue carries `reissue_device` for an expired or used-up laptop key (`--reissue-device-key --uses N`, id-admin-console.md §"Mesh key reissue"), and both reissues now record an operator-action audit event naming the requester.
- The handset key expires with the device key. A phone that is off for longer than the peer-liveness expectation is reported, not paged (`peer-liveness-expectations.md:151`).

## 3. What a phone can and cannot do on the goatnet

| | phone or tablet on the handset key | why |
|---|---|---|
| open any mesh web surface (portal, panoscope, status page, guide, gym, help desk) and log in | yes | OBSERVER reach on service ports (rule A); Ory login works in a phone browser, passkeys included |
| reach a partner-hosted service peer | yes | rule C, every person role reaches EXTERNAL-INFRA one way |
| resolve `*.netbird.<zone>` names | yes on iOS; Android to be measured (2.4) | pushed nameserver group is match-domain only; iOS applies it anyway |
| iTAK / ATAK on the mesh | yes | `docs/operations/tak-client-onboarding.md:80-83` already assumes a phone on a setup key |
| run `goat`, `goat top`, `goat who` | no | no phone binary; `get.<site>` serves desktop targets only |
| publish or subscribe on the fabric | no | no Zenoh certificate on a handset; OBSERVER has no 7447 reach. A tablet that needs it is a DEVELOPER handset by sponsor decision |
| goatdrop / filedrop | no | goat-client feature, desktop only (ADR 1063) |
| keep mesh names when mesh DNS is down | no | name fallback needs goat-clientd; a phone falls back to nothing |
| reach another person's device | never | handsets are never P2P (runbook) |

## 4. Constraints this design places on other work

- **`GOAT_RELAY_REQUIRE_PEER_BOUND=1` stays unset** on every relay until a handset client exists that speaks peer-bound tokens. Flipping it breaks every stock NetBird client, which today means every phone.
- **The Rosenpass enforce flip** already waits on a goat-client update path (`goat-client.md`); handsets on the stock app add a second reason: they have no PQC path at all and would be stranded.
- **goat-client mobile**, when it ships, replaces step 1 of the guide section and inherits the handset key unchanged. It also brings the name-fallback tier and filedrop to phones. Nothing here should be built in a way that assumes the stock app forever.

## 5. Not in this design

- QR-coded or deep-linked enrolment for the phone. The person opens the welcome email on the phone and copies the key; a `netbird://` deep link or a scanned bundle is a later convenience, and goat-client's bundle QR (`goat-client.md:212-213`) is the designed place for it.
- Mobile device management, always-on VPN profiles, or any control over the phone beyond the peer.
- A per-device certificate for a phone (the ADR 0843 crutch tier). That tier is for reaching a service without a tunnel; a phone on the tunnel does not need it.
