// ops/ory-consent/tests/claim-forwarding.test.js
//
// Behavioural test of patches 1 and 4 (groups and aff forwarding into the JWT
// id_token) after the F-572 / ADR 1188 move to admin-managed metadata_public.
// Runs on bare node, no image build, so it runs on every PR.
//
// Run:  node ops/ory-consent/tests/claim-forwarding.test.js
//
// The injected code is READ OUT OF apply-patch.sh (its awk `print indent "..."`
// lines), not copied here, so the test runs what the image runs.
//
// The property that matters (GHSA-9v3v): self-editable traits are never
// forwarded, whether or not the identity has been backfilled. A
// not-yet-backfilled identity used to fall back to traits.groups, which its
// holder can write through the settings flow while ory_authz_in_metadata is
// false, so it could self-grant PLATFORM-ADMIN.
//
// It also holds the shape verify-patched.sh greps for in the built image: the
// id_token assignment within two lines of each marker. That check only runs on
// a main push, after merge, so a layout that breaks it is caught here instead.
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const src = fs.readFileSync(path.join(__dirname, "..", "apply-patch.sh"), "utf8");

// The awk body of one patch: the `print indent ...` lines between its MARKER
// assignment and the next patch's, with the awk string concatenation undone.
function injected(marker, nextMarkerVar) {
  const start = src.indexOf(`='${marker}'`);
  const end = src.indexOf(`${nextMarkerVar}=`, start);
  if (start < 0 || end < 0) throw new Error(`patch block for ${marker} not found in apply-patch.sh`);
  return src
    .slice(start, end)
    .split("\n")
    .map((l) => l.match(/^\s*print indent (.*)$/))
    .filter(Boolean)
    .map((m) => m[1].replace(/" marker "/g, marker))
    .map((s) => s.replace(/^"|"$/g, "").replace(/\\"/g, '"').replace(/\\\\/g, "\\"));
}

const groupsLines = injected("goat-maintainer-groups-forwarding-patch", "MARKER4");
const affLines = injected("goat-maintainer-aff-forwarding-patch", "MARKER2");

// Both patches land at the same anchor inside one function, groups first.
const run = vm.runInNewContext(
  `(function (identity) { var session = { id_token: {} };\n${groupsLines.join("\n")}\n${affLines.join("\n")}\nreturn session.id_token; })`,
);

let ok = true;
const check = (name, pass) => {
  console.log(`  ${pass ? "PASS" : "FAIL"}  ${name}`);
  if (!pass) ok = false;
};
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

const selfGrant = { email: "u@example.test", groups: ["PLATFORM-ADMIN"], aff: { id: "evil", kind: "vendor" } };
const orgAff = { id: "acme", kind: "vendor" };

console.log("groups claim");
// GHSA-9v3v: fail closed. Self-editable traits never grant a role, migrated or not.
check("not migrated: traits.groups NOT forwarded", run({ traits: selfGrant }).groups === undefined);
check("migrated: metadata_public.groups forwarded, traits ignored",
  same(run({ traits: selfGrant, metadata_public: { groups: ["IAM-ADMIN"] } }).groups, ["IAM-ADMIN"]));
check("migrated with []: no claim, traits ignored",
  run({ traits: selfGrant, metadata_public: { groups: [] } }).groups === undefined);
check("migrated with null: no claim, traits ignored",
  run({ traits: selfGrant, metadata_public: { groups: null } }).groups === undefined);
check("metadata_public without a groups key: traits NOT forwarded",
  run({ traits: selfGrant, metadata_public: { org: {} } }).groups === undefined);

console.log("aff claim");
check("not migrated: traits.aff NOT forwarded", run({ traits: selfGrant }).aff === undefined);
check("not migrated: metadata_public.aff forwarded", same(run({ traits: selfGrant, metadata_public: { aff: orgAff } }).aff, orgAff));
check("migrated: metadata_public.aff forwarded, traits ignored",
  same(run({ traits: selfGrant, metadata_public: { groups: [], aff: orgAff } }).aff, orgAff));
check("migrated with null groups and no aff: traits.aff ignored",
  run({ traits: selfGrant, metadata_public: { groups: null } }).aff === undefined);

console.log("verify-patched.sh window (marker + 2 lines)");
const windowHas = (lines, marker, needle) => {
  const i = lines.findIndex((l) => l.includes(marker));
  return i >= 0 && lines.slice(i, i + 3).some((l) => l.includes(needle));
};
check("groups marker is followed by session.id_token.groups",
  windowHas(groupsLines, "goat-maintainer-groups-forwarding-patch", "session.id_token.groups"));
check("aff marker is followed by session.id_token.aff",
  windowHas(affLines, "goat-maintainer-aff-forwarding-patch", "session.id_token.aff"));

// verify-patched.sh is the post-build gate. It must accept the fixed reader
// and REJECT the pre-GHSA-9v3v one (the traits fallback), which a stale or
// cached image would still carry. Run it against a fixture app root.
console.log("verify-patched.sh on a fixture app root");
const os = require("os");
const { spawnSync } = require("child_process");
const OLD_GROUPS = [
  "// goat-maintainer-groups-forwarding-patch — forward the groups claim into the JWT id_token.",
  'var __gm = identity.metadata_public || {}, __groups = Object.prototype.hasOwnProperty.call(__gm, "groups") ? __gm.groups : identity.traits.groups;',
  "if (Array.isArray(__groups) && __groups.length > 0) session.id_token.groups = __groups;",
];
const OLD_AFF = [
  "// goat-maintainer-aff-forwarding-patch — forward the aff claim into the JWT id_token.",
  'var __am = identity.metadata_public || {}, __aff = Object.prototype.hasOwnProperty.call(__am, "groups") ? __am.aff : (__am.aff || identity.traits.aff);',
  'if (__aff && typeof __aff === "object" && Object.keys(__aff).length > 0) session.id_token.aff = __aff;',
];
function gate(groups, aff) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "verify-patched-"));
  fs.mkdirSync(path.join(root, "lib", "routes"), { recursive: true });
  fs.mkdirSync(path.join(root, "views", "partials"), { recursive: true });
  const two = (m, s) => `// ${m} ${s}\n// ${m} ${s}`;
  fs.writeFileSync(path.join(root, "lib", "routes", "consent.js"), [
    "function claims(identity, session) {", ...groups, ...aff, "if (identity.updated_at) {}", "}",
    two("goat-maintainer-audience-fallback-patch", "body.client.audience"),
    two("goat-maintainer-scope-fallback-patch", "body.client.scope"),
  ].join("\n") + "\n");
  fs.writeFileSync(path.join(root, "views", "partials", "scripts.hbs"),
    "goat-maintainer-double-submit-guard data-dlf-submitted setTimeout __dlfSubmitGuardInstalled\n");
  const r = spawnSync("sh", [path.join(__dirname, "verify-patched.sh")], {
    env: { ...process.env, VERIFY_PATCHED_ROOT: root }, encoding: "utf8" });
  fs.rmSync(root, { recursive: true, force: true });
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
}
const fixed = gate(groupsLines, affLines);
check("fixed reader: gate PASSES", fixed.code === 0 && fixed.out.includes("verify-patched.sh: PASS"));
const oldGroups = gate(OLD_GROUPS, affLines);
check("pre-fix groups reader (traits fallback): gate REJECTS",
  oldGroups.code !== 0 && oldGroups.out.includes("pre-GHSA-9v3v"));
const oldAff = gate(groupsLines, OLD_AFF);
check("pre-fix aff reader (traits fallback): gate REJECTS",
  oldAff.code !== 0 && oldAff.out.includes("pre-GHSA-9v3v"));
const otherShape = gate([OLD_GROUPS[0], "var __groups = (identity.metadata_public || {}).groups || [];",
  "session.id_token.groups = __groups;"], affLines);
check("groups marker without the fixed reader shape: gate REJECTS",
  otherShape.code !== 0 && otherShape.out.includes("not the metadata_public-only reader"));

// apply-patch.sh skips a patch whose marker is present; a target carrying the
// pre-GHSA-9v3v copy of patch 1 or 4 must be refused, not skipped as current.
console.log("apply-patch.sh on a target already carrying the pre-fix patch");
for (const [name, lines] of [["groups", OLD_GROUPS], ["aff", OLD_AFF]]) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "apply-patch-"));
  const target = path.join(dir, "consent.js");
  fs.writeFileSync(target, ["function claims(identity, session) {", ...lines, "if (identity.updated_at) {}", "}"].join("\n") + "\n");
  const r = spawnSync("sh", [path.join(__dirname, "..", "apply-patch.sh"), target], { encoding: "utf8" });
  fs.rmSync(dir, { recursive: true, force: true });
  check(`stale ${name} patch: apply-patch.sh refuses`, r.status !== 0 && (r.stderr || "").includes("pre-GHSA-9v3v"));
}

console.log(ok ? "claim forwarding: OK" : "claim forwarding: BROKEN");
process.exit(ok ? 0 : 1);
