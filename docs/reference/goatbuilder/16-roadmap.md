# 16. Roadmap

Everything in chapters 1 to 15 is on `main`. This chapter lists the items
around them and the status of each, **built**, **designed** or **not built**,
so a reader can tell them apart.

| Item | Status | What it adds | Design |
|---|---|---|---|
| goatlite | **built**: `preset: lite`, run once on hardware; never for production | a `lite` preset and a smaller verb set over the same binary and contract | [goatlite.md](../../design/goatlite.md) |
| Zero-SaaS recipes | **built**: the store-generic new-site secret generator, the local registry, the local mail sink, alert channels off when empty | a goatnet with no hosted registry, mail or alerting account | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| Tutorials 2 and 3 on hardware | **built** and rehearsed on the fixture; **not yet run** on real machines | the founder-and-joiner pair, and a boundary node granting one topic to one partner, run on real machines | [goatnet-tutorials.md](../../design/goatnet-tutorials.md) |
| A `boundary` machine class | **not built**: tutorial 3 needs none, because the boundary node is the router `goatbuilder federate` deploys | `join-node --class boundary` installing a partner egress node | [goatnet-tutorials.md](../../design/goatnet-tutorials.md) |
| Zero-SaaS standup, measured | **not built** | a goatnet stood up end to end on the `env-file` posture with no hosted account | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| Forge adapter | **designed** | the release path's pull requests, CI waits and admin nomination on a forge other than GitHub | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| OpenBao injector | **designed**; not in this repository | `ops/secrets/openbao/run`, so the `openbao` store resolves `bao://` references | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| `status` on real hosts | **designed** | `status` reads real hosts through the push converger, as `verify` and `diff` do today | [goatbuilder-conformance.md](../../design/goatbuilder-conformance.md) |
| A pull converger | **designed** | `converger: pull-agent`: an agent on each machine applies its class's signed release and `deploy` observes it; push and pull mix per class | [goatbuilder.md](../../design/goatbuilder.md) |
| A semantic error matcher | **designed** | a fourth matcher after the lexical one | [error-interpretation.md](../../design/error-interpretation.md) |
