---
heading: For someone deploying
title: "Added: a component can run on one host of each group, named by a group variable"
---

A component may declare `one_host_var: <variable>`. Each member group's
group_vars then set that variable to the inventory name of the one host that
runs it. `goatbuilder deploy` and `deploy-component.sh` verify a ledger row on
that host only, and so does `verify`; `diff` and `status` skip the group's other
hosts. When the variable is unset, empty or templated, or names a host outside
the group, verification refuses: it cannot know which host must answer, so it
does not report the group converged. Both resolvers refuse a `one_host_var:` that
is not a variable name, and one declared beside `single_homed_role`.
