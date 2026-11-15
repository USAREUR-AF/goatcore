# Site-node troubleshooting — start from the symptom

Use this page when a **site infrastructure node** cannot be installed, booted,
reached, or brought to a healthy and consistent state. It is a routing page,
not a second runbook: identify the first matching symptom, perform only the
read-only observations listed here, and then follow the linked canonical
recovery procedure.

This page covers core, observability/storage, network-jump, and break-glass
nodes. A sensor, gateway, control station, effector, or pod is a mesh peer, not
a site node; start at `node-addition.md` §1
to classify it.

> **Safety boundary.** The diagnostics below do not change a node. Do not
> restart a service, unlock or reformat a volume, re-enrol a peer, change an
> ACL, promote a standby, or re-run deployment until the applicable recovery
> runbook has been selected and that action is authorized. Capture the evidence
> first. Never paste credentials, tokens, passphrases, private keys, or command
> output containing them into an issue.

## Fast routing

| What the operator sees | Go to |
|---|---|
| Installer, config preflight, or phase 1 stops before a usable host exists | [Installation or preflight fails](#installation-or-preflight-fails) |
| No normal boot, LUKS prompt, or no usable console/recovery door | [Boot, LUKS, console, or recovery access fails](#boot-luks-console-or-recovery-access-fails) |
| The node is up but cannot be reached on its local network | [LAN reachability fails](#lan-reachability-fails) |
| LAN access works but the node is absent or degraded on the overlay | [Mesh reachability fails](#mesh-reachability-fails) |
| The overlay works but enrollment, identity, DNS, or management does not | [Management reachability fails](#management-reachability-fails) |
| The node is reachable but a declared service is absent or unhealthy | [Node service health fails](#node-service-health-fails) |
| Services run, but actual state differs from declared intent | [Consistency fails or is unknown](#consistency-fails-or-is-unknown) |
| A participant laptop or the `goat` CLI is the failing endpoint | [`goat doctor` troubleshooting](../goat-cli-book/src/troubleshooting.md) |

If more than one row matches, start at the highest physical layer: boot, then
LAN, mesh, management, services, and finally consistency. A higher layer cannot
make a lower one healthy.

## Installation or preflight fails

**Entry conditions.** The installer did not reach the documented hand-off
state, `standup-config-preflight.py` reports a missing or foreign value, or
phase 1 refuses before the site-node baseline is complete.

**Non-mutating diagnostics.** Record the hardware class and intended node role;
the exact failing phase, command, and first error; and whether the failure is on
the control machine, installer console, or target. If the intended group-vars
directory exists, run the full offline preflight from the repository root. This
is the mode that checks values, including unresolved `TBD_` sentinels and
foreign-goatnet references:

```text
ops/scripts/standup-config-preflight.py \
  "<site-group>"
```

If the group-vars directory does not exist yet, use the file-inventory-only
planning mode instead:

```text
ops/scripts/standup-config-preflight.py \
  "<site-group>" --scaffold-plan
```

`--scaffold-plan` reports which files are owed; it does not run the value
checks. After the directory exists, the full preflight above is the required
gate.

For an installed target, compare the observed hand-off facts with
[`bare-metal-install.md` §4](bare-metal-install.md#4-verify-the-hand-off-state).
Read the standup driver's message before doing anything else; its preflight
refusals intentionally name stale checkouts, missing configuration, unresolved
sentinels, and unsafe target breadth.

**Canonical recovery.** For installation media, firmware, storage layout, or
the SSH-ready hand-off, use [`bare-metal-install.md`](bare-metal-install.md).
For a new goatnet, resume at [`goatnet-standup.md` §Before phase 1](goatnet-standup.md#before-phase-1--what-the-driver-assumes-already-exists).
For a site node joining an existing goatnet, use
`node-addition.md` §2
and preserve its founder/joiner ordering.

**Pass condition.** The bare-metal hand-off checks pass, the config preflight
has no unexplained `MISSING`, `TBD_`, or foreign-goatnet value, and the intended
host is explicitly narrowed before phase 2.

**Escalate when.** The observed disk or boot layout differs from the supported
hardware path; a required config absence has no documented parity decision; or
the next step would require inventing a command, bypassing a refusal, or
targeting more than the intended host.

**Evidence to collect.** Hardware and firmware version; installer image and
checksum; boot source; disk inventory with serial numbers redacted if needed;
preflight output; phase and command; first error; and the expected versus
observed hand-off facts.

## Boot, LUKS, console, or recovery access fails

**Entry conditions.** The node does not reach the normal OS, remains at an
unlock prompt, rejects the expected unlock method, or neither the normal shell
door nor the declared recovery path can reach it.

**Non-mutating diagnostics.** At the physical or out-of-band console, record
the last visible boot stage and exact error. If the OS is running, inspect only:

```text
lsblk --fs
systemctl --no-pager --full status 'systemd-cryptsetup@*'
journalctl -b -u 'systemd-cryptsetup@*' --no-pager
```

From an authorized operator machine, test each **declared** shell door with
batch mode so the probe cannot fall through to password guessing. Record which
transport was used and whether failure was name resolution, routing, host-key,
public-key, or timeout. Do not repeatedly try an uncertain passphrase and do
not alter LUKS slots while diagnosing.

**Canonical recovery.** Follow
[`site-node-boot-recovery.md`](site-node-boot-recovery.md). It selects the
declared physical-console, remote-initramfs, or boot-source path and defines the
point at which recovery must stop. A failed core that needs a role change or
data rejoin is a failover event; use the goatnet-specific runbook, not an
improvised service start.

**Pass condition.** The expected volume is unlocked by its declared mechanism,
the node reaches the normal OS, the encrypted data mount is present, and at
least one roster-backed normal shell door plus the declared independent
recovery path have been demonstrated.

**Escalate when.** No console or independent door exists; the volume metadata,
device name, or unlock class differs from declaration; all enrolled methods
fail; a recovery would change a key slot; or there is any sign of disk damage.

**Evidence to collect.** Node role and hardware class; last console screen or
serial log; boot identifier; `lsblk --fs`; cryptsetup unit status and boot-log
excerpt; declared unlock class; each access path tried and its failure class.
Exclude passphrases, keys, and credential-bearing kernel arguments.

## LAN reachability fails

**Entry conditions.** The OS is up locally, but an expected same-site address,
gateway, peer, or operator path cannot reach the node.

**Non-mutating diagnostics.** On the console, capture:

```text
ip -brief link
ip -brief address
ip route show
networkctl --no-pager status
ss -lntup
```

Record link LEDs, switch port, cable/NIC, VLAN, DHCP versus static intent, and
the source and destination of one failed probe. Compare the facts with the
portable LAN design in `canonical-lan.md`. Do not
apply netplan, flush neighbor state, move VLANs, or change a switch while still
collecting the baseline.

**Canonical recovery.** Correct install-time NIC or addressing facts through
the node's inventory/group vars and the owning standup or node-addition path:
[`goatnet-standup.md`](goatnet-standup.md) for a new site or
`node-addition.md` for a joiner. Site-specific port, VLAN,
and resolver facts belong only in that site's network runbook; see the clearly
separated references below.

**Pass condition.** The intended interface is up with its declared address and
route, the same-site gateway and required same-site peers are reachable from
the documented source, and the relevant acceptance rows pass.

**Escalate when.** Observed cabling/VLANs do not match an approved site record;
the same address appears on two nodes; the gateway or switch is outside the
operator's authority; or recovery would interrupt other site nodes.

**Evidence to collect.** Timestamp and site/node role; sanitized interface,
address, route, neighbor, and listening-socket output; switch port/VLAN; link
state; probe source/destination/result; and expected inventory/group-var facts.

## Mesh reachability fails

**Entry conditions.** LAN access works, but the node is absent from the mesh,
reports disconnected/relayed unexpectedly, cannot reach required mesh peers,
or the mesh health domain is `FAIL` or `UNKNOWN`.

**Non-mutating diagnostics.** Capture the local client status and the existing
health view:

```text
netbird status --detail
goat health --domain mesh \
  --context "<goatnet-context>"
```

Run `netbird status` on the affected node. Run `goat health` from an authorized
operator profile whose explicit context names the affected goatnet.

Record whether management and signal are connected, the peer path type, the
last successful contact, and whether the failure affects one node, one site,
or the goatnet. Inspect recent service logs without restarting anything.
Remember that `UNKNOWN` and silence are not green; use the health meanings in
[`environment-health-surface.md` §3.3](../design/environment-health-surface.md#33-the-honesty-model--generalized-never-a-false-green).

**Canonical recovery.** For a node being added, return to the mesh ordering in
`node-addition.md`. For a site standup, use
[`goatnet-standup.md` §Phase 8](goatnet-standup.md#phase-8--mesh-enroll-in-full).
Use the remediation named by the health output; do not delete client state or
re-enrol merely because a peer is absent. Participant endpoint cases belong in
[`goat doctor`](../goat-cli-book/src/troubleshooting.md), not here.

**Pass condition.** The node is connected under its intended identity, required
peer paths satisfy the declared policy, and a fresh mesh-health reading is
`PASS` (or a documented non-green state whose cause is outside this node).

**Escalate when.** Re-enrolment would change the node's mesh IP or identity;
policy appears to deny a required path; multiple nodes or sites fail together;
or management/signal is unavailable and the independent recovery door is also
lost.

**Evidence to collect.** Sanitized detailed status; mesh-health grade and age;
client and mesh-health service log excerpts; intended node identity/groups;
affected peer set; path type; and last-known-good time. Do not include setup
keys or management tokens.

## Management reachability fails

**Entry conditions.** The node has working LAN and mesh paths, but enrollment,
management, signal, identity login, or mesh DNS cannot be reached or returns an
unexpected status.

**Non-mutating diagnostics.** Resolve the configured names through the
documented resolver, record the returned addresses, and make a read-only TCP or
HTTP health request to the configured endpoint. Capture certificate subject,
SAN, issuer, and validity dates without exporting private material. Compare
failures from the affected node with a known-good node to separate node, site,
and service scope. Inspect service status and logs on the owning core node; do
not change DNS, certificates, policy, or enrollment state during triage.

**Canonical recovery.** A standup failure returns to
[`goatnet-standup.md` §§Phase 8–12](goatnet-standup.md#phase-8--mesh-enroll-in-full).
A joiner returns to the ordered passes and founder-side mirror in
`node-addition.md`. Use
`dns-service-discovery.md` for the DNS
contract and the goatnet-specific failover runbook if a core role must move.

**Certificate renewal fails on a network that restricts DNS.** When the
Traefik log of a core node shows `Error renewing certificate` with `could not
find the start of authority` and a timeout to a public resolver, the site's
network permits DNS only to its own resolver. Confirm it from the node: a
query to a public resolver times out and the same query to the resolver the
network hands out answers. Declare that resolver in the site's group variables
(`acme_dns_resolvers: ["<address>:53"]` and
`acme_dns_skip_authoritative_checks: true`) and run `traefik-acme.yml` for the
site; the next renewal attempt then succeeds. The served certificate keeps
working until its expiry date, so read that date first to know how long there
is.

**Pass condition.** The configured name resolves to the intended current
endpoint, TLS identity and time validity are correct, the management and signal
connections succeed, and the applicable acceptance rows pass from the affected
node's network context.

**Escalate when.** The symptom spans a site or goatnet; DNS and declared intent
disagree; certificate identity or time is wrong; a role promotion is indicated;
or recovery would alter public DNS, ACLs, identity, or replicated state.

**Evidence to collect.** Endpoint name and expected role; sanitized DNS answer
and resolver used; TCP/HTTP result; certificate metadata; relevant service
status/log excerpt; affected scope; last-known-good time; and the matching
acceptance rows. Never collect cookies, bearer tokens, or private keys.

## Node service health fails

**Entry conditions.** The node is reachable on LAN and mesh, but a declared
systemd unit or container is inactive, restarting, unhealthy, missing, or not
serving its declared port.

**Non-mutating diagnostics.** Capture, without restarting:

```text
systemctl --failed --no-pager
systemctl --no-pager --full status "<declared-unit>"
journalctl -b -u "<declared-unit>" --no-pager
docker ps --all --no-trunc
docker compose -f "<declared-compose-file>" ps
```

Use only the compose path declared for that service. Record disk/inode pressure,
memory pressure, and the image digest shown by the runtime. Determine whether
the service is declared for this node class before treating absence as failure.

**Canonical recovery.** Re-converge through the service's owning Ansible role
or the phase named by [`goatnet-standup.md`](goatnet-standup.md); do not repair
deployed files by hand. Then run the generated acceptance suite as described in
[`goatnet-standup.md` §Verification](goatnet-standup.md#verification--done-means-green-proven).
For a newly added node, the completion contract is
`node-addition.md` §6.

**Pass condition.** Every service declared for the node class is active and
healthy, serves its declared endpoint, has the intended image digest, and all
applicable acceptance rows are `PASS`; `UNKNOWN` is not a pass.

**Escalate when.** The owning role or phase is unclear; logs indicate corrupt
state, failed replication, or credential/certificate mismatch; several
services fail from a shared dependency; or recovery would delete data, promote
a role, or bypass an acceptance gate.

**Evidence to collect.** Node role; failed unit/container status; bounded boot
logs with secrets redacted; compose file path; image digest; resource pressure;
declared service facts; acceptance rows; and last-known-good deployment ledger
entry.

## Consistency fails or is unknown

**Entry conditions.** Services appear to run, but `goat health --domain
consistency` or the consistency console reports `FAIL`/`UNKNOWN`, a stale
generation, a wrong digest, missing observation, or a node absent from roll-up.

**Non-mutating diagnostics.** Read the same fact at two altitudes:

```text
goat health --domain consistency \
  --context "<goatnet-context>"
goat top --once \
  --context "<goatnet-context>"
```

Run both commands from an authorized operator profile. The explicit context is
mandatory: a default context can silently inspect a different goatnet.

Record the node, grade, reason, observation age, intended generation, observed
generation/digest, and the remediation pointer carried by the report. Confirm
that both the per-node report and roll-up are fresh before diagnosing drift.
Do not infer convergence from a running container.

**Canonical recovery.** Follow the report's remediation pointer: the closed set
and re-convergence rule are canonical in
`consistency-foundation.md` §6.
Use the owning deployment path it names, then re-run the acceptance suite.

**Pass condition.** A fresh per-node observation and fresh roll-up agree on the
intended generation and digest and grade the node `PASS`; the corresponding
acceptance rows pass. A stale `PASS`, silence, `UNKNOWN`, or console-only green
does not satisfy this condition.

**Escalate when.** Intent signature or generation cannot be verified; node and
roll-up disagree; the remediation pointer is absent/invalid; multiple nodes
diverge together; or the named recovery would change signed intent rather than
converge the node to it.

**Evidence to collect.** Sanitized CLI/console result; node and roll-up topic
ages; intent generation and signature-verification result; observed digests;
remediation pointer; publisher/roll-up status and bounded logs; acceptance row;
and last-known-good deployment ledger entry.

## Escalation packet

For any scenario, provide a compact packet that another operator can evaluate
without asking you to repeat diagnostics:

1. Site, node, node role, hardware class, timestamp/time zone, and impact.
2. First matching symptom on this page and the layer already proven healthy.
3. Expected state, observed state, first error, and last-known-good time.
4. Commands run and bounded output, with secrets and personal data removed.
5. Canonical recovery page selected, pass condition not met, and the exact point
   at which you stopped.
6. Relevant acceptance/health grades including age; never summarize `UNKNOWN`
   as healthy.
