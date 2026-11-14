---
heading: For someone contributing
title: Fixed: an in-process federation verb no longer writes the process environment
---

When goatbuilder runs a federation verb in process, phase 3's `init` used
to read `DOCKER_HOST` and `FED_ROUTER_CONTAINER` from the process
environment, so goatbuilder wrote both into it with `std::env::set_var` for
the verb's duration, which every thread shares. The values are now passed
to the measurement (`derive::RouterEnv` in goat-federation), and
`DOCKER_HOST` reaches the container runtime's commands as their own
environment. `goat federate init` reads its environment as before. A test
refuses any `set_var` or `remove_var` under `crates/goatbuilder/src/federate/`
(#641).
