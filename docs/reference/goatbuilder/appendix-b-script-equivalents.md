# Appendix B. Verbs and their script equivalents

goatbuilder is the supported avenue; each script stays documented and working
([ADR 1201](../../adr/1201-two-avenues-scripts-documented-cli-supported.md)).
Where a parity gate exists, CI runs both sides in dry run on every change to
either and fails on any difference.

| Verb | Script | Parity gate |
|---|---|---|
| `validate` | `ops/goatbuilder/validate.py` | schema workflow |
| `doctor` | none; each script runs its own preflight | none |
| `status` | none | none |
| `deploy` | `ops/scripts/deploy-component.sh <c> --goatnet <g>` | `parity-deploy.sh` |
| `verify` | `ops/scripts/deploy-component.sh <c> --goatnet <g> --verify-only` | `parity-verify.sh` |
| `diff` | none | none |
| `survey` | none | none |
| `history` | `ops/scripts/rollback.sh <c> --list` | `parity-history.sh` |
| `plan`, `apply` | none | none |
| `adopt` | none | none |
| `rollback` | `ops/scripts/rollback.sh <c> [N]` | `parity-rollback.sh` |
| `release` | `ops/scripts/rc.sh` | `parity-release.sh` |
| `promote` | `ops/release/graduate.py` | `parity-promote.sh` |
| `pins` | `ops/pins/generate-pins.py` | `parity-pins.sh` |
| `fastlane` | `ops/scripts/fastlane.sh` | `parity-fastlane.sh` |
| `standup` | `ops/scripts/goat-standup.sh <site>` | `parity-standup.sh` |
| `teardown` | `ops/scripts/goat-teardown.sh` | `parity-teardown.sh` |
| `assay` | `ops/validation/generate-values.py <site>`, then `ops/validation/run-acceptance.py --env <site> --all-eligible --by-layer` | standup parity gate (phase 13) |
| `endstate` | `ops/scripts/goatnet-endstate.sh <site>` (both run `ops/goatbuilder/endstate/endstate.py`) | `parity-endstate.sh` |
| `join-node` | none; [02-first-federated-node.md](../../tutorials/02-first-federated-node.md) walks through the node-addition sequence it runs | none |
| `federate` | `ops/scripts/peering-standup.sh` | `parity-federate.sh` |
| `update` | none; the install sequence on `get.<site>.<zone>/goatbuilder/` (curl, `cosign verify-blob`, `sha256sum -c`, `install`) is the by-hand path | none |
| `secrets` | `ops/secrets/posture.py`, `ops/scripts/with-secrets` | the same module |
| `explain` | `goat explain` | the same catalog |
| `tutor` | none | none |

## Flag equivalents

| goatbuilder | Script |
|---|---|
| `--env <goatnet>` | `--goatnet <goatnet>` (`deploy-component.sh`) |
| `--dry-run` | `--dry-run`; `goat-standup.sh --plan` |
| `--from <step>` | `--from <step>` |
| `--parallel` | `goat-standup.sh --parallel` |
| `--ack <name>` | `--ack <name>` |
| `--site <site>` on `rollback` | `rollback.sh --site <site>` |
| `--offline` | `ops/scripts/fresh-source.sh --offline` |
| `--yes` | none; scripts do not prompt |
| `-o json` | `graduate.py status --json` and `review --json`; otherwise none |
