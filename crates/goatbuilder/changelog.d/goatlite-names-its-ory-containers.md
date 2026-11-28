---
heading: For someone deploying
title: "Fixed: a goatlite names its Ory containers as its compose does"
---

The Ory replication roles wait for the Kratos and Hydra containers by name,
`kratos-a` and `hydra-a` unless a site says otherwise, and a goatlite's compose
names them `mgmt-host-kratos-1` and `mgmt-host-hydra-1`. A goatlite that grew a
second site waited for containers that do not exist. A goatlite now renders
`ory_spock_kratos_container` and `ory_spock_hydra_container` with its own names.
