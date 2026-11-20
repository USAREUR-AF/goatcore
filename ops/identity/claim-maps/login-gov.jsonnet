// login-gov.jsonnet — scheme=login-gov, authority=industry, boundary_il=2.
// login.gov OIDC. `sub` is PAIRWISE (a different UUID per relying-party), keyed by
// (issuer, client_id, subject), never the identity key. Public / IL-limited IdP —
// good for civil/partner IAL2 proofing, NOT the high-IL plane. See ./README.md.
//
// Sets traits (scalar identity) + metadata_public.external_subjects (the binding).
// Reads claims.raw_claims (Kratos nests the full id_token there). See _mock.jsonnet.
local c = std.extVar('claims').raw_claims;
// Fixed per file: goat's own RP client_id at login.gov (mirror providers[].client_id).
local client_id = 'urn:gov:gsa:openidconnect.profiles:sp:sso:goat:zagros';

// login.gov signals IAL via `acr`: .../ial/2 (or an AAL2 hspd12 variant for PIV/CAC).
local acr = if std.objectHas(c, 'acr') then c.acr else '';
local is_ial2 = std.length(std.findSubstr('ial/2', acr)) > 0;
local is_piv  = std.length(std.findSubstr('hspd12', acr)) > 0;

{
  identity: {
    // Rule 2: login.gov emits no authorization groups we trust — role is a local decision.
    traits: {
      email: c.email,
      name: if std.objectHas(c, 'given_name') && std.objectHas(c, 'family_name')
            then c.given_name + ' ' + c.family_name else c.email,
    },
    metadata_public: {
      // GHSA-9v3v: login groups live in admin-managed metadata_public (the
      // migration marker); never in self-editable traits.
      groups: [],
      external_subjects: [{
        issuer: c.iss,
        client_id: client_id,
        subject: c.sub,
        scheme: 'login-gov',
        credential_type: 'oidc',
        last_assurance: {
          ial: if is_ial2 then 'ial2' else 'ial1',
          aal: if is_piv then 'aal3' else 'aal2',
          fal: 'fal2',              // login.gov encrypts PII assertions to the RP
          acr: acr,
          boundary_il: 2,
        },
      }],
    },
  },
}
