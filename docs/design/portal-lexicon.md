# Portal display lexicon — one image, goatnet-appropriate words

The goat-zenoh-portal is one Docker image serving every goatnet. Its frontend
shipped with nova-specific wording baked into the embedded HTML/JS:
"Zenoh Trial — Vendor Portal", a "Vendors" catalog, a hardcoded link to the nova
panoscope host. On ZAGROS — whose participants are organizations (commands,
units, and vendors alike), not vendors — that wording misframes the system.

The lexicon is a **display-only** vocabulary the portal substitutes at serve
time, so nova-a keeps its trial-fabric framing and ZAGROS renders "ZAGROS Fabric
Portal" with "org" as the top-level entity, from the same image.

## Invariant — display only, never a machine path

This is render-time substitution. It changes the words a human reads and
nothing else. Untouched: the API routes (`/api/vendors`, `/api/vendors/{v}/…`),
the SQLite registry's vendor-keyed rows, the cert *identity* layer (URI-SAN /
CN / DNS-SAN — per-goatnet machine values set by the role, **not** lexicon
tokens; see [Cert identity layer](#cert-identity-layer--per-goatnet-machine-not-display)),
the UUID slot anchor that is the Zenoh-wire identity, the ACL, audit records,
and any `--json` output. This is the same discipline as the
[slot-alias display layer](slot-alias-display-layer.md): a human projection
over a machine anchor, where the anchor never moves. A lexicon change can never
break routing, access, or audit, because it never reaches them.

## Mechanism

Two coupled surfaces, both driven by the resolved `config.Lexicon`:

1. **Construction-time HTML substitution.** The embedded HTML carries
   `__LEX_*__` tokens. `staticHandler` (`internal/api/static.go`) walks the
   embed once when the server is built and applies a single non-overlapping
   `strings.Replacer` pass to every `.html`, storing the substituted bytes in
   the asset table it serves from.

   This ran per request until 2026-08-12. It moved to construction time as part
   of precomputing each asset's ETag and gzip encoding (the frontend was being
   re-downloaded uncompressed on every navigation — see
   [`portal-static-delivery.md`](portal-static-delivery.md)). The property that
   made serve-time attractive is preserved exactly: the lexicon is still
   resolved from *this process's* `PORTAL_LEX_*` env, so one image still serves
   every goatnet and nothing is baked at build time. What changed is only how
   often the replace runs — the embed FS is immutable for the life of the
   process, so there is no cache to go stale and no path by which a token can
   leak.
2. **`GET /api/branding`.** Serves the resolved lexicon as JSON for labels the
   frontend JS generates at runtime (table headings, button text). Public, no
   auth — branding text, not data, same rationale as `/api/siblings`.

`Lexicon.normalize()` fills unset fields: nova-a defaults for product wording,
and regular derivation (`Org` → `Orgs` / `Org` / `Orgs`) so a goatnet typically
sets only `Org` plus the two product strings.

## Token set

| Token / JSON field | nova-a default | ZAGROS |
|---|---|---|
| `__LEX_PRODUCT_NAME__` / `product_name` | `Zenoh Trial` | `ZAGROS Fabric` |
| `__LEX_PRODUCT_TITLE__` / `product_title` | `Zenoh Trial — Vendor Portal` | `ZAGROS Fabric Portal` |
| `__LEX_ORG__` / `org` | `vendor` | `org` |
| `__LEX_ORGS__` / `orgs` | `vendors` | `orgs` |
| `__LEX_ORG_TITLE__` / `org_title` | `Vendor` | `Org` |
| `__LEX_ORGS_TITLE__` / `orgs_title` | `Vendors` | `Orgs` |
| `__LEX_ORG_UPPER__` / `org_upper` | `VENDOR` | `ORG` |
| `__LEX_PANOSCOPE_URL__` / `panoscope_url` | `https://panoscope.nova.…/` | `https://panoscope.zagros.…/` |

`__LEX_ORG_UPPER__` is the all-caps singular, used only for shell-variable names
in the peering docs' copy-paste examples (`export VENDOR=…` / `export ORG=…`, then
`$VENDOR` / `$ORG`) so a tokenized example stays internally consistent.

`ProductTitle` is kept distinct from `ProductName` because the two don't
compose uniformly: nova-a wants "Zenoh Trial — Vendor Portal" but ZAGROS wants the
shorter "ZAGROS Fabric Portal", not "ZAGROS Fabric — Org Portal".

An **unset** lexicon renders byte-identical to what shipped before the lexicon
existed — nova-a is unchanged by construction (tests in
`internal/config/lexicon_test.go` + `internal/api/static_test.go` assert both
the nova defaults and that no `__LEX_` token survives either render).

## Configuration

`PORTAL_LEX_*` env (or `--lex-*` flags), wired by the `goat-zenoh-portal`
ansible role:

- Role defaults (`defaults/main.yml`): product/org fields empty (→ nova wording);
  `portal_lex_panoscope_url` templated off `site_id`/`site_dns_zone` so every
  env — including nova — links to its own panoscope (fixing the hardcoded-nova-host
  link that shipped in the static HTML).
- ZAGROS (`group_vars/zagros_hosts/vars.yml`): `portal_lex_product_name`,
  `portal_lex_product_title`, `portal_lex_org`.

The env file `{% if … %}`-guards each var, so an empty default emits no env line
and the Go binary fills the nova default.

## Deploy + rollback

Display change → rebuild the portal image with the new binary + tokenized HTML,
re-pin `portal_image`, and refresh the ZAGROS host. The swap is a single container
recreate (sub-second); the rebuild happens offline. No data migration, no ACL
edit, no router restart — the lexicon touches nothing stateful. Rollback is the
prior image pin, or unsetting the `portal_lex_*` vars (reverts to nova wording).

## Increment status

- **Increment 1 (built, #1013):** the engine (config + serve-time render +
  `/api/branding`) and the chrome — masthead title/`<h1>`, nav, the full
  index.html landing page (sign-in, welcome, the operator mint form + success
  card), the panoscope nav link, and the product-name phrase across every page.
- **Increment 2 (built):** the sub-page body copy (`topics/vendor/schema/`
  `inspector/progress/docs/patterns.html`) and the per-page JS-generated labels.
  The JS reads `/api/branding` once at page load via an `app.js` `lexicon()`
  helper (cached; nova-a fallback if the fetch fails) and a module-level `LEX` the
  sync render helpers read. The engine does not change.

  Two boundaries the body pass enforces, both verified in-browser against a
  local portal in each lexicon:
  - **"vendor" is not always the lexicon noun.** It stays literal where it
    means something else: the `source === "vendor"` schema-source enum, the
    bulk-upload YAML field key (`vendor:`), the `(vendor, key_pattern)` upsert
    tuple, `urn:goat-nova:vendor=` URI-SANs, `/api/vendors` routes, the
    `/vendor.html` page route and `/vendor.js` script src, element ids / CSS
    classes, and the *equipment-manufacturer* sense ("vendor REST",
    "vendor-proprietary" in `patterns.html`). Only the goat-participant noun and
    namespace placeholders (`<vendor>` → `<org>`, example keys `vendor/radar` →
    `org/radar`) are tokenized.
  - **Indefinite articles are reworded, not tokenized.** "a vendor" can't become
    "a org" (vowel-initial), and a static token can't satisfy both lexicons — so
    those sentences use "each"/"one"/plural instead; the one JS site computes
    `a`/`an` from the noun's first letter.

- **Increment 3 (built):** the surfaces increment 2 missed — the entire
  `peering/*.html` constellation (guide, best-practices, our-config,
  zenohd-config, workbench, probe, echo, log-analyzer), `admin/vendor-activity.html`,
  and the last two `index.html` tooltips. Added the `__LEX_ORG_UPPER__` token for
  the peering shell examples. **`admin/vendor-activity.html` was hand-edited, not
  bulk-tokenized** — it has a large inline `<script>`, and the bulk pass had
  renamed JS *identifiers* (`const vendors`, the `data-vendor` attribute); that
  renders consistently post-substitution but is a latent footgun, so only its
  static body + headers were tokenized and every JS identifier/attribute/field
  left literal. (The interactive peering tools only had string literals + example
  code tokenized — their JS code was untouched.) Both lexicons re-verified in a
  real browser.

### What deliberately still reads "vendor" (machine / API contract)

After increment 3, a small set of "vendor" remains visible — every instance is a
real machine identifier that the display layer must not rewrite, because renaming
the *word* without renaming the *thing* would make examples wrong or break the
API. These are not oversights:

- **`/api/vendors/…`** — the HTTP routes (shown in curl examples + the swagger
  spec). The API is vendor-keyed.
- **`vendor:`** — the bulk-topic export/import YAML field (`bulkFile.Vendor`,
  `yaml:"vendor"` in `internal/api/bulk.go`); a portable on-disk format, not
  env-dependent. The docs example must match it or upload fails.
- **`traits.vendor.short_tag`** — the Ory identity trait path the onboard
  orchestrator writes (admin page intro).
- **`source === "vendor"`** — the schema-source enum value.

Renaming any of these is a separate, breaking API/schema change (it would ripple
to `goat-cli`, panoscope, and exported files), out of scope for a display-only
lexicon. If the API itself should become org-keyed, that is its own tracked
effort.

**User-facing *message* strings are NOT in that machine-contract set and were
de-vendored (2026-06-20).** A set of API error messages + `log_analyzer` FixHints
+ access-query notes used the literal phrase "vendor prefix" and — unlike the
onboarding packet — never passed through `applyLexicon`, so they leaked
vendor-centric wording to end users on org-keyed goatnets (ZAGROS). These are
human prose, not identifiers, routes, or wire fields, so they were hardcoded to
the org-agnostic "org prefix" (in `registry`, `api/handlers`, `api/access`,
`api/log_analyzer`, `api/fabric_config`, `access/query`, `pubsubstate`,
`certmint`). Hardcoded rather than lexicon-threaded because these paths have no
lexicon handle and the direction is org-agnostic post-nova; a future
lexicon-threading (so nova-a could still read "vendor") is optional polish, not a
correctness gap. Go identifiers (`VendorPrefix`, `vendorPrefix`) are untouched —
they're a separate refactor, never user-visible.

(Note: the `openapi.yaml` swagger surface still *documents* the literal
vendor-keyed `/api/vendors` routes — that's the contract — but its nova
*branding* (title/description/contact/server) was de-novaed to neutral
wording + a relative server URL so one spec serves every goatnet.)

### Cert identity layer — per-goatnet machine, not display

The minted-cert identity strings used to be hardcoded nova values that ZAGROS
inherited unchanged, so ZAGROS certs read `urn:goat-nova:vendor=<anchor>` despite the
display relex. These are **not** lexicon tokens (a lexicon change must never reach
a cert) and they are **not** fixed — they are per-goatnet machine values set by
the `goat-zenoh-portal` role. The role passes every one of them to the binary
on every goatnet; the binary's own built-in defaults are example values used only
by an unconfigured portal (tests, local runs):

| Concern | Config (role var / flag / env) | Binary default | Role default | ZAGROS |
|---|---|---|---|---|
| URI-SAN prefix | `portal_cert_sign_uri_prefix` / `--uri-prefix` / `PORTAL_URI_PREFIX` | `urn:goat:example:org=` | `urn:goat:<site_id>:org=` | `urn:goat:zagros:org=` |
| Leaf CN suffix | `portal_cert_sign_cn_suffix` / `--cn-suffix` | `-trial-peer` | `-trial-peer` | `-peer` (unused in slot-pool mode — CN = bare anchor) |
| DNS-SAN suffix | `portal_cert_sign_dns_suffix` / `--dns-suffix` | `example.peer` | `<site_id>.peer` | `zagros.peer` |
| Onboarding tarball stem | `portal_onboarding_tarball_stem` | `goat-onboarding` | `<site_id>-onboarding` | `zagros-onboarding` |
| In-tarball packet name | `portal_onboarding_packet_name` | `zenoh-trial-onboarding.md` | `zenoh-trial-onboarding.md` | `zagros-fabric-onboarding.md` |
| In-tarball CA-root name | `portal_onboarding_root_cert_name` | `ca-root.pem` | `<site_id>-ca-root.pem` | `zagros-ca-root.pem` |

The CA-root file bundled into the tarball is `--ca-root-cert` / `PORTAL_CA_ROOT_CERT`
(the role passes `--ca-root-cert`, `portal_ca_root_cert_flag`; the legacy name
`--nova-root-cert` still parses and wins when both are set). The root-CA download at `/peering/root-ca.pem` is served under the in-tarball
CA-root name.

**The URI-SAN prefix is freeze-critical:** it lands in every cert and is pinned by
external mTLS integrations + (in per-vendor mode) the router ACL `cert_common_names`.
In per-uuid-slot-pool mode (ZAGROS) the ACL matches the **CN (bare anchor)**, not the
URI-SAN, so changing the prefix does not break already-issued certs — but it must
be set before external orgs build against it, since re-minting a coalition's worth
of integrated certs is a flag-day. The `/api/vendors` route + `vendor:`/`vendor`
contract noun stay a separate, larger breaking-change track.

### Onboarding packet (also lexicon-aware)

The mint-time tarball — `NOTES.md` + the worked `first-publisher.py` /
`first-subscriber.py` rendered in `internal/certmint/certmint.go` — is org-aware
too. The `Minter` carries `config.Lexicon`; the display words are rewritten at
render time (`applyLexicon`, threaded via `tarballParams` + a `strings.Replacer`):
ZAGROS gets "Org notes", "your org prefix", `org "<x>" → ZAGROS Fabric`, and the
publisher's prefix variable becomes `ORG` (renamed consistently so the example
stays runnable). The subscriber names its slots `CERT_SLOT_ID` (the cert it
connects as) and `TOPIC_SLOT_ID` (whose topics it reads), and the lexicon
rewrites neither. nova-a defaults reproduce the prior "vendor"/`VENDOR` wording. Machine
strings — cert/key/CA-root filenames, the `urn:` URI-SAN, the `.cbor` name — carry
no matched phrase and pass through. Lands on ZAGROS with the next portal image bump.

## Panoscope display layer

Panoscope is a **separate image** from the portal, so it carries its own copy of
this same display-only lexicon — the portal's tokens don't reach it. It shipped
with the nova wording baked into its embedded frontend: the tab title +
masthead read "nova fabric", the side-panel rows / fold buttons / fleet bar
/ search overlay / legend all said "vendor". On ZAGROS that misframed the COP the
same way the portal did before its relex.

Same shape as the portal, two mechanisms because panoscope's UI is half static
HTML and half D3-generated DOM:

- **Static HTML** (`frontend/index.html`) carries `__LEX_*__` tokens
  (`__LEX_ORG__`, `__LEX_ORGS__`, `__LEX_ORG_UPPER__`, `__LEX_ORGS_UPPER__`,
  `__LEX_FABRIC__`). `src/embed.rs` substitutes them in a single non-overlapping
  pass when serving any `text/html` response. Panoscope still does this per
  request; the portal's `static.go` moved the equivalent pass to construction
  time in 2026-08 (§Mechanism). Same substitution, different moment — the
  distinction only matters if panoscope later precomputes encodings too. The JS
  bundle + other assets pass through untouched.
- **JS-generated labels** read their wording at runtime from `GET /api/legend`,
  which now carries a `lexicon` object (`config::Lexicon`, Rust-side). The
  frontend's `src/lex.ts` holds a mutable `LEX` singleton seeded with nova-a
  defaults and updated on every legend poll (`applyLexicon`); it also re-applies
  the document title + subtitle, so the wording is correct even on the Vite dev
  server where serve-time substitution doesn't run.

**Same invariants as the portal's lexicon.** Display only — the participant
cluster on the wire is always keyed by the cert-bound `vendor` field / UUID
anchor, the ACL subjects + audit + deep-link `key=` params are untouched. The
`<code>vendor</code>` schema-decl field name + the `vendor`/`site`/`role_class`
metadata list stay literal (they name a real wire field). An unconfigured
panoscope renders "vendor" and "Zenoh fabric"; every goatnet sets
`portal_lex_product_name`, so the fabric label on a deployed panoscope always
comes from its group_vars.

Configured by the `panoscope` ansible role's `env.j2` from the **same group_vars
the portal reads** — `portal_lex_org` ("org" on ZAGROS) for the noun and
`portal_lex_product_name` ("ZAGROS Fabric") for the fabric label — so the word can
never drift between the two surfaces. Role-local overrides `panoscope_lex_org` /
`panoscope_lex_orgs` / `panoscope_lex_fabric_label` exist but are normally unset.
Env vars: `PANOSCOPE_LEX_ORG` / `PANOSCOPE_LEX_ORGS` / `PANOSCOPE_LEX_FABRIC_LABEL`
(plural + caps forms derive from the singular when unset). Lands on ZAGROS with the
next panoscope image bump + role apply.

## Flashlight + stethoscope display layer

The other two inspection surfaces carry the same display-only lexicon — each is
its own image, so each has its own copy, all sourced from the one `portal_lex_org`
group_var so the participant noun is identical everywhere.

**flashlight** (server-rendered Rust + an inlined `app.js`): `render::Lexicon`
(`org`/`orgs`, nova-a default) substitutes the server-side strings (jump box
title/placeholder, the "pick a … or topic" directory prompt) directly, and emits
a `window.GOAT_LEX` global the inlined `app.js` reads for its generated labels
(the directory heading "…/groups", the "no … registered" empty-state, the
"N …(s)" count). Env `FLASHLIGHT_LEX_ORG` / `FLASHLIGHT_LEX_ORGS`.

**stethoscope** (server-rendered Rust + `stream.js` + the `stethoscope-checks`
crate): `config::Lexicon` (`org`/`org_upper`/`org_title`) relexes the hero kind
eyebrow ("VENDOR" → "ORG"), the picker kind-chip ("Vendor" → "Org"), the
key-shape placeholders (`vendor/topic/version` → `org/topic/version`), and the
auto-detect hint; a `window.STETHOSCOPE_LEX` global carries the same to
`stream.js`. The **check verdict messages** relex too — `CheckCtx` gained an
`org` field (`.with_org()` from the binary, default "vendor"), so the
registration / schema-body / decode / flow-rate / cardinality / topic-roster
checks render "Org 'X' has no portal row" etc. on ZAGROS. Env `STETHOSCOPE_LEX_ORG`.

What stays literal on both (machine, not display): the `?vendor=` deep-link query
param (Block 94 canon), the `vendor`/`key` filter + `TargetKind` value, the
portal `list_vendors` / `/api/vendors` routes, the `vendor-decode-rca` tool +
`vendor-schema-fixup.md` doc names, and equipment-manufacturer "vendor source"
(the `.fds`/`.proto` provenance, the same carve-out as the portal's "vendor
REST"). All three surfaces land on ZAGROS with their next image bump + role apply.
