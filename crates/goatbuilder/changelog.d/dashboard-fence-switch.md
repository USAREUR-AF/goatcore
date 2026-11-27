---
heading: For someone deploying
title: "Changed: dashboard-fence ends the everyday fence at once on a host whose switch is off"
---

`dashboard-fence-deploy.yml` failed on a host where
`dashboard_oauth2_proxy_enabled` was off, because its role asserted the switch.
Its everyday fence now ends at once there: it says so in one line and changes
nothing. Traefik sends the dashboard through the fence only when the switch is
on, so a site without it is left as it is. With the switch on it runs as before.
The admin fence keeps its own switch (`netbird_dashboard_split_enabled`) and
still refuses an admin address whose fence is off. This lets a verb run the
play after `dashboard-deploy.yml` on every site.
