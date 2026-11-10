#!/usr/bin/env bash
# test.sh — the smtp-sink recipe, end to end on a Linux machine with Docker and
# passwordless sudo (CI: .github/workflows/zero-saas-recipes.yml).
#
# 1. negative control: the play on a site that does not declare the sink deploys nothing
# 2. the play, with smtp_sink_enabled: true, brings Mailpit up on a `mgmt` network
#    (the role, as a site runs it, through ops/ansible/smtp-sink.yml)
# 3. the identity console's own mailer (ops/goat-id-console/internal/mailer) sends an
#    invite with an empty SMTP username; the invite, link included, is read back from
#    the sink's API; a username against the same sink is refused (control)
# 4. a second play run changes nothing on the running sink (idempotent)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
fail() { echo "FAIL  $*" >&2; exit 1; }
pass() { echo "ok    $*"; }

docker info >/dev/null 2>&1 || fail "no Docker daemon"
command -v ansible-playbook >/dev/null || fail "ansible-playbook is not installed"
command -v go >/dev/null || fail "go is not installed"

PLAY=(ansible-playbook -i "localhost," -c local "$REPO/ops/ansible/smtp-sink.yml" -e target=localhost)
VARS=(-e smtp_sink_network=goat-smtp-sink-test -e smtp_sink_network_create=true
      -e smtp_sink_smtp_publish=127.0.0.1:1025 -e smtp_sink_ui_publish=127.0.0.1:8025
      -e smtp_sink_dir=/opt/smtp-sink-test)
cleanup() {
  sudo docker compose -f /opt/smtp-sink-test/docker-compose.yml down -v >/dev/null 2>&1 || true
  sudo docker network rm goat-smtp-sink-test >/dev/null 2>&1 || true
  sudo rm -rf /opt/smtp-sink-test
}
trap cleanup EXIT
cleanup

"${PLAY[@]}" "${VARS[@]}" >/dev/null || fail "the play failed on a site with the sink off"
docker inspect goat-smtp-sink >/dev/null 2>&1 && fail "the sink was deployed on a site that did not declare it"
pass "negative control: a site without smtp_sink_enabled gets no sink"

"${PLAY[@]}" "${VARS[@]}" -e smtp_sink_enabled=true || fail "the play failed"
[ "$(docker inspect -f '{{.State.Running}}' goat-smtp-sink)" = true ] || fail "goat-smtp-sink is not running"
docker inspect -f '{{json .NetworkSettings.Networks}}' goat-smtp-sink | grep -q goat-smtp-sink-test \
  || fail "the sink is not on the declared docker network"
pass "the play brought Mailpit up on the declared network"

gt="$(cd "$REPO/ops/goat-id-console" && GOAT_SMTP_SINK_SMTP=127.0.0.1:1025 GOAT_SMTP_SINK_API=http://127.0.0.1:8025 \
    go test ./internal/mailer/ -run 'TestSendInvite_smtpSinkContainer|TestSendInvite_emptyUsernameSendsWithoutAuth' -count=1 -v 2>&1)" \
  || { echo "$gt"; fail "the console's mailer could not deliver to the sink"; }
echo "$gt" | grep -E '^--- ' | sed 's/^/      /'
echo "$gt" | grep -q -- '--- PASS: TestSendInvite_smtpSinkContainer' || fail "the container test did not run (skipped?)"
pass "the console's mailer delivered an invite to the sink and it was read back with its link"

out="$("${PLAY[@]}" "${VARS[@]}" -e smtp_sink_enabled=true)"
echo "$out" | grep -E 'changed=0 ' >/dev/null || { echo "$out" | tail -5; fail "a second run changed something"; }
pass "a second play run changes nothing"
echo "smtp-sink recipe: all checks passed"
