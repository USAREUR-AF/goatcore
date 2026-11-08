# Tutorial 4 — Federate across the boundary to another sovereign

You will run both sides of a peering yourself: two sovereigns, `pasture` and
`meadow`, each with its own certificate authority, its own signing key and its
own SPIRE trust domain, federating through the real driver. You do it twice,
first on one Linux machine in containers, then on two hosts across a real
network. Both end at the same acceptance gate, read from the receiving side:
data carries both ways, refusals fire, and a round trip completes.

| | |
|---|---|
| Audience | A platform admin who has finished tutorial 3 and wants to see a whole peering before doing one with a real counterparty |
| Time | The container run: about 25 minutes. The two-host run: about 15 minutes per scenario, after prerequisites |
| You need | For the container run: one Linux machine with Docker and the `wireguard` kernel module. For the two-host run: two Ubuntu 24.04 x86_64 hosts you can SSH to with passwordless sudo, UDP 51820 open between them, outbound internet |
| You finish with | Two federated sovereigns, proven from both sides in both orders, and a record directory you can read back |
| Prior | [Tutorial 3 — a boundary node](03-boundary-node.md) |

Tutorial 3 left you with a plan and a counterparty to wait for. Here you play
the counterparty too. Nothing about the driver changes: every phase you run is
the one you would run against a real partner. On container hosts, phase 5
exchanges both packages, CA certificates and SPIFFE bundles through a peering
directory the run starts beside the two edges (saltlick behind a TLS front),
exactly as against a real partner; the voice call is a keyring the run writes. On
cloud hosts (`live.sh`) the run hands the files between its own two hosts with
`FED_NO_DIRECTORY=1`. Phase 10's review is done the way two operators do it, by
reading what arrived.

Both runs below are synthetic: two sovereigns of your own, synthetic samples, and FIPS waived. A real
peering also needs a counterparty, and the validated FIPS backend where your accreditation requires it.

## The words used here

- **sovereign** — a party with its own goatnet, its own certificate authority and
  its own identity domain. Here both are yours, and they share nothing.
- **edge** — the one router per partner that trusts that partner's authority and
  nothing else. Your core never trusts a partner.
- **the agreement** — one signed document both sides compose from their two
  packages, with one digest and no perspective. Phase 5 composes it.
- **the receiving side** — where every proof is read. A sender's success says
  nothing; a denial is silent (F-177).

## The fourteen phases, in one table

| # | Kind | What happens |
|---|---|---|
| 1 | gate | Preflight: the declaration, the edge group, the tools |
| 2 | task | Prepare the edge: the WireGuard interface and the SPIRE bundle export |
| 3 | task | `goat federate init`: your half, read from the running system |
| 4 | task | `goat federate lint` |
| 5 | manual | Exchange packages, CA certificates and SPIFFE bundles through the peering directory; read the keys aloud and record them |
| 6 | task | `goat federate compose`: one agreement |
| 7 | task | `goat federate verify` from your side |
| 8 | task | Mint the peering leaf certificates from the agreement |
| 9 | task | `goat federate apply`: the access rules and configuration |
| 10 | manual | Review what will change, check the partner CA, admit the edge on your core |
| 11 | task | Open the transport the agreement selected (WireGuard or IPsec), refuse any other |
| 12 | task | Deploy the edge router |
| 13 | task | Federate the two SPIRE trust domains |
| 14 | gate | Acceptance: carries, refuses, round-trips, read from the receiving side |

## Run it in containers

On a Linux machine with Docker and the `wireguard` module, with `goat` and
`goatbuilder` built from this checkout:

```text
ops/federation-e2e/run.sh --goat <path to goat> --goatbuilder <path to goatbuilder>
```

The run needs nothing beyond this checkout. It reads the SPIRE image from the
`spire` component in `ops/pins/fleet-pins.yaml` and writes each sovereign's own
`ops/goatbuilder/goatbuilder.yaml` and edge `group_vars` into that sovereign's
clone, so a public checkout runs it unchanged. To check that preparation without
Docker, on any machine with `goatbuilder` built:

```text
ops/federation-e2e/run.sh --prepare-only --goatbuilder <path to goatbuilder>
```

It generates both sovereigns, validates each clone's config, plans each side's
federation, and stops before the first container.

Four privileged containers stand in for four hosts: a core and an edge per
sovereign. The two cores cannot reach each other or the boundary; the two edges
share the boundary network, and only the tunnel's UDP port crosses it. The run
does two scenarios: `pasture` goes first through `goatbuilder federate`, then
`meadow` goes first through `ops/scripts/peering-standup.sh`. Nothing is
asymmetric, so both must pass.

**Checkpoint 1.** The run prints four lines of this shape, one per side per
scenario, and every one must read `fail=0 skip=0`:

```text
acceptance-counts: pass=13 fail=0 skip=0 as=pasture
```

## Run it on two hosts

The same scenario across a real network, one host per sovereign:

```text
ops/federation-e2e/live.sh --goat <linux goat> --goatbuilder <linux goatbuilder> \
  --pasture-host <public ip> --meadow-host <public ip> \
  --pasture-key <ssh key> --meadow-key <ssh key>
```

Run `--dry-run` first and read what it will do on each host. It installs its
own prerequisites, ships the commit you are on, and generates each sovereign's
authority and keys on its own host; no private key leaves the host it was made
on. The binaries are the `federation-live-binaries` artifact of a green
`federation-e2e` run, so the hosts run the commit CI proved.

Each host carries its sovereign's core and edge together. That departs from the
runbook, which keeps the edge off the core's machine, and it is acceptable only
because nothing here is real data. Between the hosts, WireGuard over the public
addresses is the only path, and the run checks that nothing core-side answers on
a public address before any phase runs.

**Checkpoint 2.** The same four `acceptance-counts` lines, all `fail=0 skip=0`,
and a record directory with each side's acceptance log, the router traces that
show the refusals, and each SPIRE server's log.

## What the gate proves

Thirteen checks per side:

- **Carries.** A sample published on your egress lane is read back from the
  partner's core, and theirs from yours.
- **Refuses.** Writes outside the agreed prefix are denied, proven from the
  receiving edge's access trace next to an allowed control write, because a
  refusal checked against a dead path passes by accident.
- **Identity.** A forged name matches no subject. A certificate their authority
  mints with your core's name also matches no subject, because your edge binds
  your core's identity to the core link: zero authorizations on any key, and
  nothing lands.
- **SPIRE.** Your server refreshes their trust bundle across the tunnel, the bundle
  you hold is the one they publish, and a workload certificate their server issued
  validates against it and not against your own bundle.
- **Round trip.** Last, because it is the only check that fails when an earlier
  stage half-works. Cut the link and it fails; restore it and it passes.

## Undo

`live.sh` signs its hosts out of the registry unless you pass `--keep`; the hosts themselves are yours to destroy. On a real
peering, cutting the link is local and immediate: stop the transport on your
edge, and remove the partner's authority file. See
[the runbook, step 9](../operations/sovereign-federation-runbook.md).

## What you have now, and what is still ahead

You have run both sides of a peering, proven from both receiving sides. What
remains before a real one: a counterparty who runs their own goatnet and, if your
accreditation requires it, the validated FIPS backend.
Sharing released data and sealing it to one receiver is
rung 10 of the [learning ladder](../design/goatlite.md#8-the-learning-ladder).
