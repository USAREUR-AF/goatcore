# Browser session authentication

## Status and scope

**Built in this change:** portal and SNITCH browser session handlers, frontend
migration and regression coverage. A merge or a passing fixture does not establish
that either service has been deployed. This contract covers the two browser
surfaces; bearer API clients retain their existing authentication path.

## Credential flow

The browser exchanges an API credential for a random, opaque session identifier.
The credential is held in service memory rather than browser Web Storage. The
cookie is host-only, HttpOnly and SameSite Strict; it is Secure except at literal
localhost or a loopback IP address. Session lifetime is eight hours and the
process holds at most 4096 entries. Expired entries are removed; a replacement
invalidates the prior session supplied by that browser.

Cookie-authenticated mutations require an explicit matching Origin. Underlying
credentials and roles are checked again on authenticated requests. An invalid
explicit bearer credential cannot fall back to a cookie. Logout invalidates the
session and expires its cookie. Responses to session exchange are not cacheable;
portal cache revalidation varies on Cookie as well as Authorization and encoding.

Sessions are process-local. A restart or failover requires sign-in again, and
concurrent replicas require session affinity. Session persistence and a shared
session service are not built by this change.

## Browser-facing transport

Non-loopback browser origins require trusted HTTPS, including private mesh names
and mesh IP addresses. Mesh encryption does not change browser cookie rules.
Plain HTTP sign-in on those origins is refused; there is no deployment flag that
turns off the Secure attribute. Local development and an operator-managed SSH
forward may use HTTP on localhost or a loopback IP.

TLS can terminate at a reverse proxy. The backend hop may remain HTTP while the
browser sees HTTPS. Preserve the browser-facing host through the proxy so the
Origin comparison remains exact. A forwarded-protocol header is not used to grant
an HTTP cookie exception.

## Deployment and verification

Deliver each service's embedded frontend and API together. Before moving image
pins, inspect actual dashboard URLs and provide trusted HTTPS for non-loopback
HTTP routes. This source change does not inventory or migrate deployed URLs.
Verify sign-in, authenticated navigation, downloads/uploads, streams and sign-out
through the real ingress. Verify that credential revocation and role changes take
effect without waiting for session expiry. Do not disable certificate validation
to make a private CA work; install its trust root through the deployment workflow.

Regression tests live in each service's browser session and browser auth tests;
the portal browser-session Playwright spec checks the frontend exchange and
navigation against a mocked API. Those browser fixtures do not prove production
TLS or reverse-proxy configuration. Database-backed tests require an explicitly
provided disposable test database; skipped tests are not acceptance evidence.
