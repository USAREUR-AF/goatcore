// _mock.jsonnet — Kratos OIDC data mapper for the federation proof harness.
// scheme=native (test upstream). See ./README.md for the four rules.
//
// The mapper sets BOTH the scalar identity (traits) AND the external-subject binding
// (metadata_public). The binding is metadata, NOT a trait: Kratos coerces a nested
// array *trait* to {} during oidc-registration (proven in ops/identity/federation-proof/),
// but identity.metadata_public passes through untouched — so one mapper does it all,
// no after-registration webhook needed.
//
// Kratos surfaces only a few STANDARD claims at the top of `claims`; the full id_token
// (incl. non-standard claims like `groups`) is under claims.raw_claims — read from there
// so nothing is silently dropped (proven in the harness).
local c = std.extVar('claims').raw_claims;
local client_id = 'goat-proof';   // fixed per file: goat's RP client_id at this issuer

// Rule 2: explicit external-group -> goat role-family allow-map. Unlisted grants nothing.
local group_map = { 'goat-admins': 'ADMINS', 'goat-devs': 'DEVELOPER' };
local ext_groups = if std.objectHas(c, 'groups') then c.groups else [];
local mapped_groups = std.filter(
  function(g) g != null,
  std.map(function(g) (if std.objectHas(group_map, g) then group_map[g] else null), ext_groups)
);

{
  identity: {
    traits: {
      email: c.email,
      name: if std.objectHas(c, 'name') then c.name else c.email,
    },
    // Binding + assurance ride metadata_public (admin-managed, descriptor-side), never traits.
    metadata_public: {
      // GHSA-9v3v: login groups live in admin-managed metadata_public (the
      // migration marker); never in self-editable traits.
      groups: mapped_groups,
      external_subjects: [{
        issuer: c.iss,
        client_id: client_id,
        subject: c.sub,          // NEVER the identity key — a mapped attribute
        scheme: 'native',
        credential_type: 'oidc',
        last_assurance: {
          ial: 'ial1',
          aal: if std.objectHas(c, 'acr') && c.acr == 'aal2' then 'aal2' else 'aal1',
          fal: 'fal1',
          acr: if std.objectHas(c, 'acr') then c.acr else '',
          boundary_il: 0,
        },
      }],
    },
  },
}
