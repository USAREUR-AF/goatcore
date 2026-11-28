---
heading: For someone deploying
title: "Changed: oci-substrate stops when the admit-audit password variable is unset"
---

The admit-audit play of `oci-substrate-deploy.yml` reads `admit_audit_password`
from the deploy environment (`ADMIT_AUDIT_PASS`, or the variable
`admit_audit_password_env` names) with no fallback value. When the variable is
unset, the play stops at the admit-audit role's own "Require
admit_audit_password" check, with that check's message. Set the variable before
running the play with `oci_substrate_enabled` on.
