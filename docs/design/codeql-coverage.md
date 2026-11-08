# CodeQL coverage

Built: `.github/workflows/codeql.yml` uses the same Go, Python and
JavaScript/TypeScript analysis categories for every triggered pull-request,
main, scheduled and manual run. Go traces builds of all tracked modules on
both branches. Python and JavaScript/TypeScript use source extraction.

Selecting only changed languages on a pull request leaves configurations
present on main without matching results. GitHub then reports incomplete
coverage, even when the selected language analysis succeeds. Workflow path
filters still avoid runs for unrelated file changes; language selection
within a triggered run must preserve the baseline categories. Full Go builds
increase the cost of Python-only and JavaScript-only pull requests.

A Go module that fails to build still emits a warning and is excluded from
extraction, as before. This change does not strengthen that existing behavior
or turn off the separately managed Code Quality scan.

The seed-retry fixture intentionally renders SQL and shell with HTML escaping
disabled. It produces no browser content. Its Jinja XSS alert is a false
positive; enabling HTML escaping would alter SQL literals and shell quoting.
