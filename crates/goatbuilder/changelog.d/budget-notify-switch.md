---
heading: For someone deploying
title: "Changed: budget-notify runs only where budget_notify_enabled is set"
---

`budget-notify-deploy.yml` has a switch, `budget_notify_enabled`, off by
default. A site that runs the notifier sets `budget_notify_enabled: true` in its
host group's group_vars; until it does, a run of the play says in one line that
the switch is off and changes nothing. With the switch on the play runs as
before.
