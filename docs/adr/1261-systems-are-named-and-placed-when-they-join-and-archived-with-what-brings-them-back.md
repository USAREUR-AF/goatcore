# ADR 1261 — Systems are named and placed when they join, tidied by the tactical admin, and archived with what brings them back

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-24 |
| Repo scope | goat core (enrolment, portal, CLI), goatstuff (`goat-feeds`, `goat-map`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1096](1096-delegated-machine-enrollment.md), [ADR 1189](1189-netbird-read-only-dashboard-and-warrant-owned-membership.md), [ADR 1199](1199-one-error-catalog-interpreted-by-both-clis.md), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md), [ADR 1259](1259-fabric-feeds-self-service-approved-delivery-of-fabric-topics-to-mesh-systems.md), ADR 1260 (proposed: every tactical link in one registry), [`fabric-feeds.md`](../design/fabric-feeds.md) |

## TL;DR (for a layperson)

Field systems join a goatnet with whatever name the laptop had, and nothing records where
they are or what they do. Afterwards someone has to work out which console is at which site
before the status pages make sense. This decision makes the two moments when a system
arrives, joining the mesh and being given a data link, ask for its site and its job and
propose a proper name, and refuse or warn when they are missing. Anything that still slips
through lands on a tidy-up page that the site's tactical admin works, not an engineer. When an
operation ends, its systems are archived with a note of how they are expected to come back,
so they can be brought back in one step.

## Context

A tactical deployment adds systems quickly: sensor nodes, operator consoles, radars behind
a node. On a recent deployment, consoles joined under appliance host names such as
`DESKTOP-` followed by a serial, and links to them showed "receiving system not in the estate"
until an engineer worked out each one's site by hand. When the operation ended there was no
way to say "this radar is coming back to the same place" or "this one will be a different
unit somewhere else", short of deleting it.

What exists: machine enrolment (`goat machine enroll`, ADR 1096) checks only the characters
of a handle and records no site. The setup key's groups come from the node class alone.
Warrant owns group membership (ADR 1189). The estate file of `goat-map` declares regions, sites
and systems, and is the denominator of site readiness; it has no lifecycle. `goat-feeds`
(ADR 1259) lets a person pick any mesh peer, or type a name and address, as a destination.
A boundary node (ADR 1253) is shown by its host name, which says nothing about where the
data goes after it.

## Decision

1. **One name rule.** A system's name is `<kind>-<site>-<function>-<nn>`: lower case,
   `[a-z0-9-]`, the site token is a site id declared for the goatnet, `nn` is two digits.
   Examples: `nav-north-console-01`, `node-harbor-sensor-02`. The rule is implemented once
   per language that enforces it (portal in Go, `goat-feeds` in Go, CLI in Rust), each with the
   same table of test cases, and is served read-only (`GET /api/machines/naming` on the portal,
   `GET /api/naming` on `goat-feeds`) so other tools reuse it rather than copy it.
2. **Joining the mesh asks for site and function.** `goat machine enroll` takes `--site` and
   `--function`; the portal stores both on the enrolment record and validates the site
   against the goatnet's declared sites. When the handle does not follow the rule, the
   response carries a suggested name built from kind, site and function, and the CLI offers it.
   Per goatnet, `machine_naming` is `off`, `warn` (default) or `enforce`; under `enforce` a
   handle that breaks the rule is refused with a catalogued error (ADR 1199). A peer whose
   mesh name is not its handle is reported with the rename to make.
3. **Site groups are opt-in per goatnet.** With `site_groups: true` the setup key also carries
   `<PREFIX>-<SITE>-<class suffix>`, so membership by site exists from the first minute.
   Default off, because a new group changes the generated mesh rules and warrant must own it
   (ADR 1189).
4. **Giving a system a link asks the same questions.** In `goat-feeds` a destination must be a
   declared system (estate file or placement). A bare name or address opens a "declare this
   system" step: site and function from pick lists, a display label, and the suggested name.
   The API refuses an unresolved destination with an error that says what to add.
5. **A boundary node always says where the data goes.** A boundary system declares
   `far_side` (label, direction). Every view shows "To <far side> through boundary node
   <name>", never the host name alone; a boundary system without a far side needs attention.
   The receiver's address stays a secret (ADR 1253); its name does not.
6. **The tactical admin tidies, from a page.** `goat-feeds` keeps placements written by tactical
   and platform admins (site, function, label, lifecycle), audited, layered over the estate
   file. A Tidy up page lists every system or link that is unplaced, misnamed, a boundary
   without a far side, or still switched on towards an archived system, each with its reason
   and the action that fixes it. The merged estate is served (`GET /api/estate/effective`) so
   a placement can be made permanent in the estate file by pull request.
7. **Archive, never delete, when an operation ends.** A system's `status` is `active` or
   `archived`. An archive records when, by whom, the operation, how it is expected to return
   (`same-system-same-site`, `same-system-new-site`, `new-system`, `unknown`), whether its mesh
   peer is kept or must rejoin (`peer-kept`, `rejoin-needed`, `unknown`), notes, and the links to
   switch back on. Archived systems raise no attention and are not expected by site readiness.
   Bring back reverses it and lists, or for requested feeds resumes, those links.

## Alternatives considered

- **Documentation only (a naming guide).** Already true in part: the field procedure says to
  name the peer after the handle, and appliance names still got through. A rule that the
  form and the API apply is what changes behaviour.
- **Infer site from traffic or mesh groups.** The estate is declared, never inferred, because an
  inferred member silently leaves its site when it goes quiet. Groups are used as a hint on the
  Tidy up page, never as the answer.
- **Delete systems when an operation ends.** Loses the history and every setting that bringing
  them back needs; the return expectation is the information worth keeping.
- **Engineers clean up by pull request.** Works, and stays available through the merged-estate
  export, but puts a person outside the operation in the loop for every console.

## Consequences

- A new goatnet starts with `machine_naming: warn` and `site_groups: false`; turning either
  stricter is one setting.
- Existing systems keep their names; the Tidy up page shows which break the rule.
- The estate file gains `status`, `archive` and `far_side`; older files read as all active.
- Site ids become a declared vocabulary that enrolment and `goat-feeds` both read.

## Status of the parts

Designed in this record. The builds are in progress: enrolment site, function and the name
rule in goatcore; placements, Tidy up, archive and boundary far side in `goat-feeds` and
`goat-map`. Site groups (decision 3) are designed, not built, until warrant owns them.
