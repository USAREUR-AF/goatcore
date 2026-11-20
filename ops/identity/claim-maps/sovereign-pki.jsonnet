// sovereign-pki.jsonnet — Kratos OIDC data mapper for card sign-in through the
// icam-mtls-proxy (ADR 1209). GENERATED from ops/identity/sovereign-pkis.yaml by
// ops/identity/sovereign-pkis-render.py claim-map; CI refuses drift. Edit the registry.
//
// The proxy hands Dex X-Remote-User = "<pki>:<identifier>"; Dex carries it as
// preferred_username. This mapper splits it, refuses an unknown profile, and records
// the binding in metadata_public.external_subjects with pki + pki_subject. A profile
// that names an alias claim (us-dod: edipi) gets it filled too, for readers that still
// look for the old name. Nothing here reads or sets a nationality (ADR 1171).
local c = std.extVar('claims').raw_claims;
local client_id = 'goat-zagros';   // fixed per file: our RP client_id at the proxy's Dex

// One row per live profile in the registry.
local profiles = {
  'us-dod': { scheme: 'dod-piv', identifier_name: 'EDIPI', alias: 'edipi', email_domain: 'mil' },
};

local user = if std.objectHas(c, 'preferred_username') then c.preferred_username else c.sub;
local sep = std.findSubstr(':', user);
local pki = if std.length(sep) > 0 then std.substr(user, 0, sep[0]) else '';
local pki_subject = if std.length(sep) > 0 then std.substr(user, sep[0] + 1, std.length(user) - sep[0] - 1) else '';
// Fail closed: a username without a known profile prefix mints nothing.
local profile = if std.objectHas(profiles, pki) && pki_subject != '' then profiles[pki]
                else error 'card sign-in: unknown PKI profile or empty identifier in ' + user;

{
  // The card's own e-mail when it carries one; otherwise an address that cannot be
  // delivered (the trait is required and must be unique). us-dod keeps <EDIPI>@mil.
  local email = if std.objectHas(c, 'email') && std.length(std.findSubstr('@', c.email)) > 0
                then c.email else pki_subject + '@' + profile.email_domain,
  identity: {
    // Rule 2: a card carries no goat authorization — role/affiliation are assigned locally.
    traits: {
      email: email,
      name: if std.objectHas(c, 'name') then c.name else user,
    },
    metadata_public: {
      // GHSA-9v3v: login groups live in admin-managed metadata_public (the
      // migration marker); never in self-editable traits.
      groups: [],
      external_subjects: [{
        issuer: c.iss,
        client_id: client_id,
        subject: user,              // "<pki>:<identifier>" — a mapped attribute, never the key
        pki: pki,
        pki_subject: pki_subject,
        identifier_name: profile.identifier_name,
        scheme: profile.scheme,
        credential_type: 'x509-card',
        last_assurance: {
          ial: 'ial3',            // in-person issuance of a sovereign card
          aal: 'aal3',            // hardware, phishing-resistant
          fal: 'fal2',
          acr: if std.objectHas(c, 'acr') then c.acr else '',
          boundary_il: 5,
        },
      } + (if profile.alias != null then { [profile.alias]: pki_subject } else {})],
    },
  },
}
