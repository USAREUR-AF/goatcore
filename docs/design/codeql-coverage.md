# CodeQL coverage

Built: `.github/workflows/codeql.yml` uses the same Go, Python and
JavaScript/TypeScript analysis categories for every triggered pull-request,
main, scheduled and manual run. Go traces builds of all tracked modules on
both branches. Python and JavaScript/TypeScript use source extraction.

Selecting only changed languages on a pull request leaves configurations
present on main without matching results. GitHub then reports incomplete
coverage, even when the selected language analysis succeeds. Language
selection within a run must preserve the baseline categories.

The workflow has no path filter. It runs for every pull request and every
push to `main`, whatever files changed, so that every merged commit
carries a code-scanning result. A run that does not start leaves its commit
with no result, and a reader of the commit cannot tell that case from a
commit that was never analyzed. A whole run takes about three minutes, most
of it the Go builds; the workflow is not a required check and runs beside the
others. `ops/ci/unfiltered-pr-workflows.txt` lists it with this reason.

A Go module that fails to build still emits a warning and is excluded from
extraction, as before. This change does not strengthen that existing behavior
or turn off the separately managed Code Quality scan.

The seed-retry fixture intentionally renders SQL and shell with HTML escaping
disabled. It produces no browser content. Its Jinja XSS alert is a false
positive; enabling HTML escaping would alter SQL literals and shell quoting.
