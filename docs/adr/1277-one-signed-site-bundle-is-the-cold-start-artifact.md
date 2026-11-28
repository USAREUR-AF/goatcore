# ADR 1277 — One signed site bundle is the cold-start artifact for a stand-up, a join and a rebuild

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-28 |
| Deciders | operator |
| Repo scope | goat core (goatbuilder, the release path, the site cache roles, `ops/bootstrap/`) |
| Supersedes | — |
| Superseded by | — |
| Related | [`cold-start-designs-reconciled.md`](../design/cold-start-designs-reconciled.md) (the evidence) · [`bootstrap-bundle.md`](../design/bootstrap-bundle.md) · [`goatbuilder-speed.md`](../design/goatbuilder-speed.md) · [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) (row C1, group D) · [ADR 1237](1237-a-site-is-rebuilt-from-nothing-and-a-combed-extract-of-its-identities.md) · [ADR 1109](1109-fleet-management-dimension-and-pull-convergence.md) |

## TL;DR (for a layperson)

A new site needs a few gigabytes of software before it can start: the programs
as container images, some packages, and the instructions. Today each machine
downloads these from the internet, piece by piece, while the site is being
built. This record proposes that one signed archive holds all of it. The
archive can be downloaded once, copied from a neighboring site, or carried in
on a drive. The tool checks its signature before it uses anything inside. The
archive holds no data, no identities and no secrets, so it may be stored
anywhere.

## Context

The tree holds four partial answers to "where does a site's software come from
on day one", and the stand-up uses none of the first three
([cold-start-designs-reconciled.md](../design/cold-start-designs-reconciled.md)):

- `ops/bootstrap/rebuild.sh` builds an encrypted bootstrap bundle that nothing reads.
- `ops/bootstrap/publish-containers.sh` saves a goatnet's pinned images as one signed archive that nothing loads.
- `ops/scripts/goat-image-bundle-build.sh` and `goat-image-import.sh` carry signed images to a closed network; no stand-up step calls them.
- The site cache and the image pre-pull are in the stand-up. The cache starts empty and serves Docker Hub and apt only, so a first site, and every goat image, depends on the internet.

They use three signature schemes. The gate's group D holds a row, "Zero-SaaS
standup, measured", that a stand-up cannot pass while its images come from a
registry.

## Decision

Proposed, for the operator to rule on:

1. **One artifact.** A site bundle is the single cold-start artifact. It
   contains the repository at a release tag, every image the pin table names
   for one goatnet and one architecture, by digest, the package set the node
   classes declare, and a manifest of digests.
2. **It is stateless.** A site bundle holds no mission data, no identity, no
   secret and no runtime state. Enrollment bundles and a rebuild's extract are
   separate artifacts and are never placed inside it.
3. **The release path produces it and the release verifier checks it.** It is
   signed as a release file is, keylessly, and `goatbuilder` verifies it with
   the verifier it already has. The held-key signatures of the earlier
   producers are retired when their producers are folded in. The bundle is not
   encrypted: it holds nothing secret.
4. **Three sources, one order.** A stand-up or a join reads the bundle from
   the site (carried media or the site cache), then from another site of the
   same goatnet, then from object storage. The source is a declared setting
   with those values; it adds no required field to `goatbuilder.yaml`.
5. **The cloud holds it only as a stateless artifact.** Object storage may hold
   a site bundle, as it holds the tool's own releases. Nothing in this decision
   places a control plane, a store or any state in a cloud.
6. **The image pre-pull is the consumer.** Given a bundle, it loads images by
   digest in place of pulling them, and fills the site cache from the same
   bundle. With no bundle it behaves as it does today.
7. **A rebuild uses the same bundle.** The rebuild of ADR 1237 takes the site
   bundle for what to run and its extract for who belongs.

## Alternatives considered

| Alternative | Why it is not proposed |
|---|---|
| Build the April bootstrap bundle as designed | Encrypting a bundle that holds nothing secret adds a key ceremony for no protection, and its signing scheme is a third one beside the release path's |
| Fill the site cache only | The registry mirror cannot serve `ghcr.io`, where the goat images are, and a cache cannot be carried to a site with no internet |
| A registry at each site, kept in sync from the cloud on a timer | Written and switched off. It is a second site-local image source and a second timer beside the cache and the pull converger |
| Leave it as it is | The "no outside account" property stays unmeasured, and a first stand-up's image time stays tied to the site's link |

## Consequences

- The three written producers are folded into one, in the release path. Until
  that is built they stay as they are.
- The time a bundle saves is unmeasured. The timed run of row C1 must first
  record image-pull and package time on their own.
- No gate row is added or removed. A stand-up from a bundle is one way to pass
  the existing "Zero-SaaS standup, measured" row.
- A bundle is large. Its size and how long a site keeps old ones are for the
  design that builds it.

## Trigger conditions for revisit

- The timed record shows image and package time is under five minutes of a
  first stand-up: the bundle is then justified by the no-internet case alone.
- The container runtime gains a mirror setting for `ghcr.io`: the cache alone
  may then serve a connected site.
