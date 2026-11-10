# goat-get

Serves release assets inside a goatnet at `get.<site>.<zone>`: the goat CLI
binaries at the root, and opt-in families under subdirectories (`sdk/`,
`kmod/`, `octo-pod/`, `goatbuilder/`). Design: `docs/design/goat-cli-mesh-download.md`.
Playbook: `ops/ansible/goat-get-deploy.yml`.

## Where the assets come from

The goat CLI assets come from the download mirror (`goat_get_mirror_base`)
when it carries the complete set for the tag, otherwise from the GitHub
release. Every other family comes from its GitHub release. Integrity does not
depend on the source: the `.sha256` sidecars and cosign bundles travel with
the assets.

## GitHub access: token or anonymous

Each GitHub release is read through `ops/ansible/tasks/github-release-access.yml`,
which decides once per family whether requests carry the token:

| Token | Repository | GitHub answer to the token | Result |
|---|---|---|---|
| empty | public | n/a | read anonymously, no `Authorization` header |
| empty | private (`*_release_private: true`) | n/a | fails: the token is required |
| set | any | 200 | read with the token |
| set | public | 401 or 403 | warning naming the refused token, then read anonymously |
| set | any | 404 | fails: on a private repository the token lacks `contents:read`, or the tag is not cut |

The token is `goat_get_github_token`: `GOAT_CLI_RELEASE_TOKEN`, else
`GH_TOKEN`, else `GHCR_READ_TOKEN` from the operator's environment. It is
never written to the host and no task prints it.

| Family | Repository | Private flag (default) |
|---|---|---|
| goat CLI | `goat_get_release_repo` | `goat_get_release_private` (false) |
| SDK | `goat_get_sdk_release_repo` | `goat_get_sdk_release_private` (follows the CLI flag) |
| kmod | `goat_get_kmod_release_repo` | `goat_get_kmod_release_private` (false) |
| octo-pod | `goat_get_octopod_release_repo` | `goat_get_octopod_release_private` (false) |
| goatbuilder | `goat_get_goatbuilder_release_repo` | `goat_get_goatbuilder_release_private` (follows the CLI flag) |

The `goat-docs` role reads its releases the same way (`goat_docs_release_private`,
`goat_docs_goatbuilder_release_private`).

## Tests

`tests/github-release-access.yml` runs the shared tasks against a real release
from localhost only; usage is in its header. `tests/syntax-check.sh` is what
CI runs.
