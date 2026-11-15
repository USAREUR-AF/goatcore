#!/usr/bin/env bash
# test-apt-source.sh: docker-engine adds Docker's apt repository for the
# distribution the host runs, on Debian as on Ubuntu.
#
# It runs the role's own repository tasks, from "Detect existing Docker
# install" through "Refresh apt cache after adding Docker source", taken from
# tasks/main.yml by name, in a debian:13 and an ubuntu:24.04 container. Then it
# checks three things against the container's /etc/os-release, not against
# Ansible's facts, so the check does not just repeat what the role computed:
#   1. the source line names download.docker.com/linux/<ID> <VERSION_CODENAME>;
#   2. apt refreshed against it (the role's own refresh task succeeded);
#   3. apt offers docker-ce from download.docker.com.
# A last leg runs the same tasks on Debian with the repository fixed to
# "ubuntu" and expects them to fail, so the test is seen to catch the defect.
#
# Needs docker (no root: the containers are the targets), ansible-playbook and
# the community.docker collection (ops/ansible/requirements.yml), and network
# access to the distribution mirrors and download.docker.com.
#
# Exit codes: 0 pass, 1 fail. A missing prerequisite skips with 0 locally and
# fails under CI (CI=true), so the CI step cannot pass by skipping.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGES="debian:13 ubuntu:24.04"
FIRST_TASK="Detect existing Docker install (skip work if present + healthy)"
LAST_TASK="Refresh apt cache after adding Docker source"

CONTAINERS=""
WORKDIR=""
cleanup() {
  for c in $CONTAINERS; do
    docker rm -f "$c" >/dev/null 2>&1 || true
  done
  if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    rm -rf "$WORKDIR"
  fi
}
trap cleanup EXIT

say() { echo "docker-engine apt-source: $*"; }
fail() { say "FAIL: $*" >&2; exit 1; }
skip() {
  if [ "${CI:-}" = "true" ]; then
    fail "prerequisite missing under CI: $1"
  fi
  say "SKIP: $1" >&2
  exit 0
}

command -v docker >/dev/null 2>&1 || skip "docker not on PATH"
docker info >/dev/null 2>&1 || skip "docker daemon not reachable"
command -v ansible-playbook >/dev/null 2>&1 || skip "ansible-playbook not on PATH"
command -v python3 >/dev/null 2>&1 || skip "python3 not on PATH"
ansible-doc -t connection community.docker.docker >/dev/null 2>&1 \
  || skip "community.docker collection not installed (ops/ansible/requirements.yml)"

WORKDIR="$(mktemp -d -t docker-engine-apt-source.XXXXXX)"

# The role's repository tasks, by name, so a renamed or reordered task fails
# here instead of the test quietly running something else.
python3 - "$ROLE_DIR/tasks/main.yml" "$WORKDIR/repo-tasks.yml" "$FIRST_TASK" "$LAST_TASK" <<'PY'
import sys, yaml
src, dst, first, last = sys.argv[1:5]
tasks = yaml.safe_load(open(src))
names = [t.get("name") for t in tasks]
for n in (first, last):
    if n not in names:
        sys.exit("docker-engine apt-source: FAIL: task not found in tasks/main.yml: %s" % n)
i, j = names.index(first), names.index(last)
if j < i:
    sys.exit("docker-engine apt-source: FAIL: %r comes before %r" % (last, first))
yaml.safe_dump(tasks[i:j + 1], open(dst, "w"), sort_keys=False)
PY

cat >"$WORKDIR/play.yml" <<EOF
- hosts: all
  gather_facts: true
  vars_files:
    - $ROLE_DIR/defaults/main.yml
  tasks:
    - ansible.builtin.include_tasks: $WORKDIR/repo-tasks.yml

    - name: os-release, read directly
      ansible.builtin.command: sh -c '. /etc/os-release && echo "\$ID \$VERSION_CODENAME"'
      register: osr
      changed_when: false

    - name: the Docker source line
      ansible.builtin.slurp:
        src: "{{ docker_engine_apt_sources_list }}"
      register: source_list

    - name: apt's docker-ce candidate
      ansible.builtin.command: apt-cache policy docker-ce
      register: policy
      changed_when: false

    - name: the source names this host's distribution and codename, and apt offers docker-ce from it
      vars:
        want: "https://download.docker.com/linux/{{ osr.stdout.split()[0] }} {{ osr.stdout.split()[1] }} stable"
        line: "{{ source_list.content | b64decode }}"
      ansible.builtin.assert:
        that:
          - want in line
          - "'download.docker.com/linux/' ~ osr.stdout.split()[0] in policy.stdout"
          - "'Candidate: (none)' not in policy.stdout"
        fail_msg: "want '{{ want }}' in '{{ line | trim }}'; apt-cache policy: {{ policy.stdout }}"
        success_msg: "{{ osr.stdout }}: {{ line | trim }}"
EOF

start() {
  # $1 image, $2 name. The image has no python3; Ansible's docker connection needs it.
  CONTAINERS="$CONTAINERS $2"
  docker run -d --name "$2" "$1" sleep infinity >/dev/null
  docker exec -e DEBIAN_FRONTEND=noninteractive "$2" sh -c \
    'apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 >/dev/null' \
    || fail "$1: could not install python3 in the container"
}

play() {
  # $1 container; the rest go to ansible-playbook.
  c="$1"; shift
  ANSIBLE_HOST_KEY_CHECKING=False ANSIBLE_RETRY_FILES_ENABLED=False \
    ansible-playbook -i "$c," -c community.docker.docker \
    -e ansible_python_interpreter=/usr/bin/python3 "$@" "$WORKDIR/play.yml"
}

n=0
for image in $IMAGES; do
  n=$((n + 1))
  c="docker-engine-apt-source-$$-$n"
  say "$image: starting"
  start "$image" "$c"
  if play "$c" >"$WORKDIR/$n.log" 2>&1; then
    say "$image: ok, $(grep -o '"msg": "[^"]*"' "$WORKDIR/$n.log" | tail -1)"
  else
    cat "$WORKDIR/$n.log" >&2
    fail "$image: the role's repository tasks or the check failed"
  fi
done

# The defect this guards: a repository fixed to one distribution. On Debian,
# Docker's Ubuntu path has no release for the Debian codename, so the role's
# refresh task must fail.
c="docker-engine-apt-source-$$-wrong"
say "debian:13 with the repository fixed to ubuntu: expecting failure"
start debian:13 "$c"
if play "$c" -e docker_engine_apt_distro=ubuntu >"$WORKDIR/wrong.log" 2>&1; then
  cat "$WORKDIR/wrong.log" >&2
  fail "debian:13 with docker_engine_apt_distro=ubuntu passed; the test cannot see the defect"
fi
failed_task="$(awk '/^TASK \[/ { t = $0 } /^fatal:/ { print t; exit }' "$WORKDIR/wrong.log")"
case "$failed_task" in
  "TASK [$LAST_TASK]"*) ;;
  *)
    cat "$WORKDIR/wrong.log" >&2
    fail "the fixed-to-ubuntu leg failed at '$failed_task', not at the role's refresh task"
    ;;
esac
say "debian:13 with the repository fixed to ubuntu: failed, as it must"

say "PASS: Docker's apt source follows the distribution on $IMAGES"
