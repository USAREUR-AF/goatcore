# goatbuilder tests

| Kind | Where | Run |
|---|---|---|
| Unit | `#[cfg(test)]` modules in `src/` | `cargo test -p goatbuilder --bins` |
| Golden commands | `tests/cmd/*.toml`, driven by `tests/cli_golden.rs` (trycmd, as goat-cli) | `cargo test -p goatbuilder --test cli_golden`; regenerate with `TRYCMD=overwrite` |
| Whole flows | `tests/flows.rs`: the built binary on fixture copies and a detached worktree | `cargo test -p goatbuilder --test flows` (needs PyYAML + jsonschema) |
| Configuration compatibility (gate A2) | `tests/config_compat.rs` over `tests/compat/configs/`: each published release's configuration files, kept with the outcome that release's binary gave them | `cargo test -p goatbuilder --test config_compat`; record a new release with `tests/compat/record.py` ([its README](compat/README.md)) |
| Parity with the scripts | `ops/goatbuilder/parity-*.sh` (ADR 1201) | CI, `goatbuilder-pr.yml` |
| Hosts, registry, GitHub | the deployment e2e harness, `ops/e2e/` | CI, `deploy-e2e.yml` |

Golden cases are read-only and independent of the live pins. A case that changes a
fixture belongs in `flows.rs`, on a copy.

Defect classes the suites cover through the verbs' contracts:

- silent exit: every refusal prints a reason and exits non-zero
- target and scope: the declared environment, group and component only; a templated `hosts:` resolves to an extra var
- checkout and repository root: `-C` decides the tree read
- dry runs that hide a step: every decision and phase is printed
- tool errors read as data: only a digest is a digest; refusals name the real cause
- stale release records: `promote check` refuses them, `abandon` closes them
- non-idempotent re-runs: a second run on an unchanged tree changes nothing
- provenance of done: pending and unknown are never counted converged; a rollback appends a ledger row
- topology versus skew: an absent deviation is not drift
- secrets in output: names only, never values
