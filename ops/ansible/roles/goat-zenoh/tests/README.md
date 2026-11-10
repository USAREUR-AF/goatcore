# goat-zenoh role tests

| Tier | File | Coverage |
|------|------|----------|
| Static | [`playbook.yml`](playbook.yml) + [`syntax-check.sh`](syntax-check.sh) | Parse-only smoke per Block 50H. Catches template syntax + task-key typos. Wired into `.github/workflows/ansible-syntax-check.yml`. |
| Behavioral | the trial fabric's `smoke.sh` | End-to-end PASS/FAIL against a running router. Cannot fire until Track A delivers vendor certs. Evidence to `docs/reports/<date>-zenoh-trial-fabric-smoke.jsonl`. |
| Behavioral (live) | Operator-fired deploy against a goatnet's router host | First-real-run validation that the role's pre-flight assert fires correctly, the rendered configs are valid JSON5, the router accepts mTLS connections, the ACL deny path is observable in audit-logger output. |

Why no `inspec` / `molecule` here: the role's behavior is exercised through
the trial fabric's smoke harness directly, in the same way the parent
zenoh role's behavior is exercised through `integdev-bootstrap.yml` Phase 8.
Re-implementing the same checks inside molecule would duplicate the
smoke harness without the cert + container shape that mTLS testing
needs.
