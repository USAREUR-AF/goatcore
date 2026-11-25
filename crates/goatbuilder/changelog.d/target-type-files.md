---
heading: For someone deploying
title: "Added: `join-node` and `standup` read each machine type from a file, and a private overlay can add its own types"
---

What a machine of each type (`core`, `obs`, `breakglass`, `network-jump`,
`phone`) converges as is now a data file in `ops/goatbuilder/targets/`, and the
kinds of hardware a type accepts are files in `ops/goatbuilder/families/`
(ADR 1257). Every plan is unchanged. A checkout without those directories uses
the files in goatbuilder; one that has them reads only its own. A private overlay adds types and families in
`ops/goatbuilder/overlay/`, with no public change. The format is in
`ops/goatbuilder/targets/README.md`.

New refusals, each before any phase: a type or family file that is not valid,
an unknown field included (GB-374); a platform family with no file (GB-375); a
machine whose declared `platform_family` its type does not accept (GB-376); and,
for a run, a play of the type that is not in the tree, with the repository it
comes from (GB-377). In an assembled tree, a file in `targets/` or `families/`
that is not the tree's goatcore's own is refused (GB-378): an overlay only adds.
A machine's `platform_family` is read as Ansible resolves it; one that declares
none is not checked. The list of known classes in GB-247's message is now in alphabetical
order.
