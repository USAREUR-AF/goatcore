# Relay enrollment isolation

Status: built. The shared rathole target and the standalone client-template
fallback both use `127.0.0.1:8443`, matching the management stack's loopback
binding for the enrollment entrypoint, and so do the hand-deployed
`ops/relay/client.toml` and `ops/relay/setup-client.sh`. Websecure remains separate. Explicit
custom target overrides are retained for custom stacks; they must provide
an equivalent isolation boundary. Sites overriding the target to port 443
must review that override before rollout. This does not repair the broader
Docker-source allowlist policy on other ingress paths.

The `relay-refuses-mesh-only-hosts` acceptance row checks every declared relay
IP, without requiring an inventory label that could silently skip it. It first
requires HTTP 401 from `/api/groups` on the public enrollment hostname, then
requires HTTP 404 for the dashboard root, authentication session-check path
and OAuth discovery path with their protected Host headers on the same
connection destination. Using real API paths avoids treating an exposed
service's ordinary root-page 404 as isolation. TLS still verifies the enrollment name; curl
uses the relay IP directly and bypasses HTTP proxies. Redirects are not
followed. A timeout, TLS error or missing tool is UNKNOWN. A broken positive
control or a response other than the required routing refusal fails.

This is a compound probe with its positive control inside each relay check;
it must not use `must_refuse` verdict inversion. Tests cover failed controls,
reachable protected routes, TLS/transport errors, explicit target overrides,
and the default listener binding. Live enrollment and isolation testing on a
canary deployment remains required before rollout.

The mesh-only allowlists admit 100.64.0.0/10 and the `mgmt` Docker network's own
subnet, read from the live network by `ops/ansible/tasks/mgmt-network-subnet.yml`.
They no longer admit all of 172.16.0.0/12, which let in any off-host client
numbered inside it.

The websecure publish is IPv4-only (`0.0.0.0:443:443`): a `[::]` publish is served
by docker-proxy, which also delivers off-host clients from the bridge gateway.
`tests/test_mesh_allowlist_ingress.py` fails if that shape or any rathole target
reverts to websecure.

Run: `python3 -m unittest discover -s tests -p 'test_relay_enrollment_isolation.py'` and
`python3 -m unittest discover -s tests -p 'test_mesh_allowlist_ingress.py'`.
