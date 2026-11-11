# goatbuilder portability: no environment or programme name in the tool

| | |
|---|---|
| Status | **Built.** Both phases. |
| Gate | `ops/ci/check-goatbuilder-portable.py`, run by `goatbuilder-pr.yml`; its baseline holds four lines, listed below |

goatbuilder works for any team that points it at its own config, so nothing it prints or does may depend on one deployment's names. [goatbuilder.md](goatbuilder.md) §"What this is NOT" sets that rule: "no goatnet or environment name, no program or goatnet-family name". This document says where each project-specific value now comes from, and how the tool reaches zero names.

## Where a value comes from

A project declares its values in `ops/goatbuilder/goatbuilder.yaml`. Where that file is generated, `gen-config.py` takes `repo:` and `deploy:` from `ops/goatbuilder/project.yml`, `secrets:` from `ops/goatbuilder/posture.yml`, and sites and peerings from `ops/goatbuilder/federation.yml`; a deployment that keeps its values in a private overlay carries its own copies of all three files there. An environment variable overrides a value for one command. A value nobody declared falls back to a name that belongs to no project, or the command refuses when no such name can be right.

| Value | Override for one command | Declared in `goatbuilder.yaml` | Undeclared |
|---|---|---|---|
| GitHub owner and repository | none | `repo.source` (required) | not applicable: validation refuses the config |
| Image organisation on `ghcr.io` | `GHCR_ORG` | `repo.image_namespace` (required) | not applicable; another registry has no `{{ ghcr_org }}` form, so references stay as written |
| AWS profile | `GOAT_AWS_PROFILE` | `deploy.aws_profile` | `default`, the AWS tools' own fallback |
| SSH user on a new machine | none | `deploy.ssh_user`, then `lite.ssh_user` | a plan prints `<deploy.ssh_user>`; a real `join-node` refuses |
| Local state directory | none | `deploy.state_dir` | `~/.goat/state` |
| Stage names offered by `--env` completion | none | `promotion.path[].name` | none offered |
| Secrets store and 1Password account | `GOAT_SECRET_STORE`, `OP_ACCOUNT` | `secrets:` (ADR 1204) | `env-file` |

`crates/goatbuilder/src/settings.rs` reads the deploy values in that order. The public tree's `project.yml` declares the public repository's release source and image namespace and no `deploy:` values; `gen-config.py` itself names no deployment, and without `project.yml` it writes the placeholder `example-org/example-repo`.

## What a person reads

Help text, error messages, the error catalog, tutor lessons and the goatbuilder reference pages name a goatnet only as `<goatnet>` or `<site>`. A lesson that runs against the checkout reads its goatnet and component from that checkout's pins, so it can be completed in any project.

## The gate

`ops/ci/check-goatbuilder-portable.py` counts the lines naming an environment or programme (`zagros`, `nova`, `usareur`, `goat`, `usareur-af`, `goat-maintainer`, `nethost`, `thor`) in everything that reaches a reader:

- `crates/goatbuilder/src/*.rs`, up to each file's tests, with `///` help text included and `//` comments excluded
- `crates/goatbuilder/i18n/en/goatbuilder.ftl`
- `ops/errors/catalog.yaml`
- `docs/reference/goatbuilder/*.md`

`ops/goatbuilder/portability-baseline.txt` holds today's count per file. A file above its count fails and prints the lines. A file below its count fails until the baseline is lowered with `--write`, so the list only shrinks.

Tests, fixtures and CI parity jobs may use real goatnet names: a reader of the tool never sees them.

## Phases

**Phase 1, built.** The additions the rest depend on:

- `settings.rs`, the `deploy:` schema block, and the deploy values in `ops/goatbuilder/project.yml`
- `join-node`: the AWS profile and SSH user are read from the checkout, and a real run refuses without an SSH user
- `adopt`: the image organisation comes from `repo.image_namespace`
- `--env` completion: stage names come from `promotion.path`
- the gate and its baseline

**Phase 2, built:**

- `deploy`, `release`, `doctor`, `federate` and `standup` read the AWS profile and image organisation through `settings.rs`, so `GOAT_AWS_PROFILE` and `GHCR_ORG` work on every verb.
- `deploy.state_dir` holds the local state directory; the CA key path, standup logs and the name-snapshot step read it. Undeclared, it is `~/.goat/state`, which the scripts avenue still writes literally.
- Help text, error messages, the error catalog, the reference pages and the tutor use placeholders. Lessons that run against the checkout take their goatnet, site and component from its `goatbuilder.yaml`.

Four lines stay in the baseline, each naming something real rather than a deployment:

| File | Line | Why it stays |
|---|---|---|
| `crates/goatbuilder/src/standup.rs` | the synthetic-vendors phase | runs the playbook `ops/ansible/zenoh-synth-vendors-slot-pool-deploy.yml` by its file name |
| `crates/goatbuilder/src/update.rs` | the signer when no config can be read | the binary's own publisher; `update` always verifies the signature, so it needs a signer even outside a checkout |
| `docs/reference/goatbuilder/02-install.md` | the `cosign verify-blob` example | goatbuilder's own releases are signed by `USAREUR-AF/goatcore`'s workflow |
| `docs/reference/goatbuilder/11-workflows.md` | the worked example | names the example repository `USAREUR-AF/hello-galaxy-goatenv` |

Done means those four lines are the whole baseline, and every lesson completes in a checkout that declares no name from the gate's list.

Everything a goatnet deploys, such as `ops/ansible/group_vars/`, inventories and the pins, stays out of scope: that is one deployment's config.
