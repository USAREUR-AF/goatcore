# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# breakglass (ADR 1114), on the small-computer family.
HOST_VARS="platform_family: small-computer"
type_cases() {
  step phase-13 0 "Phase 13" "!fatal:" -- "${JOIN[@]}" --from 13 --to 13
}
