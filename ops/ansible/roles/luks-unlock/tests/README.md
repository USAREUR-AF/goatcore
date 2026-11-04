# luks-unlock role tests

## Coverage level

These layers gate this role at PR-time:

| Layer | What it catches | Where |
|---|---|---|
| `syntax-check.sh` (this dir) | YAML + Jinja2 parse errors in `tasks/main.yml` + the test playbook | local + CI |
| `check-register-uniqueness.sh` (workspace) | F-075-class within-play `register:` name collisions | local + CI |
| `clevis-bind-one-line.yml` (this dir, run by `syntax-check.sh`) | a `clevis luks bind` in `tasks/installer-luks.yml` whose pin is not on its shell line | local + CI |

Run locally:

```bash
./syntax-check.sh
```

CI runs both via [`.github/workflows/ansible-syntax-check.yml`](../../../../../.github/workflows/ansible-syntax-check.yml).

## What's NOT covered here

LUKS keyslot operations (TPM2 enrollment, FIDO2 enrollment, Tang binding,
cold-boot unseal) are exercised by:

- [`ops/scripts/cold-boot-drill.sh`](../../../../scripts/cold-boot-drill.sh) — Block 65A.1 + 65D drill harness against a real Orin
- F-058 and F-075 document the surfaced bugs the structural lint plus the drill caught.

A privileged-container fixture with a loop-back LUKS device + simulated
TPM2 + soft FIDO2 (e.g. softhsm2-style) would close the structural-vs-
behavioral gap and is queued; it is not a prerequisite for the lint to
catch the F-075 class.
