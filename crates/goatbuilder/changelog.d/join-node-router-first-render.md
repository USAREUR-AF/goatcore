---
heading: For someone deploying
title: "Fixed: a joining site's data router renders its slots unassigned instead of refusing"
---

In per-uuid-slot-pool mode the router's role refuses to render without the
portal's slot fragment, and a joining site runs no portal yet (gate row I4), so
join-node phase 13 stopped at the site's data router. join-node now asks for the
first-render check that standup phase 9 uses: a router that serves no holder
renders every slot unassigned, granting nobody, and holders keep using the
founder's router until the site has a portal.
