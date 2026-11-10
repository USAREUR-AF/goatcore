# OS release cadence — which Ubuntu, and when to move

**Status:** measured 2026-09-06 against live AWS instances of 22.04, 24.04 and 26.04. The table
below is the measurement, not a plan someone hopes is true.

**Related:** [`os-baseline-and-luks.md`](os-baseline-and-luks.md) (what the OS must provide),
`fips-pqc-path.md` (why the compliance services matter),
`environment-parity.md` (one fact, one home).

## The answer

**24.04 is the baseline for anything carrying a compliance claim, and it will stay so until
Canonical ships `fips-updates` and `usg` for 26.04.** Not a preference — 26.04 does not offer
them yet. Hosts carrying no compliance claim can move to 26.04 whenever their software supports
it, and gain a strongSwan that has the post-quantum IKE transforms.

## What each release actually offers

`pro status --all` on unattached AWS instances, and the strongSwan the archive ships:

| | `fips` | `fips-preview` | `fips-updates` | `usg` | strongSwan |
|---|---|---|---|---|---|
| **22.04** jammy | no | **yes** | **yes** | **yes** | 5.9.x |
| **24.04** noble | no | no | **yes** | **yes** | not measured |
| **26.04** resolute | no | no | no | no | **6.0.4** |

Three things fall out of that table, and each changes a decision:

- **26.04 has no compliance services at all.** `fips-updates` and `usg` are both available on
  22.04 and 24.04 and neither is on 26.04. A boundary node, or any host whose STIG evidence comes
  from `usg`, cannot move to 26.04 today.
- **26.04 ships strongSwan 6.0.4**, which is the 6.x that `fips-pqc-path.md` names for the
  `ke1_mlkem768` / `ke1_mlkem1024` hybrid transforms. So the release that gives post-quantum IKE
  natively is the one that has no FIPS packaging — the two arrive out of order, and a node cannot
  have both from the archive today.
- **`fips` reads `no` on every release measured**, including 22.04, and the reason is not
  visible from `pro status`: Canonical's pricing page states FIPS crypto-modules are included in
  Pro **with 24.04 certification pending**. So `fips-updates` shipping while `fips` does not is a
  certification-in-progress state, not a packaging quirk. Read the table as what a host is
  offered, and the certification status from Canonical.

  Verified separately on an **attached** host: an Ubuntu Pro 24.04 AWS instance shows
  `fips-updates` and `usg` as *entitled*, and `pro enable fips-updates` installs and boots a FIPS
  kernel (`6.8.0-1063-aws-fips`, `fips_enabled=1`, two OpenSSL FIPS providers). `fips` itself
  stays unavailable on 24.04.

## `fips` and `fips-updates` are not the same claim

`fips` is the validated module as certified. `fips-updates` is that module **plus** security
patches, so it is no longer bit-identical to what CMVP validated. Programs differ on whether
`fips-updates` satisfies a control; some require strict `fips` and accept the CVE exposure that
comes with freezing a module.

**Which one applies here is an assessor question, not an engineering one.** Nothing in this repo
should assert that `fips-updates` satisfies a FIPS control. Where a claim is made, name which
service the host is running.

## Does 22.04 buy a better FIPS claim than 24.04? No.

Asked because the honest answer changes whether the fleet should move backwards. Measured on
**attached Ubuntu Pro** instances of both:

| | `fips` | `fips-preview` | `fips-updates` | enable → FIPS kernel |
|---|---|---|---|---|
| **22.04** | entitled, status **`n/a`** | entitled | entitled | not tried |
| **24.04** | not listed | not listed | **entitled** | **yes** — `6.8.0-1063-aws-fips`, `fips_enabled=1`, 2 OpenSSL FIPS providers |

**`fips-updates` is what is actually installable, and it is available on both.** On 22.04, strict
`fips` reads *entitled* but status `n/a` — an entitlement you cannot simply turn on, not a service
waiting to be enabled. 22.04's only extra is `fips-preview`, which is packages *undergoing*
certification — less certified, not more.

So **moving the fleet back to 22.04 for FIPS would gain nothing and cost fleet parity.** Stay on
24.04, run `fips-updates`, and record which service the claim rests on.

**Where this is incomplete, so nobody over-reads it:** the two rows come from different commands —
`pro status --all` on 22.04 and plain `pro status` on 24.04 — and both are AWS pay-as-you-go
entitlements, not a direct Canonical subscription. A direct subscription may expose `fips`
differently. Settle it by running `pro status --all` on both under the subscription actually
purchased, before writing a certification claim.

Canonical's pricing page states FIPS modules are included in Pro with **24.04 certification
pending**; no date is published there, and the certifications documentation does not list
per-release certificate numbers. Treat the timeline as unknown rather than imminent.

## What it costs

Two routes, and they are priced differently because they are different products.

| | |
|---|---|
| **AWS pay-as-you-go** | **~$0.007/hour** over standard. `c6i.xlarge` in region 1: `$0.194` → `$0.201`. No token, entitled at boot, commercially licensed. |
| **Ubuntu Pro Server**, per physical machine | **$500 / machine / year** (Canonical list) |
| Ubuntu Pro Workstation | $25 / machine / year |
| Server + 24/7 infra support | $1,775 / machine / year |
| Server + 24/7 full support | $3,400 / machine / year |

**Activation runbook:** [ubuntu-pro-activation](../operations/ubuntu-pro-activation.md).

**Getting FIPS onto on-prem hardware without a purchase decision first:** Canonical's Ubuntu Pro
**free trial is 30 days, up to 50 machines, and is explicitly for enterprise and commercial use**.
It includes everything in a Pro subscription except support and Knowledge Base access. It requires
a payment method and **auto-bills at day 30 unless cancelled**, so it is a decision with a
deadline, not a free sample.

Do **not** use the separate free-for-personal-use tier (5 machines) for this. It is licensed for
personal projects, and this is not one — the licence, not the machine count, is why.

**The cloud route does not cover on-premises hardware.** The AWS entitlement is per-instance and
the kernel is `-aws-fips`; a mini PC needs its own Pro subscription and runs the generic FIPS
kernel. Use the cloud route to *test* behaviour that belongs to strongSwan and the archive — such
as which plugin provides a cipher — and a real subscription for anything the on-prem host must
actually claim.

## When to move a host

The trigger is a capability, not a date. Move when **all** of these hold:

1. Every compliance service the host's claims depend on is `available` on the target release —
   check with `pro status --all` on that release, not from a changelog.
2. The software the host runs is packaged for the target, or the pin is bumped and CI is green.
3. The host is not mid-accreditation. An OS change during an assessment invalidates the evidence
   already submitted.

And re-measure rather than assume: F-466
was found on 22.04 and **still reproduces on 26.04 with a different strongSwan major version**, so
release changes do not fix crypto-selection defects on their own — but they can move which
version a defect lives in.

## Checking, rather than remembering

An AMI lookup is where a stale release assumption hides, because it fails in a way that names
nothing. Canonical **changed the AMI path between releases** — jammy under `hvm-ssd`, noble and
resolute under `hvm-ssd-gp3` — and a lookup with the wrong prefix returns
`Your query returned no results`, naming neither the release nor the prefix.

Read the current state instead of recalling it:

```bash
# what releases exist as AMIs
aws ec2 describe-images --region <r> --owners 099720109477 \
  --filters "Name=name,Values=ubuntu/images/*ubuntu-*-amd64-server-*" \
  --query 'reverse(sort_by(Images,&CreationDate))[:5].Name' --output text

# what Pro offers on a release — run ON that release
sudo pro status --all
```

`ops/federation-partnerenv/` takes the release as three variables
(`ubuntu_release_codename`, `ubuntu_release_version`, `ubuntu_ami_storage_prefix`) so a forward
test on the next LTS is a tfvars change, not an edit to the AMI filter. That is the shape any
environment should use: **the release is configuration, not a literal in a lookup.**

## What is not covered here

- **In-place upgrade.** Everything above is about which release a host is *built* on. Nothing here
  has tested `do-release-upgrade` on a goat host, and the LUKS, dm-verity and Tegra constraints in
  `os-baseline-and-luks.md` make an in-place upgrade its own piece of work.
- **Tegra.** Jetson runs Ubuntu-for-Tegra on NVIDIA's schedule, not Canonical's, and its FIPS
  kernel is mutually exclusive with the tegra kernel (ADR 1118). None of this table applies there.
- **24.04's strongSwan version**, which was not measured.
