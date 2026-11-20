# The zero-SaaS lab run: a goatlite stood up with no route out, every packet recorded

**Lab only.** Nothing in this directory is part of `goatbuilder` or of any deployment.
No product command calls these files. In particular, `phase11-answer.mjs` and
`laptop-phase11.sh` stand in for the person at phase 11 in this lab and nowhere else.
The product's phase 11 stays a person's step ([goatlite.md](../../../docs/design/goatlite.md) §5 row 6).

## What it measures

Tutorial 00 says a goatlite needs no account with any outside service. This run checks
the stronger statement: **during the standup, no machine of the run sends a packet to an
outside address or asks DNS for an outside name.** A run that passes this also passes "no
account", since an account is always reached over the network.

- **The machines:** the laptop that runs `goatbuilder` and the box it stands up.
- **The clock:** from `goatbuilder standup` starting to `goatbuilder verify` finishing,
  including phase 11.
- **Outside the clock, as for every timed run:** building the box image
  (`ops/vm-image/build.sh`, which pulls every image) and preparing the laptop
  (`laptop-provision.sh`, tutorial 00 section 1: the packages, the checkout, the two
  programs, Ansible, the browser, and the Go modules phase 1 builds from).

## How

| Machine | Address | Network |
|---|---|---|
| laptop | `10.88.0.1`, `fd88::1` | the private segment, plus qemu user networking with `restrict=on`: the runner reaches it over SSH, and its gateway drops every other packet |
| box | `10.88.0.2`, `fd88::2` | the private segment only; default route via the laptop, which forwards nothing; DNS server the laptop, which runs none |

The segment is a qemu `stream` netdev between the two VMs. Neither machine has a route to
anything else. Both machines have a default route, so an attempt to reach outside puts a
packet on the wire instead of failing inside the kernel. qemu's `filter-dump` writes every
frame of both laptop NICs to pcap. Every frame the box sends crosses the laptop's segment
NIC, so the record is complete whatever either guest does, and it needs no root and no
change inside either guest.

The laptop resolves the goatnet's names through the box, the alternative tutorial 00
offers to the hosts-file line, and it also gets the hosts-file line. The box's own
resolver points at the laptop, so every name the box looks up is in the record.

**The pass rule** is `judge.py`: over the clock's window, no packet to an address outside
`10.88.0.0/24`, `fd88::/64`, `10.0.2.0/24` or `fec0::/64`, and no DNS question for a name
outside the goatnet's zone or the lab's reverse zones. Multicast, link-local and broadcast
frames belong to the segment and are not counted. `tests/test_zero_saas_judge.py` covers
the rule.

**The laptop's own operating system** is quietened before the clock: unattended upgrades,
the apt timers, the MOTD news, the firmware-update check, snapd and the clock sync are stopped. The record then holds
what the run does, not what the laptop's own Ubuntu does. The box is the goatlite image as
built, and nothing on it is turned off for the lab.

**The source of record.** By default `goatbuilder` fetches `origin` before it touches a
host (contract C3, `ops/scripts/fresh-source.sh`), and a learner's `origin` is the
published repository. With no route out that fetch fails, so the run passes
`goatbuilder --offline`: the product's own flag for a site with no route to its source of
record. The checkout's `main` and `origin/main` both name the commit under test.
`LAB_GOATBUILDER_FLAGS=` (empty) runs the default path instead, and the record then
shows the fetch.

## Run it

It needs Linux, `qemu-system-x86_64` (KVM if it can, emulation is very slow), `qemu-img`,
`xorriso` or `cloud-localds`, `ssh`, `curl` and `python3`. It needs no root.

```bash
ops/vm-image/build.sh --out /path/image                              # the box image
ops/vm-image/zero-saas/lab.sh prepare-laptop --work /path/lab        # the laptop, with network
ops/vm-image/zero-saas/lab.sh run --work /path/lab \
  --image /path/image/goatlite-ubuntu-24.04-amd64.qcow2 --record /path/record
```

`LAB_BOX_MEMORY_MB` (default 10240) and `LAB_LAPTOP_MEMORY_MB` (default 3072) size the two
VMs.

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | the standup and verify passed, and the judge passed |
| 1 | the judge failed: `verdict.json` lists every outside destination and name with its count |
| 3 | the standup or verify failed; its logs are in the record |
| 2 | usage or environment |

**The record** holds:

- `summary.json` and `verdict.json`
- both pcaps
- the standup, phase 11, verify and status logs. Every value shaped like a key or token is
  masked; the goatnet is a throwaway in any case.
- `timings.tsv` and both consoles

CI runs it in `.github/workflows/goatlite-vm-image.yml`, after the image build, when the workflow is started by hand. It does not run on a push or a pull request until it can pass.
