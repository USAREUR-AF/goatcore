#!/usr/bin/env bats
#
# fastlane resolves a tag to a digest by reading the GHCR packages API through
# the GitHub CLI. Three things have to hold, and none of them did:
#
#   - a gh that fails is a refusal with gh's own reason, not a silent death
#     (the script runs under `set -e`, and a pipeline's status is the
#     pipeline's, so the check that read `$?` could never run);
#   - a gh that succeeds over many pages is a digest, not a refusal (`head -1`
#     closing the pipe mid-fetch made `--paginate` look like a failure, which
#     is likeliest on exactly the busy images --paginate was added for);
#   - a tag goes into the jq program as text, so it has to be a tag.
#
# Each case runs the real script against a fixture repository of its own, with
# a stub gh and --dry-run: no registry is read, no host is touched, and the
# cases do not depend on what this repository's own pins and records say today.
# `fastlane.sh` takes its repository root from `git rev-parse --show-toplevel`,
# so running it from inside the fixture is all it takes.

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  BIN="$BATS_TEST_TMPDIR/bin"
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$BIN"
  DIGEST=sha256:b324d932b1b9c660b06c4ed06f5dc111b5a40b084d4188aa13cb93aaa30f3df6
  fixture_repo
}

# The least a fastlane needs: a component pinned to an image, a role to deploy
# through, and a play to deploy with.
fixture_repo() {
  mkdir -p "$FIXTURE/ops/pins" "$FIXTURE/ops/ansible/roles/widget" "$FIXTURE/ops/goatbuilder"
  printf 'repo:\n  source: USAREUR-AF/goatcore\n  image_namespace: ghcr.io/usareur-af\n' > "$FIXTURE/ops/goatbuilder/goatbuilder.yaml"
  cat > "$FIXTURE/ops/pins/fleet-pins.yaml" <<YAML
components:
  widget:
    var: widget_image
    image_workflow: .github/workflows/widget-image.yml
    fleet: ghcr.io/{{ ghcr_org }}/widget:sha-0000000@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
goatnets: {}
YAML
  printf '%s\n' 'hosts: "{{ target }}"' 'tasks: []' > "$FIXTURE/ops/ansible/widget-deploy.yml"
  ( cd "$FIXTURE" || exit 1
    git init -q
    git add -A
    git -c user.name=t -c user.email=t@t commit -q -m fixture
  ) || return 1
}

fastlane() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" /bin/bash "$REPO/ops/scripts/fastlane.sh" widget \
    --ref "${1:-sha-b7a1621}" --reason "registry read test" --ack "Bats Suite" --dry-run
}

@test "fastlane: a gh that fails refuses with gh's own reason" {
  cat > "$BIN/gh" <<'SH'
#!/bin/sh
echo "gh: HTTP 401: Bad credentials" >&2
exit 4
SH
  chmod +x "$BIN/gh"
  fastlane

  [ "$status" -ne 0 ]
  [[ "$output" == *"fastlane: REFUSE"* ]]
  [[ "$output" == *"cannot read ghcr.io/usareur-af/widget: gh: HTTP 401: Bad credentials"* ]]
  [[ "$output" == *"nothing moved"* ]]
}

@test "fastlane: a gh that fails without a word still refuses, and says so" {
  printf '#!/bin/sh\nexit 1\n' > "$BIN/gh"
  chmod +x "$BIN/gh"
  fastlane

  [ "$status" -ne 0 ]
  [[ "$output" == *"cannot read ghcr.io/usareur-af/widget: gh gave no reason"* ]]
}

@test "fastlane: a busy image resolves to the first digest, not a failure" {
  # More than any pipe will hold (about 150 KiB), because that is the condition
  # the bug needed: a reader that leaves early, and a writer with enough left to
  # write that it notices. 400 lines fit in a pipe buffer on macOS and the script
  # survived there while dying on Linux.
  cat > "$BIN/gh" <<SH
#!/bin/sh
echo "$DIGEST"
i=0
while [ \$i -lt 2000 ]; do
  echo "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  i=\$((i + 1))
done
exit 0
SH
  chmod +x "$BIN/gh"
  fastlane
  echo "$output"   # bats prints this only when an assertion below fails

  # A read that worked: the plan names the digest the first page carried, the
  # registry refused nothing, and the run reached its own end. The exit code is
  # in here because the bug this case exists for killed the script after the
  # digest was already in hand.
  [ "$status" -eq 0 ]
  [[ "$output" == *"would set components.widget.fleet = "*"widget:sha-b7a1621@$DIGEST"* ]]
  [[ "$output" != *"cannot read"* ]]
  [[ "$output" != *"not found under"* ]]
}

@test "fastlane: a tag that is not a tag is refused before it reaches jq" {
  printf '#!/bin/sh\necho %s\n' "$DIGEST" > "$BIN/gh"
  chmod +x "$BIN/gh"
  fastlane 'sha-b7a1621"; whoami'

  [ "$status" -ne 0 ]
  [[ "$output" == *'is not a container tag: a tag is letters, digits, dot, dash or underscore'* ]]
  [[ "$output" == *"Nothing moved"* ]]
}

@test "fastlane: a placeholder --ack is refused" {
  printf '#!/bin/sh\necho %s\n' "$DIGEST" > "$BIN/gh"
  chmod +x "$BIN/gh"
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" /bin/bash "$REPO/ops/scripts/fastlane.sh" widget \
    --ref sha-b7a1621 --reason "registry read test" --ack "<your name>" --dry-run

  [ "$status" -ne 0 ]
  [[ "$output" == *"is still a placeholder"* ]]
}

@test "fastlane: an image pinned under another owner is read and pinned there" {
  # A component whose image another organisation builds is pinned as an
  # absolute ghcr.io/<owner>/<name> ref. Reading it under this checkout's
  # owner finds no package; the stub fails any read that is not that owner's.
  sed -i.bak 's#fleet: ghcr.io/{{ ghcr_org }}/widget:#fleet: ghcr.io/other-org/widget:#' "$FIXTURE/ops/pins/fleet-pins.yaml"
  ( cd "$FIXTURE" && git -c user.name=t -c user.email=t@t commit -qam absolute ) || return 1
  cat > "$BIN/gh" <<SH
#!/bin/sh
case "\$*" in *"/orgs/other-org/packages/container/widget/"*) echo "$DIGEST"; exit 0 ;; esac
echo "gh: Package not found. (HTTP 404)" >&2
exit 1
SH
  chmod +x "$BIN/gh"
  fastlane
  echo "$output"

  [ "$status" -eq 0 ]
  [[ "$output" == *"would set components.widget.fleet = ghcr.io/other-org/widget:sha-b7a1621@$DIGEST"* ]]
  [[ "$output" != *"{{ ghcr_org }}/widget:sha-b7a1621"* ]]
}
