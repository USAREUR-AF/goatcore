---
heading: For someone deploying
title: Added: `join-node` names what the site does not run, and why
---

The dry-run plan and the summary of a real `join-node` run end with a section
headed "What <site> does not run". It lists every catalog service the
end-state model (`endstate.py model <site>`) marks absent at the site, each
with the model's reason. A service the site turns off reads its flag, for
example "`isitup_enabled` is false in group_vars". At a joining site, the
NetBird dashboard, the portal, the id console and the onboarding engine read
"single-homed at the founder (ADR 1084 §D3); every site once ADR 1165 lifts
it for this class", followed by the declaration that keeps each one off.
`goatbuilder endstate` gives an absent service the same reason, so the plan
and the grade cannot disagree. When the model cannot be read, the section
says so in one line and lists nothing; the run is not stopped. (#612)
