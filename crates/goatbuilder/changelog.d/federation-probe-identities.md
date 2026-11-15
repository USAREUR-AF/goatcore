---
heading: For someone deploying
title: Added: phase 14's probe identities, issued from your own CA for each run
---

`ops/federation/standup/probe-identities.sh` is the launcher to give as
`FED_PROBE_LAUNCHER_OURS`. It gives each of your probes its identity and
hands it to `probe-launcher.sh`. The agreed probe holds a copy of your
outward leaf. The forged-name probe and the impersonation probe each hold a
leaf issued from your own peering CA for that run, by
`mint-peering-leaves.sh --probe <run id>`. A leaf bearing the partner's
core's name (`FED_PROBE_PARTNER_CORE_CN`, never defaulted) is issued only
with `--impersonate-for-run` naming the same run. Each probe leaf is a
client certificate only, marked `urn:goat:acceptance-probe:<run id>`, valid
for `FED_PROBE_TTL` plus 15 minutes, and recorded in your CA's
issued-certificates record (`FED_PROBE_ISSUED_RECORD`: point it at the record
your CA's other issuances go to, or a file kept with it). It never goes into a
trust file. On the machine running phase 14, the identity sits in a private
folder on memory-backed storage only while the start copies it to the edge
host. The folder is deleted on any exit, and the deletion is checked and
recorded in the run's probe manifest as `key_deleted_driver`. A copy that
was not deleted fails `cleanup/probes`. Before, you issued these identities
yourself.
