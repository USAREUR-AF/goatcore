// entra.jsonnet — scheme=entra, authority=industry, boundary_il=5.
// DoD Microsoft Entra ID (Gov). Person id = `oid` (per-tenant stable object id);
// `sub` is per-app (pairwise), so we key the subject on `oid`. IL2/4/5 (IL6 = Gov
// Secret). Conditional access supplies phishing-resistant MFA / CBA. See ./README.md.
//
// Sets traits + metadata_public.external_subjects. Reads claims.raw_claims.
local c = std.extVar('claims').raw_claims;
local client_id = 'api://goat-zagros';   // fixed per file: our Entra app registration id

// Rule 2: Entra app roles are mapped ONLY through this explicit allow-map. Unlisted
// roles grant nothing. Populate per deployment agreement.
local role_map = {
  'goat.admin': 'ADMINS',
  'goat.developer': 'DEVELOPER',
  'goat.observer': 'OBSERVER',
};
local ext_roles = if std.objectHas(c, 'roles') then c.roles else [];
local mapped_groups = std.filter(
  function(g) g != null,
  std.map(function(r) (if std.objectHas(role_map, r) then role_map[r] else null), ext_roles)
);

// AAL from the auth-method-reference. Hardware / CBA (x509) => aal3; MFA => aal2.
local amr = if std.objectHas(c, 'amr') then c.amr else [];
local has(m) = std.length(std.find(m, amr)) > 0;
local aal = if has('x509') || has('hwk') || has('fido') then 'aal3'
            else if has('mfa') || has('otp') then 'aal2' else 'aal1';

{
  identity: {
    traits: {
      email: if std.objectHas(c, 'email') then c.email
             else if std.objectHas(c, 'preferred_username') then c.preferred_username else c.oid,
      name: if std.objectHas(c, 'name') then c.name else c.oid,
    },
    metadata_public: {
      // GHSA-9v3v: login groups live in admin-managed metadata_public (the
      // migration marker); never in self-editable traits.
      groups: mapped_groups,
      external_subjects: [{
        issuer: c.iss,
        client_id: client_id,
        subject: c.oid,          // per-tenant stable object id (not `sub`, which is per-app)
        scheme: 'entra',
        credential_type: 'oidc',
        last_assurance: {
          ial: 'ial2',           // DoD Entra accounts are proofed
          aal: aal,
          fal: 'fal2',
          acr: if std.objectHas(c, 'acr') then c.acr else '',
          boundary_il: 5,
        },
      }],
    },
  },
}
