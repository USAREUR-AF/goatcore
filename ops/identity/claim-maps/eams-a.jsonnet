// eams-a.jsonnet — scheme=dod-piv, authority=doctrinal, boundary_il=5.
// DoD PKI / PIV / CAC. Person id = EDIPI. Two toggle modes feed the SAME claims here:
//   - mode: upstream_oidc  — an EAMS-A / enterprise broker validates the CAC and
//     delivers EDIPI as an OIDC claim (edipi / sub).
//   - mode: mtls_proxy     — the local icam-mtls-proxy (ADR 1209) terminates x509 mTLS and
//     reads the EDIPI from the SAN otherName UPN (<EDIPI><persona code>@mil); its claim
//     map is sovereign-pki.jsonnet (generated), which also fills `edipi` for us-dod.
//   - mode: keycloak_frontend (path #3) — Keycloak's x509 authenticator asserts EDIPI.
// EDIPI is reissued / non-unique over time — the doctrinal reason it is NEVER the key.
//
// Sets traits + metadata_public.external_subjects. Reads claims.raw_claims. See ./README.md.
local c = std.extVar('claims').raw_claims;
local client_id = 'goat-zagros';   // fixed per file: our RP client_id at the CAC broker

// EDIPI source, in order: an explicit `edipi` claim (EAMS-A broker / Keycloak mapper),
// then `preferred_username` (Keycloak's username = EDIPI; Dex authproxy carries the
// mTLS-proxy X-Remote-User there), then `sub`. All three CAC modes land here — proven
// live in ops/identity/keycloak-cac-proof/ (#3) and ops/identity/mtls-proxy-proof/ (#2).
// The icam-mtls-proxy (ADR 1209) now hands "us-dod:<EDIPI>" and its own claim map is
// sovereign-pki.jsonnet; a provider still pointed here with that username gets the
// prefix stripped so the EDIPI stays an EDIPI.
local strip_pki = function(u) if std.startsWith(u, 'us-dod:') then std.substr(u, 7, std.length(u) - 7) else u;
local edipi = if std.objectHas(c, 'edipi') then c.edipi
              else if std.objectHas(c, 'preferred_username') then strip_pki(c.preferred_username)
              else c.sub;

{
  // Use the upstream email only if it's actually an address — the mtls_proxy path (Dex
  // authproxy) defaults the email claim to the username (the EDIPI) when the cert carries
  // no rfc822 email, so fall back to <edipi>@mil rather than an invalid-email trait.
  local email = if std.objectHas(c, 'email') && std.length(std.findSubstr('@', c.email)) > 0
                then c.email else edipi + '@mil',
  identity: {
    // Rule 2: CAC carries no goat authorization — role/affiliation assigned locally.
    traits: {
      email: email,
      name: if std.objectHas(c, 'name') then c.name else edipi,
    },
    metadata_public: {
      // GHSA-9v3v: login groups live in admin-managed metadata_public (the
      // migration marker); never in self-editable traits.
      groups: [],
      external_subjects: [{
        issuer: c.iss,
        client_id: client_id,
        subject: edipi,
        scheme: 'dod-piv',
        credential_type: if std.objectHas(c, 'edipi') then 'oidc' else 'x509-piv',
        last_assurance: {
          ial: 'ial3',            // CAC issuance = in-person proofing
          aal: 'aal3',            // hardware, phishing-resistant
          fal: 'fal2',
          acr: if std.objectHas(c, 'acr') then c.acr else '',
          boundary_il: 5,
        },
      }],
    },
  },
}
