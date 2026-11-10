// SPDX-License-Identifier: Apache-2.0
//
// Playwright config for goat-id-console's browser e2e. Two modes:
//
//   mocked (default, CI): serve.mjs serves web/static/, the intake + find
//   specs stub /api/onboard/* via page.route. The review-page specs that need
//   the real Go templates and store (onboarding-restore.spec.ts) drive a real
//   console binary, built from ../../cmd/console and run against throwaway
//   Postgres schemas on E2E_PG_DSN (a postgres:// URL; CI's service container)
//   with DEV_BYPASS_AUTH — no Kratos, Hydra, mailer or goatnet.
//
//   live (INTAKE_URL=https://intake.<goatnet>/onboard.html?t=oti_…): the spec
//   drives a real goatnet's public intake page and expects the real 201.
//   ops/scripts/onboard-intake-smoke.sh mints the invite, runs this, and
//   rejects the resulting proposal. The local console is not started.

import { defineConfig, devices } from "@playwright/test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const PORT = Number(process.env.PORT) || 8789;
const BASE_URL = `http://127.0.0.1:${PORT}`;
const LIVE = !!process.env.INTAKE_URL;

// The real console for the review-page specs. CONSOLE_URL points at one
// already running (its operator key in CONSOLE_API_KEY); otherwise the config
// builds and starts one here. Set CONSOLE_E2E=0 to skip it (the specs that
// need it then skip themselves).
export const CONSOLE_PORT = Number(process.env.CONSOLE_PORT) || 8790;
export const CONSOLE_URL = process.env.CONSOLE_URL || `http://127.0.0.1:${CONSOLE_PORT}`;
export const GOVERNED_CONSOLE_PORT = CONSOLE_PORT + 2;
export const GOVERNED_CONSOLE_URL = process.env.GOVERNED_CONSOLE_URL || `http://127.0.0.1:${GOVERNED_CONSOLE_PORT}`;
export const CONSOLE_API_KEY = process.env.CONSOLE_API_KEY || "e2e-runner-key";
export const IDENTITY_PORT = CONSOLE_PORT + 1;
export const IDENTITY_URL = `http://127.0.0.1:${IDENTITY_PORT}`;
const START_CONSOLE = !LIVE && !process.env.CONSOLE_URL && process.env.CONSOLE_E2E !== "0";
const E2E_PG_DSN = process.env.E2E_PG_DSN || "";
if (START_CONSOLE && !E2E_PG_DSN) {
  throw new Error(
    "E2E_PG_DSN is not set: the local console stores onboarding and org data in Postgres (ADR 1165). " +
      "Point it at a server (postgres://user:pass@host:port/db?sslmode=disable), or set CONSOLE_E2E=0 to skip the console specs.",
  );
}

// The DSN for one throwaway schema: the console creates its tables there.
function schemaDSN(schema: string): string {
  const u = new URL(E2E_PG_DSN);
  u.searchParams.set("search_path", schema);
  return u.toString();
}

function consoleServer(port = CONSOLE_PORT, site = "zagros", membershipScopePath = "") {
  // Anchored on this file, not the shell cwd, so `playwright test -c <path>`
  // from the repo root starts the same console.
  const here = dirname(fileURLToPath(import.meta.url));
  const repoOps = join(here, "..", "..", "..");
  return {
    // One pair of throwaway schemas per port, dropped and recreated at start:
    // Playwright ends the server with a signal a shell trap does not survive,
    // so cleanup on exit is not reliable and a fresh-per-run schema would leak
    // one per run. Nothing is created when this config is merely evaluated
    // (every worker does).
    command:
      `psql "$E2E_PG_DSN" -v ON_ERROR_STOP=1 -qc ` +
      `'DROP SCHEMA IF EXISTS e2e_${port}_onboard CASCADE; CREATE SCHEMA e2e_${port}_onboard; ` +
      `DROP SCHEMA IF EXISTS e2e_${port}_org CASCADE; CREATE SCHEMA e2e_${port}_org;' && ` +
      `exec go run ${join(here, "..", "..", "cmd", "console")}`,
    url: `http://127.0.0.1:${port}/healthz`,
    // Never reuse: a leftover console from an aborted run would skip the
    // state wipe above and test stale code. Point CONSOLE_URL at your own
    // console instead when that is what you want.
    reuseExistingServer: false,
    timeout: 420_000, // a cold CI runner compiles the console with no module cache
    // Quiet locally (the console logs every request as JSON); set
    // CONSOLE_E2E_LOG=1 to see it when the server will not come up. CI always
    // pipes stderr, so a console that fails to start shows the reason instead
    // of only the timeout.
    stdout: "ignore" as const,
    stderr: (process.env.CONSOLE_E2E_LOG || process.env.CI ? "pipe" : "ignore") as "pipe" | "ignore",
    env: {
      IDP_TENANT_KRATOS_ADMIN_URL: IDENTITY_URL,
      IDP_TENANT_HYDRA_ADMIN_URL: IDENTITY_URL,
      LISTEN_ADDR: `127.0.0.1:${port}`,
      NOTIFY_GOATNET: site,
      PERMISSIONS_YAML_PATH: join(repoOps, "permissions", "group-permissions.yml"),
      COALITION_FRAMEWORKS_PATH: join(repoOps, "identity", "coalition-frameworks.yaml"),
      DEV_BYPASS_AUTH: "1",
      STEP_UP_ENFORCED: "false",
      AUDIT_BACKEND: "stderr",
      ONBOARD_OPERATOR_API_KEYS: `e2e-runner:${CONSOLE_API_KEY}`,
      MEMBERSHIP_SCOPE_PATH: membershipScopePath,
      MEMBERSHIP_AUTHORITY: membershipScopePath ? "scope" : "ungoverned",
      HOLD_RECORDS_PATH: join(here, "fixtures", site === "zagros-dev" ? "hold-records-governed.jsonl" : "hold-records.jsonl"),
      NOTIFY_MGMT_URL: `http://127.0.0.1:${IDENTITY_PORT}/${site}`,
      ENROLL_WATCH_NB_TOKEN: "e2e-netbird-token",
      E2E_PG_DSN,
      ONBOARD_DB_DSN: schemaDSN(`e2e_${port}_onboard`),
      ORG_DB_DSN: schemaDSN(`e2e_${port}_org`),
    },
  };
}

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: "list",
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
    // INTAKE_RESOLVE=<host>=<ip>[,<host>=<ip>]: pin names to addresses inside
    // the browser only. Live mode runs from an operator laptop that is usually
    // ON the goatnet's mesh, where mesh DNS answers the public intake name with
    // the management host's mesh address and the request lands on the
    // mesh-facing entrypoint (404). A candidate is off-mesh; this makes the
    // browser leg resolve the way a candidate does.
    launchOptions: process.env.INTAKE_RESOLVE
      ? { args: ["--host-resolver-rules=" + process.env.INTAKE_RESOLVE.split(",").map((m) => "MAP " + m.replace("=", " ")).join(", ")] }
      : {},
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  // No local server in live mode — the page under test is the goatnet's.
  webServer: LIVE
    ? undefined
    : [
        {
          command: "node serve.mjs",
          url: BASE_URL,
          reuseExistingServer: !process.env.CI,
          timeout: 30_000,
        },
        ...(START_CONSOLE ? [{
          command: "node identity-fixture.mjs",
          url: `${IDENTITY_URL}/healthz`,
          env: { IDENTITY_PORT: String(IDENTITY_PORT) },
          reuseExistingServer: false,
        }, consoleServer(), consoleServer(
          GOVERNED_CONSOLE_PORT,
          "zagros-dev",
          join(dirname(fileURLToPath(import.meta.url)), "fixtures", "membership-scope-governed.yaml"),
        )] : []),
      ],
});
