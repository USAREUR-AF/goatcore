#!/usr/bin/env bash
# mgmt-host-compose-watch.sh — F-013 mitigation, roadmap 9L.
#
# Watches the docker-compose stack at /opt/mgmt-host and revives any service
# whose container has exited. Bridges the gap until Docker's native
# restart-policy auto-recovery is reliable on this host's Docker version.
#
# Why a watcher script instead of `docker compose up --abort-on-container-exit`
# under systemd? Targeted recovery: when one service exits unexpectedly, we
# revive only that service rather than tearing down + bringing up the whole
# stack. Lower blast radius; doesn't disturb healthy peers within the stack.
#
# Why not use `docker events`? Watching docker events misses the boot-time
# case where a container was already in 'exited' state when the watcher
# started. The poll-and-revive pattern catches both new exits and stale
# exits without state.
#
# Forward-compatible with the Phase 2D Podman+Quadlet successor track per
# docs/design/container-orchestration.md — Quadlet supervises containers
# natively via systemd. When that migration lands, this watcher retires.
#
# See: docs/findings/F-013-docker-kill-no-auto-restart-on-29.md
#      docs/project/roadmap.md §9L
#
# Block 78E: emit Prometheus textfile-collector
# metrics for every compose-defined service so an Exited-and-not-revived
# orphan triggers an alert even when it matches no allowlist entry for revive.
# Metric: compose_service_running{service="...",site="..."} = 0|1.
# Consumed by ops/compose/mgmt-host/monitoring/rules/compose-orphans.yml.

set -uo pipefail

readonly COMPOSE_DIR="${MGMT_HOST_COMPOSE_DIR:-/opt/mgmt-host}"
readonly POLL_INTERVAL="${MGMT_HOST_COMPOSE_WATCH_INTERVAL:-30}"

# Block 78E — Prometheus textfile-collector metric output.
# `node_exporter` on each Orin scrapes /var/lib/node_exporter/textfile/*.prom.
# Atomic write via tempfile + mv (node_exporter ignores partial writes).
readonly METRICS_DIR="${MGMT_HOST_COMPOSE_WATCH_METRICS_DIR:-/var/lib/node_exporter/textfile}"
readonly METRICS_FILE="${METRICS_DIR}/compose_service_running.prom"
readonly SITE_LABEL="${MGMT_HOST_COMPOSE_WATCH_SITE:-$(hostname -s)}"

# Optional allowlist — space-separated list of compose service names. When
# set, only services whose name matches one of these are revived. When
# empty/unset, all exited services are revived (preserves prior behavior).
#
# Why: services with `restart: "no"` that exit 0 by design (e.g.,
# kratos-migrate, hydra-migrate, db-bootstrap one-shots) would otherwise
# trigger a tight revive loop on every poll cycle — the watcher revives
# them, they immediately exit 0 again, the next poll revives them again.
# Each cycle hammers the database with redundant work and can destabilize
# the long-running services that depend on them. Set the allowlist to the
# long-running services (kratos-a, hydra-a, etc.) when one-shot migration
# containers share the compose project. Otherwise repeated revivals can
# destabilize Kratos enough that clients see Connection-reset-by-peer.
readonly WATCH_SERVICES="${MGMT_HOST_COMPOSE_WATCH_SERVICES:-}"

# #1365 / #1364 — patterns for services the reporting half must stay quiet
# about, mirrored from ops/ci/check-service-revival.sh's ONESHOT/EPHEMERAL
# (that script cannot be sourced here: it runs over SSH from the control
# node against this host, it is never deployed onto it). Keep both copies in
# sync by hand if either changes.
#
# ONESHOT: migration/bootstrap containers that exit 0 by design every boot.
# EPHEMERAL: load rigs and soak harnesses whose correct steady state is "not
# running" between experiments.
readonly WATCHER_ONESHOT_PATTERN='(^|-)(migrate|init|bootstrap|seed|setup|provision)(-[0-9]+)?$'
readonly WATCHER_EPHEMERAL_PATTERN='(^|-)(rig|loadgen|querygen|peergen|samplertest|nemesis|soak)(-|$)|^[Ll][0-9]+-'

log() {
  # Send to journald via systemd-cat-equivalent (logger writes to syslog,
  # which journald ingests on Ubuntu); also stdout for `journalctl -u`
  # to capture.
  echo "[$(date -Is)] $*"
}

cd "$COMPOSE_DIR" || {
  log "FATAL: compose dir $COMPOSE_DIR not found"
  exit 1
}

# Block 78E — load /opt/mgmt-host/.env so compose-file env interpolation
# works for overlays that reference secrets (admit-audit's ADMIT_AUDIT_PASSWORD,
# Ory's KRATOS_COOKIE/HYDRA_COOKIE secrets, etc.). Without this, compose ps
# returns rc!=0 on env interpolation failure and the watcher logs "WARN:
# docker compose ps failed" every cycle, never detecting orphans.
if [ -f "${COMPOSE_DIR}/.env" ]; then
  set -a
  # shellcheck source=/dev/null
  . "${COMPOSE_DIR}/.env"
  set +a
fi

# Discover compose files actually present. Order matters for compose layering;
# follow the project's convention: base first, then overlays.
#
# Wait-and-retry: on a fresh host the watcher may start BEFORE mgmt-host.yml's
# `tasks:` block has copied docker-compose.yml. Block here until the base
# compose file appears, then proceed. Keeps systemd from spinning the unit in
# Restart=always (which leaves it stuck in 'activating' from the verifier's
# point of view).
discover_compose_files() {
  COMPOSE_FILES=()
  for f in \
    docker-compose.yml \
    docker-compose.acme.yml \
    docker-compose.phase2.yml \
    docker-compose.ory.yml \
    docker-compose.obs.yml \
    docker-compose.admit-audit.yml \
    docker-compose.dashboard.yml \
    docker-compose.isitup.yml \
    docker-compose.oauth2-proxy-dashboard.yml \
    docker-compose.id-admin-console.yml \
    docker-compose.oauth2-proxy-id-console.yml \
  ; do
    [ -f "$f" ] && COMPOSE_FILES+=(-f "$f")
  done
}

discover_compose_files
if [ ${#COMPOSE_FILES[@]} -eq 0 ]; then
  log "no compose files yet in $COMPOSE_DIR; waiting for orchestrator to populate..."
  while [ ${#COMPOSE_FILES[@]} -eq 0 ]; do
    sleep 10
    discover_compose_files
  done
  log "compose files appeared; proceeding"
fi

log "starting; compose dir=$COMPOSE_DIR; files=${COMPOSE_FILES[*]}; poll=${POLL_INTERVAL}s"

# Trap SIGTERM cleanly so systemd stops us promptly
trap 'log "SIGTERM received; exiting"; exit 0' TERM INT

# Block 78E — emit the running-state of every compose-defined service.
# Atomic write via tempfile + mv. node_exporter scrapes these metrics
# every 15s by default; alerts in monitoring/rules/compose-orphans.yml
# fire on `compose_service_running == 0 for 5m` regardless of revive
# policy (so non-allowlisted services that exit-and-stay-down get
# operator-visible signal, instead of staying down with no alert).
emit_metrics() {
  mkdir -p "$METRICS_DIR" 2>/dev/null || return 0
  local tmp="${METRICS_FILE}.tmp.$$"
  {
    echo "# HELP compose_service_running 1 if the compose service is in 'running' state, 0 otherwise."
    echo "# TYPE compose_service_running gauge"
    # #1365 — NOT filtered to label=com.docker.compose.project=mgmt-host
    # any more. That filter meant this watcher could only ever report on
    # 1 of ~33 compose projects on the host and most of each host had no
    # running-state signal at all, so an exited container could stay down
    # for weeks with every check green.
    # This is the REPORTING half only; the REVIVAL loop below still only
    # touches $COMPOSE_DIR's own services — widening revival to every
    # project on the host can destabilize Kratos with hundreds of revivals
    # in minutes, so it is deliberately not done here. `project` is a label on the series rather than a
    # filter, so a dashboard or alert can still scope to mgmt-host alone.
    #
    # ONESHOT/EPHEMERAL (#1366-class containers: load rigs, retired-goatnet
    # leftovers) are skipped so a widened reporting surface does not become
    # the same noise-nobody-reads failure F-546 is about. Use raw `docker
    # ps` filtered on the compose project label existing (not its value) —
    # this works EVEN IF some compose overlays have unresolved env-var
    # interpolations (e.g. ADMIT_AUDIT_PASSWORD missing in .env breaks
    # `docker compose ps` but not raw docker ps). If the metric-emit depended on `docker compose
    # ps`, a missing env var anywhere in any overlay would kill the entire
    # alerting surface — exactly the failure mode 78E is supposed to
    # CATCH. Using raw docker labels makes the metric-emit independent
    # of compose env state.
    docker ps -a --filter "label=com.docker.compose.project" \
      --format '{{.Label "com.docker.compose.project"}} {{.Label "com.docker.compose.service"}} {{.State}}' 2>/dev/null | \
      awk -v site="$SITE_LABEL" -v oneshot="$WATCHER_ONESHOT_PATTERN" -v ephemeral="$WATCHER_EPHEMERAL_PATTERN" '
        $1 != "" && $2 != "" {
          if ($2 ~ oneshot || $2 ~ ephemeral) next
          running = ($3 == "running") ? 1 : 0
          printf "compose_service_running{service=\"%s\",site=\"%s\",state=\"%s\",project=\"%s\"} %d\n", $2, site, $3, $1, running
        }'

    # Traefik can be RUNNING and still wrong.
    #
    # Any `docker compose up -d` that names the base file without
    # docker-compose.acme.yml recreates Traefik with no
    # `--certificatesresolvers.le.*` flags. Traefik then logs "Router uses a
    # nonexistent certificate resolver" per router and serves its self-signed
    # default certificate. Every peer fails the TLS handshake to management
    # and signal, and the container stays `running` throughout — so
    # compose_service_running says 1 and nothing else on the host objects.
    #
    # Emitted ONLY on a host that carries the overlay: elsewhere the flags
    # are correctly absent and the gauge would be a false alarm. If
    # `docker inspect` cannot be read the line is omitted rather than
    # guessed at — an unreadable source must never render as clean. Traefik
    # being absent altogether is already covered by
    # compose_service_running{service="traefik"}.
    if [ -f "${COMPOSE_DIR}/docker-compose.acme.yml" ]; then
      if inspect_out=$(docker inspect traefik 2>/dev/null); then
        if printf '%s' "$inspect_out" | grep -q -- '--certificatesresolvers\.le\.'; then
          resolver_present=1
        else
          resolver_present=0
        fi
        echo "# HELP traefik_acme_resolver_present 1 if the running Traefik carries its ACME certificate resolver on a host that has the ACME compose overlay."
        echo "# TYPE traefik_acme_resolver_present gauge"
        printf 'traefik_acme_resolver_present{site="%s"} %d\n' "$SITE_LABEL" "$resolver_present"
      fi
    fi

    # #1364 — restart-policy drift. A container's restart policy is fixed at
    # creation time; nothing on the host or in CI compares it afterward with
    # what the compose file on disk now declares, so the stored config hash
    # can differ from the one `docker compose config` renders from the files
    # on disk. An ordinary deploy does not repair this (`docker compose up
    # -d` on the project leaves a stale container untouched; only
    # `--force-recreate` fixes it), so this
    # is report-only — no revive, no auto-recreate. A human applies the
    # documented repair.
    #
    # Scoped to $COMPOSE_DIR ($COMPOSE_FILES) ONLY, not every project this
    # watcher can now see for compose_service_running above. Rendering a
    # project's declared config needs that project's .env sourced for
    # interpolation to resolve — this watcher already has mgmt-host's .env
    # loaded safely at startup (line ~80); reaching into another project's
    # directory to read ITS secrets so this gauge could cover it too is the
    # exact widening #1365 argues against for revival, for the same reason.
    # ops/ci/check-service-revival.sh already reaches every project from
    # OUTSIDE the host (DOWN / NO-RESTART classes) without touching secrets.
    #
    # Degrades to omitting the series entirely if the render fails (missing
    # `docker compose`, unresolved interpolation, etc.) rather than guessing
    # — same rule the ACME check above follows.
    if declared_json=$(docker compose "${COMPOSE_FILES[@]}" config --format json 2>/dev/null) && [ -n "$declared_json" ]; then
      echo "# HELP goat_mgmt_host_restart_policy_drift 1 if a running container's live RestartPolicy disagrees with what docker compose would render from the files on disk now, 0 if it agrees."
      echo "# TYPE goat_mgmt_host_restart_policy_drift gauge"
      while IFS=' ' read -r rpd_service rpd_state rpd_cid; do
        [ -n "$rpd_service" ] || continue
        case "$rpd_service" in
          *[!A-Za-z0-9_-]*) continue ;;  # defensive: only plain compose service names
        esac
        if printf '%s' "$rpd_service" | grep -qE "$WATCHER_ONESHOT_PATTERN"; then continue; fi
        if printf '%s' "$rpd_service" | grep -qE "$WATCHER_EPHEMERAL_PATTERN"; then continue; fi
        # A stopped service is check-service-revival.sh's DOWN class, not a
        # drift comparison — there is no live RestartPolicy worth comparing
        # against on a container that is not running.
        [ "$rpd_state" = "running" ] || continue

        rpd_declared=$(printf '%s' "$declared_json" | jq -r --arg s "$rpd_service" 'if .services[$s] == null then empty else (.services[$s].restart // "no") end' 2>/dev/null)
        [ -n "$rpd_declared" ] && [ "$rpd_declared" != "null" ] || continue

        if ! rpd_live=$(docker inspect --format '{{.HostConfig.RestartPolicy.Name}}' "$rpd_cid" 2>/dev/null); then continue; fi
        [ -n "$rpd_live" ] || continue

        rpd_drift=0
        [ "$rpd_live" = "$rpd_declared" ] || rpd_drift=1
        printf 'goat_mgmt_host_restart_policy_drift{service="%s",site="%s",declared="%s",live="%s"} %d\n' \
          "$rpd_service" "$SITE_LABEL" "$rpd_declared" "$rpd_live" "$rpd_drift"
      done < <(docker ps -a --filter "label=com.docker.compose.project=mgmt-host" \
                 --format '{{.Label "com.docker.compose.service"}} {{.State}} {{.ID}}' 2>/dev/null)
    fi

    # F-472 — a run timestamp, so staleness can be measured at all.
    #
    # A staleness alert cannot compute `time() - max(compose_service_running)`:
    # that subtracts a 0/1 gauge from a Unix timestamp and is true forever, so a
    # dead watcher goes unnoticed. This is the series that expression needs. Written last, so a value
    # here means every read above completed this cycle.
    echo "# HELP compose_watch_last_run_timestamp_seconds Unix time of the last completed mgmt-host-compose-watch metrics cycle."
    echo "# TYPE compose_watch_last_run_timestamp_seconds gauge"
    printf 'compose_watch_last_run_timestamp_seconds{site="%s"} %d\n' "$SITE_LABEL" "$(date +%s)"
  } > "$tmp" 2>/dev/null
  if [ -s "$tmp" ]; then
    mv -f "$tmp" "$METRICS_FILE"
  else
    rm -f "$tmp"
  fi
}

while :; do
  # Block 78E — emit metrics every cycle so node_exporter scrape sees fresh
  # state. Done first so even if compose ps fails on revive path below,
  # at least the metric reflects current truth.
  emit_metrics

  # `docker compose ps --status=exited --format=...` lists exited services;
  # restart=no, restart=on-failure containers that genuinely exited count
  # here. We revive each one. Containers in 'restarting' or 'created' state
  # are NOT exited — leave Docker to handle those.
  if ! exited=$(docker compose "${COMPOSE_FILES[@]}" ps --status=exited --format '{{.Service}}' 2>/dev/null); then
    log "WARN: docker compose ps failed (transient docker daemon issue?); will retry"
    sleep "$POLL_INTERVAL"
    continue
  fi

  if [ -n "$exited" ]; then
    # Sort -u to dedupe in case the same service has multiple exited containers
    for service in $(echo "$exited" | sort -u); do
      # Allowlist check: when set, skip services not in the list.
      if [ -n "$WATCH_SERVICES" ]; then
        case " $WATCH_SERVICES " in
          *" $service "*) ;;  # matched — revive
          *) continue;;       # not in allowlist — skip silently
        esac
      fi
      log "service '$service' exited; running compose up -d $service"
      if out=$(docker compose "${COMPOSE_FILES[@]}" up -d "$service" 2>&1); then
        log "service '$service' revived: ${out//$'\n'/ | }"
      else
        log "ERROR reviving service '$service': ${out//$'\n'/ | } — will retry next poll"
      fi
    done
  fi

  sleep "$POLL_INTERVAL"
done
