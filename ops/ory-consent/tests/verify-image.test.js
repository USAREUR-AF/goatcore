// ops/ory-consent/tests/verify-image.test.js
//
// GHSA-9v3v: the ory role's deploy-time consent-image gate
// (roles/ory/tasks/consent-image-gate.yml -> verify-image.sh) must accept an
// image carrying the fixed readers, reject one carrying the pre-fix traits
// fallback, and fail closed (rc 2) when it cannot check.
//
// Builds tiny LOCAL images FROM scratch (nothing is pulled) and removes them.
// Needs a docker daemon; skipped (exit 0, "SKIP") without one.
//
// Run:  node ops/ory-consent/tests/verify-image.test.js
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const sh = (cmd, args, opts = {}) => spawnSync(cmd, args, { encoding: "utf8", ...opts });
if (sh("docker", ["info"]).status !== 0) {
  console.log("verify-image: SKIP (no docker daemon)");
  process.exit(0);
}

const src = fs.readFileSync(path.join(__dirname, "..", "apply-patch.sh"), "utf8");
function injected(marker, nextMarkerVar) {
  const start = src.indexOf(`='${marker}'`);
  const end = src.indexOf(`${nextMarkerVar}=`, start);
  return src.slice(start, end).split("\n")
    .map((l) => l.match(/^\s*print indent (.*)$/)).filter(Boolean)
    .map((m) => m[1].replace(/" marker "/g, marker))
    .map((s) => s.replace(/^"|"$/g, "").replace(/\\"/g, '"').replace(/\\\\/g, "\\"));
}
const FIXED = [...injected("goat-maintainer-groups-forwarding-patch", "MARKER4"),
  ...injected("goat-maintainer-aff-forwarding-patch", "MARKER2")];
const PREFIX = [
  "// goat-maintainer-groups-forwarding-patch — forward the groups claim into the JWT id_token.",
  'var __gm = identity.metadata_public || {}, __groups = Object.prototype.hasOwnProperty.call(__gm, "groups") ? __gm.groups : identity.traits.groups;',
  "if (Array.isArray(__groups) && __groups.length > 0) session.id_token.groups = __groups;",
  "// goat-maintainer-aff-forwarding-patch — forward the aff claim into the JWT id_token.",
  'var __am = identity.metadata_public || {}, __aff = Object.prototype.hasOwnProperty.call(__am, "groups") ? __am.aff : (__am.aff || identity.traits.aff);',
  'if (__aff && typeof __aff === "object" && Object.keys(__aff).length > 0) session.id_token.aff = __aff;',
];

const tag = `goat-test/verify-image-${process.pid}`;
const built = [];
function image(name, claimLines, withConsent = true) {
  const ctx = fs.mkdtempSync(path.join(os.tmpdir(), "verify-image-ctx-"));
  const two = (m, s) => `// ${m} ${s}\n// ${m} ${s}`;
  fs.mkdirSync(path.join(ctx, "app", "lib", "routes"), { recursive: true });
  fs.mkdirSync(path.join(ctx, "app", "views", "partials"), { recursive: true });
  if (withConsent) {
    fs.writeFileSync(path.join(ctx, "app", "lib", "routes", "consent.js"), [
      "function claims(identity, session) {", ...claimLines, "if (identity.updated_at) {}", "}",
      two("goat-maintainer-audience-fallback-patch", "body.client.audience"),
      two("goat-maintainer-scope-fallback-patch", "body.client.scope"),
    ].join("\n") + "\n");
  }
  fs.writeFileSync(path.join(ctx, "app", "views", "partials", "scripts.hbs"),
    "goat-maintainer-double-submit-guard data-dlf-submitted setTimeout __dlfSubmitGuardInstalled\n");
  fs.writeFileSync(path.join(ctx, "Dockerfile"), "FROM scratch\nCOPY app /usr/src/app\n");
  const ref = `${tag}:${name}`;
  const r = sh("docker", ["build", "-q", "--pull=false", "-t", ref, ctx]);
  fs.rmSync(ctx, { recursive: true, force: true });
  if (r.status !== 0) throw new Error(`docker build ${ref} failed: ${r.stderr}`);
  built.push(ref);
  return ref;
}

let ok = true;
const check = (name, pass, detail) => {
  console.log(`  ${pass ? "PASS" : "FAIL"}  ${name}`);
  if (!pass) { ok = false; if (detail) console.log(detail); }
};
const verify = (ref) => {
  const r = sh("sh", [path.join(__dirname, "verify-image.sh"), ref]);
  return { code: r.status, out: (r.stdout || "") + (r.stderr || "") };
};

try {
  const fixed = verify(image("fixed", FIXED));
  check("fixed consent image: gate passes (rc 0)", fixed.code === 0 && fixed.out.includes("verify-patched.sh: PASS"), fixed.out);
  const prefix = verify(image("prefix", PREFIX));
  check("pre-fix consent image: gate rejects (rc 1)", prefix.code === 1 && prefix.out.includes("pre-GHSA-9v3v"), prefix.out);
  const empty = verify(image("noconsent", [], false));
  check("image without consent.js: cannot check (rc 2, fail closed)", empty.code === 2, empty.out);
  const absent = verify(`${tag}:never-built`);
  check("image not present locally: cannot check (rc 2, never pulls)",
    absent.code === 2 && absent.out.includes("not present locally"), absent.out);
} finally {
  for (const ref of built) sh("docker", ["rmi", "-f", ref]);
}

console.log(ok ? "verify-image: OK" : "verify-image: BROKEN");
process.exit(ok ? 0 : 1);
