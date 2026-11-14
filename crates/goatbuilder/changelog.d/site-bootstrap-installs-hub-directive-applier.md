---
heading: For someone deploying
title: Added: site bootstrap installs the hub-directive applier at Spock sites that name a hub
---

`site-bootstrap.yml` now applies the `spock-hub-directives` role (ADR 1205)
at each site that replicates with Spock and sets `goatnet_hub_site`. A Spock
site that names no hub gets a message and no applier. The play carries the
tag `hub-directives` beside its phase tag, so it can be run alone. (#596)
