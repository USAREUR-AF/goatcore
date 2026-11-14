---
heading: For someone deploying
title: Changed: every site's end state declares every service, and a joining site's missing ones are named gaps
---

`goatbuilder endstate` and `ops/goatbuilder/endstate/endstate.py` now declare
every service a site cut off from the others needs at every site that
replicates: the replication core (Traefik, NetBird management, signal, Ory,
the Kratos self-service UI), the NetBird dashboard, the portal, the id console,
the onboarding engine and mesh DNS. No catalog row is founder-only. What a
joining site does not run yet is graded `GAP`, "declared, not yet built at
this site", with the gate row that closes it: I1 and I4 for the admin
surfaces, I3 for mesh DNS. A `GAP` is counted and printed and does not by
itself make the site red. Acceptance run with `--endstate` reports a row such a
service owns as `SKIPPED` with the same text instead of N/A. `goatbuilder
join-node` lists the gaps under "What <site> does not run yet", from the
catalog, before the services the site turns off (ADR 1233, #671).
