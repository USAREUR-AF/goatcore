---
heading: For someone deploying
title: "Added: standup runs the dashboard's fences in phase 10"
---

`goatbuilder standup` and `ops/scripts/goat-standup.sh` now run
`dashboard-fence-deploy.yml` in phase 10 on a site whose
`group_vars/<site>_hosts/vars.yml` sets `dashboard_oauth2_proxy_enabled` or
`netbird_dashboard_split_enabled`. It runs after the dashboard on a LAN site and
after the OIDC clients on an internet site. A site that sets neither, such as a
goatlite, gets no new step. A site that sets
`dashboard_oauth2_proxy_enabled` gets its everyday fence, and Traefik already
routes the dashboard through that fence from phase 7. Before this change the
dashboard was not served until a person ran the fence play. A site that sets
`netbird_dashboard_split_enabled` also gets its admin fence. Each fence ends at
once where its switch is off. `teardown` stops the two fence containers, and
their Hydra clients stay.
