---
heading: For someone deploying
title: "Fixed: deploy applies the release ladder (GB-373) as deploy-component.sh does"
---

`goatbuilder deploy`, dry run and real, now judges every member group's
declared digest against the release ladder (ADR 1120) before the secrets, the
transport or any host, with `ops/release/ladder_gate.py`, the module
`deploy-component.sh` calls. It refuses in the script's words when the rung
below pins another digest, or when no rung below pins the component at all
(fail-closed), and passes the lowest rung, a goatnet that is not a live rung,
and an open hotfix record. When the rung below pins the digest, a real run reads
that rung's ledgers (`--verify-only`) and refuses unless it converged; a dry
run names that read. Before, a dry run said it would deploy where the script
refused, and a real run met only `ansible-preflight.sh`'s gate for direct
plays, which does not read the rung below's ledgers.

New flags `--skip-rung --reason "<why>"`, with `--ack "<operator>"`, take the
recorded exception: the gate refuses either half alone, and the reason and the
ack go into the deploy ledger row (`ladder_skip`). `--reason` without
`--skip-rung` is refused. A gate that cannot run refuses. `--verify-only` is not
judged, and `release --dry-run` is not judged either, as `rc.sh`'s dry run runs
no deploy (#1187).
