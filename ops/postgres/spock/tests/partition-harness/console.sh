# shellcheck shell=bash disable=SC2034  # sourced by run.sh; DBS and EXTRA are its arrays
# The id console and an onboard-runner at each harness site (ADR 1165 D5;
# design island-mode-multi-writer.md §6; inventory D12: "the double-claim case
# (C7) needs two onboard-runners").
#
# Sourced by run.sh after it declares DBS and EXTRA. Each site runs:
#   - goat-id-console, built from ops/goat-id-console, on that site's
#     id_console_onboard and id_console_orgs databases (its own migrations make
#     the tables);
#   - the onboard-runner binary, built from ops/onboard-runner, run once per
#     scenario step against that site's console.
# Both console databases join a Spock replication set, id_console, holding the
# tables each store declares (onboardstore.Tables, orgstore.Tables). No role does
# this yet: it is what program step 6 would turn on, and the harness shows what
# happens when it is.
#
# The runner's mint is `goat admin onboard <handle> --native`. Here --goat-bin
# is goat-shim.sh, which stands in for goat-cli and makes the two writes the
# native mint makes at the runner's own site: a NetBird setup key named for the
# handle (inventory N1, as SQL, like every other NetBird write in this harness)
# and a Kratos identity for the manifest's email, owned by the site (ADR 1205
# D5). It prints the mint's summary line, which the runner parses. The console,
# its store and the runner are the real code.
#
# Inputs: CONSOLE_IMAGE (default: built here) and RUNNER_BIN (default: built
# here from the runner Dockerfile's build stage).

CONSOLE_DBS=(id_console_onboard id_console_orgs)
DBS+=("${CONSOLE_DBS[@]}")
EXTRA+=("$PFX-console-a" "$PFX-console-b")
CONSOLE_PORT_a="${HARNESS_PORT_CONSOLE_A:-58080}"
CONSOLE_PORT_b="${HARNESS_PORT_CONSOLE_B:-58081}"
CONSOLE_KEY="partition-harness-runner-key-not-secret"
CONSOLE_ENV="zagros"

BUILD_CONSOLE=0; [[ -n "${CONSOLE_IMAGE:-}" ]] || { CONSOLE_IMAGE="goat-id-console:partition-harness"; BUILD_CONSOLE=1; }
BUILD_RUNNER=0;  [[ -n "${RUNNER_BIN:-}" ]] || { RUNNER_BIN="$WORK/onboard-runner"; BUILD_RUNNER=1; }
console_build() { # the images the caller did not supply; run in the background, so it reports by file
  local c
  if [[ "$BUILD_CONSOLE" == 1 ]]; then
    docker build -q -t "$CONSOLE_IMAGE" "$REPO/ops/goat-id-console" >/dev/null
  fi
  if [[ "$BUILD_RUNNER" == 1 ]]; then
    docker build -q --target build -t onboard-runner-build:partition-harness "$REPO/ops/onboard-runner" >/dev/null
    c=$(docker create onboard-runner-build:partition-harness)
    docker cp -q "$c:/onboard-runner" "$RUNNER_BIN"
    docker rm "$c" >/dev/null
  fi
  : > "$WORK/console-built"
}

console_schema() { # each site's console, started on its own databases; its migrations make the tables
  local s db port
  [[ -f "$WORK/console-built" && -x "$RUNNER_BIN" ]] || { echo "::error::console image or onboard-runner build failed"; exit 1; }
  for s in a b; do
    for db in "${CONSOLE_DBS[@]}"; do q "$s" "$db" "CREATE EXTENSION IF NOT EXISTS spock;"; done
  done
  for s in a b; do
    port="CONSOLE_PORT_$s"
    docker run -d --name "$PFX-console-$s" --network "$NET" -p "127.0.0.1:${!port}:8080" \
      -e ONBOARD_DB_DSN="host=$PFX-$s user=postgres password=$PW dbname=id_console_onboard sslmode=disable" \
      -e ORG_DB_DSN="host=$PFX-$s user=postgres password=$PW dbname=id_console_orgs sslmode=disable" \
      -e ONBOARD_OPERATOR_API_KEYS="onboard-runner:$CONSOLE_KEY" \
      -e IDP_TENANT_KRATOS_ADMIN_URL="http://$PFX-kratos-$s:4434" \
      -e STEP_UP_ENFORCED=false \
      -e PERMISSIONS_YAML_PATH=/etc/goat-id-console/group-permissions.yml \
      -v "$REPO/ops/permissions/group-permissions.yml:/etc/goat-id-console/group-permissions.yml:ro" \
      "$CONSOLE_IMAGE" >/dev/null
  done
  for s in a b; do
    port="CONSOLE_PORT_$s"
    for _ in $(seq 1 60); do curl -fs -o /dev/null "http://127.0.0.1:${!port}/healthz" && break; sleep 1; done
    curl -fsS -o /dev/null "http://127.0.0.1:${!port}/healthz" \
      || { echo "::error::console at site $s not ready"; docker logs "$PFX-console-$s" 2>&1 | tail -20; exit 1; }
  done
  echo "  console at both sites; stores migrated"
}

console_wire() { # both console databases into repset id_console, subscribed both ways
  local db
  for db in "${CONSOLE_DBS[@]}"; do wire "$db" id_console; done
}

# ---------------------------------------------------------------- helpers for scenarios
capi() { # capi <site> <METHOD> <path> [json]   the console's machine API, as the runner calls it
  local port="CONSOLE_PORT_$1"
  curl -fsS -X "$2" "http://127.0.0.1:${!port}$3" -H "Authorization: Bearer $CONSOLE_KEY" \
    -H 'Content-Type: application/json' ${4:+-d "$4"}
}
console_admit() { # console_admit <site> <handle> <email>  -> prints the proposal id, approved at that site
  local manifest id
  manifest=$(jq -cn --arg h "$2" --arg e "$3" --arg env "$CONSOLE_ENV" '{env:$env, manifest:{
    identity:{handle:$h, email:$e, name:$h, status:"active", added_by:"admin@example.org"},
    affiliation:{id:"harness-unit", kind:"organic", nation:"USA"},
    netbird:{enabled:true, sites:[$env], role:"peer", usage_limit:1, auto_groups:["OBSERVER"]},
    ory_site:{enabled:true, sites:[$env], groups:[], credentials:{password:true, passkey:false}},
    ory_idp:{enabled:false, sites:[], groups:[], fido2_required:false},
    zenoh:{enabled:false}}}')
  id=$(capi "$1" POST /api/onboard/intake "$manifest" | jq -r .id)
  capi "$1" POST "/api/onboard/proposals/$id/sponsor-approve" '{}' >/dev/null
  capi "$1" POST "/api/onboard/proposals/$id/approve" '{}' >/dev/null
  echo "$id"
}
run_runner() { # run_runner <site>   one poll cycle of that site's onboard-runner; log in $SCRATCH/runner-<site>.log
  local root="$SCRATCH/runner-$1"
  mkdir -p "$root"
  chmod 0777 "$root"
  docker run --rm --network "$NET" --user postgres \
    -v "$RUNNER_BIN:/usr/local/bin/onboard-runner:ro" \
    -v "$HERE/goat-shim.sh:/usr/local/bin/goat-shim:ro" \
    -v "$root:/work" \
    -e ONBOARD_API_TOKEN="$CONSOLE_KEY" -e HARNESS_SITE="$1" -e HARNESS_PFX="$PFX" -e PGPASSWORD="$PW" \
    --entrypoint /usr/local/bin/onboard-runner "$IMAGE" \
    --once --portal http://portal.invalid --onboard "http://$PFX-console-$1:8080" \
    --goat-bin /usr/local/bin/goat-shim --repo-root /work --runner-id "onboard-runner-site_$1" \
    --only-env "$CONSOLE_ENV" > "$SCRATCH/runner-$1.log" 2>&1
}
cdb() { q "$1" id_console_onboard "$2"; }
