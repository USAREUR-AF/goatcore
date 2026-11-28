---
heading: For someone deploying
title: "Changed: join-node refuses a core joiner whose founder's ACL does not let INFRA reach INFRA"
---

A core or phone node of a joining site replicates with the founder's
management host over the mesh, and a goatlite's first ACL has no rule between
management hosts, because it has one. join-node now refuses such a join before
the node enrols (GB-442) and names the rule to add to the founder's rule
intents (INFRA to INFRA, any protocol, ADRs 1131 and 1114), how to regenerate
the matrix, and the standup phase that applies it.
