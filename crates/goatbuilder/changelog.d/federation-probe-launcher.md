---
heading: For someone deploying
title: Added: a probe launcher for phase 14's acceptance probes on your own edge hosts
---

`ops/federation/standup/probe-launcher.sh` starts, stops and checks the
probe sessions phase 14's refusal rows need, on a real edge host reached over
ssh with sudo (`FED_PL_SSH`) or on the edge host itself (`FED_PL_LOCAL=1`).
Give it as `FED_PROBE_LAUNCHER_OURS` in a bilateral session, with your own
identities in `FED_PL_DIR`, or as `FED_PROBE_LAUNCHER` when you run both
goatnets. Its image is the edge host's `zenoh_router_image` from
`FED_PL_PINS`, pinned by digest; its REST port binds to `FED_PL_REST_IP`
only; its files live under `/run/goat-probe`, the key mode 0600, and the
key is deleted on stop and on any exit of a start that did not complete,
each deletion checked on the edge host and recorded in the run's probe
manifest as `key_deleted`; a key that could not be deleted fails
`cleanup/probes`. It carries
no identity and mints nothing: you supply the certificates. A missing or
malformed setting is X-PROBE-001, pins without a digest X-PROBE-002, and a
probe that does not answer X-PROBE-003. Before, no launcher shipped for a
deployment (#919).
