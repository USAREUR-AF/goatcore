# Roadmap

What the goat maintainers are working on, and in what order.

- **Now:** this month.
- **Next:** this quarter.
- **Later:** after that.

Each item links to a public issue, so you can follow it or help. The [project board](https://github.com/orgs/USAREUR-AF/projects/8) shows the same items. The maintainers review this file every two weeks, and record the date here.

Last reviewed: 2026-11-07.

Want something that isn't here? Open a feature request, or ask in [Discussions](https://github.com/USAREUR-AF/goatcore/discussions). Security problems go through [SECURITY.md](SECURITY.md), never a public issue.

## Now

- **The same CI safety checks in every public repository:** pinned actions, least-privilege permissions, a DCO check and Dependabot ([#103](https://github.com/USAREUR-AF/goatcore/issues/103)).
- **Issue and pull request templates for every repository** ([#105](https://github.com/USAREUR-AF/goatcore/issues/105)).
- **GOVERNANCE.md in every repository** ([#109](https://github.com/USAREUR-AF/goatcore/issues/109)).
- **Privately reported security issues.** Fix them, give advance notice to registered operators, and publish the advisories with credit to the reporters.

## Next

- **OpenSSF Scorecard in every public repository** ([#104](https://github.com/USAREUR-AF/goatcore/issues/104)).
- **A CONTRIBUTING.md that matches each repository** ([#106](https://github.com/USAREUR-AF/goatcore/issues/106)).
- **Signed provenance and an SBOM for every release and container image** ([#107](https://github.com/USAREUR-AF/goatcore/issues/107)).
- **A CHANGELOG in every repository that publishes** ([#108](https://github.com/USAREUR-AF/goatcore/issues/108)).
- **Contributors can see what their changes do:**
  - an end-to-end check that signs up, logs in, enrolls a machine and publishes;
  - dashboards contributors can read.
- **Partner sharing:**
  - guidance for request and reply over slow international links;
  - a pod that creates its own key and sends only a signing request;
  - revoking one sensor without cutting off its pod;
  - a written way to replace a failed edge.

## Later

- **Fuzzing for the protocol parsers** ([#110](https://github.com/USAREUR-AF/goatcore/issues/110)).
- **Island mode:** sites keep operating when the management plane is unreachable (ADR 0206).
- **Reproducible builds** for every release artifact.
- **Test coverage targets.**

## Not planned

<!-- Maintainers: list what the project has decided not to do, each with its reason. -->
