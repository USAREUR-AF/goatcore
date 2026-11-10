# Federation partner declarations

**One file per federated sovereign partner.** This is where the trust level lives — the thing ADR
1157 says is *"declared data in the repo, changed by pull request, reviewed like any other
configuration"*, and deliberately **not** a certificate extension and **not** a signed memorandum.

Canon: [ADR 1157](../../docs/adr/1157-cross-sovereign-federation-edge-terminated.md) ·
design ·
[runbook](../../docs/operations/sovereign-federation-runbook.md) ·
[partner-side runbook](../../docs/operations/partner-federation-runbook.md)

## Why a file rather than a certificate

Because the level has to change faster than a certificate lifetime, be reviewable by people who
are not cryptographers, and be diffable. The mechanism that would have carried it inside a
certificate — RFC 5280 policy mapping — is unimplemented in our TLS stack and is being deprecated
by the standards body anyway (obstruction register, O1/O3).

## Why a file rather than a database

Because the review *is* the control. A pull request against one of these files is a legible record
of who widened a partner's access, when, and with what argument in the description. That is the
audit artifact an assessor asks for, and it costs nothing extra to produce.

## The contract

- **Adding a file grants nothing.** It declares an intent. Access follows only when the edge role
  is run with it.
- **`level` is the whole trust decision.** `recognised` holds material and nothing else;
  `connected` allows a tunnel and a session that can do nothing; `exchanging` is the only level
  that moves data.
- **`ingress` and `egress` are exact prefixes, never wildcards at the root.** The validator
  refuses `**`.
- **`partner_cn` must be unique across all partners and must not collide with any of our own
  service names.** Two partners sharing a name, or a partner using one of ours, is the failure
  F-424 describes; the validator refuses it.

Validate with `ops/federation/validate-partners.py` — wired into CI.

## What else lives here

| | |
|---|---|
| [`reference/`](reference/) | Reference implementation of the package format — one implementation of the spec, not the contract |
| [`vectors/`](vectors/) | Byte-exact conformance vectors. These, and the spec, ARE the contract |
| [`republisher/`](republisher/) | The **outbound** republisher: lifts our declared release set from its local staging name onto the wire name `from/<our sovereign_id>/<set>/`, because the fabric cannot rename a key expression in transit. Runs on our core host, both sessions on our core router; deployed in peering phase 12 |
| [`bundle/`](bundle/) | Packs the spec and the vectors into one archive a partner can be handed, with a manifest binding them and a verifier the partner runs. Closes spec §8.3's own concession that *"an external partner may not have"* the vectors |
