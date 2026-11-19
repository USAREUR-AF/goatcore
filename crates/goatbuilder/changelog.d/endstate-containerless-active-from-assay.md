---
heading: For someone deploying
title: "Fixed: `goatbuilder endstate` no longer grades a service with no container as running"
---

A catalog service with an empty `containers` list and no `oneshot` was graded
`active` PASS ("0 container pattern(s) running") whatever was true. Such a
service is now active only when its acceptance rows pass. It is FAIL when one
fails, and UNKNOWN when none ran or the assay was not given. It is not also
graded `healthy` FAIL "not running", since it has no container to be unhealthy.
No service in today's catalog has an empty container list, so no grade changes
until one is declared that way.
