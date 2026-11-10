# Repository Guidelines

Instructions for AI coding agents, and a short version for anyone working here. [CONTRIBUTING.md](CONTRIBUTING.md) is the full process.

## What this repository is

goatcore is the goat substrate: the code that builds and runs a goatnet. That covers the NetBird/WireGuard mesh control plane, the Zenoh data fabric, identity (Ory, SPIFFE), the `goat` and `goatbuilder` command-line tools, and the Ansible roles and Terraform modules that deploy them.

A real deployment is goatcore at a pinned commit plus a separate overlay repository that holds that deployment's configuration. No deployment's configuration is ever in this repository. The sibling repositories:

| Repository | Holds |
|---|---|
| [goatstuff](https://github.com/usareur-af/goatstuff) | inspection tools and demonstration services that run on a goatnet |
| [goatdocs](https://github.com/usareur-af/goatdocs) | architecture, strategy, design system, guides |
| [goatenv](https://github.com/usareur-af/goatenv) | a complete example environment, the synthetic goatnet `zagros` |
| [hello-galaxy-goatenv](https://github.com/usareur-af/hello-galaxy-goatenv) | the tutorial worked example and its checkpoints |
| [goatrodeo](https://github.com/usareur-af/goatrodeo) | games that run on the fabric |

## Layout

| Path | What |
|---|---|
| `crates/` | Separate Rust workspace: `goat-cli` (the `goat` CLI), `goatbuilder`, `goat-errors`, `panoscope-core` |
| `goat-relay/`, `goat-relay-protocol/` | separate root Rust workspace for the relay and wire protocol |
| `signal-server/` | Go signalling service with its own module and tests |
| `tests/` | shell and Python regression suites |
| `ops/ansible/` | roles and playbooks; `group_vars/_examples/` shows the shape of a deployment's values |
| `ops/<component>/` | services and tools, mostly Go and Python, each with its own tests |
| `ops/ci/` | every CI gate, as a script you can run locally |
| `terraform/modules/` | reusable cloud modules; a deployment's roots live in its overlay |
| `docs/design/` | one design document per component, with build status per part |
| `docs/adr/` | decisions; `docs/adr/INDEX.md` lists them |
| `docs/tutorials/` | start with `00-goatlite.md` |

## One build for every deployment

A tool or image built from this repository is the same for every deployment that runs it. Nothing about a deployment is compiled into a binary, baked into an image or used as a default, not even as a fallback: no names, domains, addresses, repositories, ACL tables, certificate authorities or display text. Each value comes from the deployment's configuration: the goat context and its `goatnet.toml`, Ansible group_vars, an environment variable, or a file mounted at deploy time. When the configuration is missing, the tool stops with an error from `ops/errors/catalog.yaml` that names what to supply. A default is allowed only when it belongs to no deployment, such as an empty list or a port number. A deployment never builds its own copy of a tool or image; its overlay repository holds configuration only.

## Rules CI enforces

1. **No identifier of a real deployment.** Hostnames, domains, addresses, account ids, keys, people, places and organisations of any real goatnet never enter a file or a commit message. Use the example goatnet `zagros`, documentation addresses (RFC 5737, and `100.127.0.0/16` for mesh), and `example.org` mail. The leak gate (`.github/workflows/leak-gate.yml`) checks every added line against a private list of deployment names. It never prints what matched, so a red leak gate means: rename the identifier. Locally, `ops/ci/check-no-identity-spillage.sh` and `ops/ci/check-no-live-secret.sh` catch keys, addresses and credential shapes; the name list itself is not available outside CI.
2. **Commits.** Conventional Commits subject of 100 characters or fewer, with a scope (`fix(goatbuilder): …`). Every commit is signed off (`git commit -s`, the DCO) and signed with a key GitHub verifies; the required check `verified-commits` refuses a pull request holding any unverified commit (ADR 1226). One logical change per pull request.
3. **No assistant session links.** `ops/ci/check-no-agent-references.sh` refuses assistant session URLs and session trailers outside `CLAUDE.md` and `.claude/`.
4. **Design documents move with the code.** A change to behaviour, data flow, topology or an integration point updates its `docs/design/` document in the same pull request, using the status words built, measured, designed and not built exactly.
5. **ADRs are never rewritten.** A new decision supersedes an old one, and its row goes into `docs/adr/INDEX.md` in the same change.

## Before you push

Run the gates for what you touched; the table in [CONTRIBUTING.md](CONTRIBUTING.md#gates) maps each kind of change to its script. The common ones:

```bash
cd crates && cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
bash ops/ci/check-no-identity-spillage.sh
bash ops/ci/check-bash-portability.sh        # shell runs on bash 3.2
```

A red check names its script, so every failure reproduces locally.

### Build and validate the affected component

- In `crates/`, `cargo build --release -p goatbuilder` builds the deployment CLI. The root README’s mock `standup` and `status` example rehearses a deployment locally.
- The root and `crates/` Rust workspaces build separately. Run `cargo test --workspace --locked` in the affected workspace; use the applicable workflow’s toolchain and Clippy flags. Relay tests have Linux/kernel requirements: `.github/workflows/goat-relay.yml` documents the privileged-test skip. macOS does not exercise Linux-only paths.
- In a changed Go module, run `go vet ./... && go test ./...`.
- Run affected shell suites with `bats tests/<suite>.bats`. Follow each Python suite’s documented invocation; some files are standalone scripts rather than pytest suites.
- For Ansible, run `ops/ci/check-role-template-refs-resolve.sh` and `ansible-playbook --syntax-check <playbook>`. Role-specific fixtures live under `ops/ansible/roles/<role>/tests/` where provided.
- For docs, run `ops/ci/check-doc-links.py` and `ops/ci/check-doc-code-citations.sh`.

Use `cargo fmt` for Rust, `gofmt` for Go, four-space Python indentation, and two-space YAML indentation. Follow adjacent naming conventions, including Python `test_*.py` and Go `*_test.go`.

Install the tools and dependencies named by the component’s CI workflow before running its checks. Add regression coverage for behavior changes, and report commands, results, and untested gaps in the PR template. Report skips separately from passes: `check-no-live-secret.sh` requires the operator’s configured local environment and skips when no live secret values are loaded.

## Ownership, tracking and review

- Check related open and merged issues and pull requests before starting. Reuse existing work where appropriate, and identify changes that supersede an older proposal.
- Trace affected callers, configuration, tests and sibling repositories. Check the applicable ADRs and their current status; document dependencies and merge order when a change spans repositories.
- Use the repository issue or pull-request template, a scoped Conventional Commit title, appropriate existing labels and the established project. For public product work, use the [public roadmap](https://github.com/orgs/USAREUR-AF/projects/8) and state every affected repository. Do not invent priorities or delivery dates.
- Preserve existing assignees and assign the original author and people doing the work when they are eligible. This includes migrated stories filed by a maintainer on someone else's behalf. Do not change repository access just to make an assignment possible.
- When migrating work, link the replacement from the original tracker. Close superseded or migrated items as such; do not describe migration as a completed fix. Close a fixed report only when evidence covers its acceptance criteria, and leave partially addressed work open.
- Record code merged, tests passed and deployment verified as separate outcomes. Include reproducible commands, results and meaningful gaps; neither a merge nor a successful build proves a running deployment was updated.

## Private reports and public information

Follow [SECURITY.md](SECURITY.md) for suspected vulnerabilities. Keep findings and private operational evidence out of public issues, pull requests and project cards. Check for an existing private report before creating another. Do not publish an advisory or request a CVE without explicit maintainer approval.

Keep public instructions reusable: use synthetic examples and plain, translatable English, without private issue links, deployment details or personal machine paths. Link detailed procedures instead of duplicating them here.

The `security-reviewed` label records the review described in [CONTRIBUTING.md](CONTRIBUTING.md#outside-contributions); it is not a substitute for that review or for CI. A different owner gives the final merge approval. New commits require a fresh review of the updated diff.

## Read first

- [architecture-spine.md](https://github.com/usareur-af/goatdocs/blob/main/docs/design/architecture-spine.md) in goatdocs: the architecture in one read.
- [docs/design/zenoh-brokerless-consequences.md](docs/design/zenoh-brokerless-consequences.md): the data fabric has no central broker, which rules out a class of fixes.
- [docs/adr/INDEX.md](docs/adr/INDEX.md): why things are the way they are.

## Skills

`.claude/skills/` holds task instructions for coding agents, each a `SKILL.md` with a `GUIDE.md` for people: [goat-cli](.claude/skills/goat-cli/SKILL.md) (driving a goatnet with the `goat` CLI) and [adr](.claude/skills/adr/SKILL.md) (proposing an ADR).
