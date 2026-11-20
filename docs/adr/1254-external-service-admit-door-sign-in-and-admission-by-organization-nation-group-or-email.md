# ADR 1254 — An external service can let goat sign people in and admit them by organization, nation, group or e-mail

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `ops/ansible/external-service.yml`, `ops/door-admit/`, roles `door-admit` and `oauth2-proxy-surface`, the mgmt-host Traefik template |
| Amends | [ADR 1178](1178-external-service-golden-path.md) (a new `door` value, `admit`, and `host_class: INFRA`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) (the boundary monitor is the first user), design [`external-service-golden-path.md`](../design/external-service-golden-path.md) |

## TL;DR (for a layperson)

A goatnet can now publish any small web page under its own name and say, in one line of its declaration, who may open
it: for example everyone in one organization and its sub-organizations, everyone from one nation's organizations, a
role group, or a named list of people. goat signs the visitor in with the goatnet's own sign-in and checks the rule
before the page sees the request. The rule is written once per page and enforced the same way for every page, so a
new page with a new audience is a new declaration rather than new code.

## Context

The external-service golden path (ADR 1178) gives a service a mesh-only name, a certificate, a DNS record and a status
row, but leaves identity to the service: `door` records what the service's own fence decides. A small page with no
fence of its own, such as a boundary node's monitor, was therefore either open to every mesh member or reachable only by
platform admins. Neither matches an audience defined by organization or nation, and the mesh cannot express one:
organization is an identity attribute, never a NetBird group (the identity grammar).

The sign-in token already carries the organization: the consent patch forwards the identity's `aff` block
(`{id, kind, nation}`) into the ID token. The org registry (`ops/identity/orgs/<env>.yaml`, synced from the console)
holds each organization's parent and nation.

## Decision

1. **A new door value, `admit`.** The declaration names an `admit:` rule and the two env vars that hold the door's
   secrets:

   ```yaml
   door: admit
   admit:
     orgs: [<org id>]        # the organization and every descendant in the registry
     nations: [<ISO alpha-3>]  # every organization with that nation, and their descendants
     groups: [<group>]       # role groups, passed through
     emails_env: <ENV VAR>   # optional comma-separated allowlist, resolved from the secret store at run time
   door_secrets: {client_secret_env: <ENV VAR>, cookie_secret_env: <ENV VAR>}
   ```

2. **Enforcement at the edge, by goat.** `external-service.yml` sets up two pieces and points the service's Traefik
   router at them.
   - **The sign-in.** `oauth2-proxy-ext-<name>` is the existing `oauth2-proxy-surface` role. It provisions its own
     Hydra client `ext-<name>` and passes the ID token upstream.
   - **The check.** `door-admit-<name>` is standard-library Python on the pinned `python:3.12-alpine`, publishing no
     port. It admits when the token's `aff.id`, a group or the e-mail matches the rule. It forwards without the token,
     and refuses with a page that names the rule. It logs each decision as one line, and never logs the token.

   The request then reaches `backend_url` over the mesh.
3. **The rule is expanded at deploy time** by `ops/door-admit/expand_rule.py`. The output is concrete organization ids,
   including aliases. The run fails when a named organization or nation matches nothing, so a typo cannot admit nobody
   in silence. A registry change takes effect on the next run.
4. **`host_class: INFRA`.** A declaration may name a goatnet's own substrate host, such as a boundary node, with a
   written `host_class_reason`. The edge reaches it INFRA to INFRA. Participants reach only the edge's port 443.

## Alternatives considered

- **A mesh rule per audience.** It is impossible by design: affiliation is never a NetBird group, and a group per
  audience would put identity into the network plane.
- **oauth2-proxy's own group gate.** It checks one flat claim. It cannot follow the registry's parent tree or select by
  nation without putting affiliation into `groups`, which the identity grammar forbids.
- **Each service implements its own sign-in.** That is the status quo. Every small page would re-implement OIDC and the
  rule, and drift.
- **One shared admission service for all doors.** Fewer containers, but one fault refuses every service at once.
  A check per service keeps a bad rule local to its service.

## Consequences

- A person is admitted only if their identity records an organization. An identity with no `aff` passes only by group
  or by e-mail. Recording organizations is the identity admins' work, done through the console.
- Each admitted service adds two small containers on the mgmt host and one Hydra client.
- The check trusts the token because only the door can reach it, on the mgmt Docker network. Any container on that
  network is the mgmt host's own trust, as for every other sidecar there.
