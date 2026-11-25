# CodeQL coverage

Built: `.github/workflows/codeql.yml` uses the same Go, Python,
JavaScript/TypeScript and Rust analysis categories for every triggered
pull-request, main, scheduled and manual run. Go traces builds of all tracked
modules on both branches. Python, JavaScript/TypeScript and Rust use source
extraction.

Rust is the largest language in the repository. GitHub's default setup
analyzed it on 3 and 4 November 2026, and its critical and high alerts were
reviewed then (`docs/security/code-scanning-2026-11-04.md`). The workflow that
replaced default setup listed three languages and left Rust out, so Rust had
no CodeQL result from 4 November until this workflow listed it again. An alert
dismissed in that review stays dismissed: GitHub matches an alert by its rule
and location, not by the configuration that reported it.

C and C++ are not analyzed. The only C file is
`ops/grant-crypto/sdk/capi/include/grant_crypto.h`, a header that cbindgen
generates from the Rust source. There is nothing to compile, so the C/C++
extractor has no input and its one run under default setup failed. That
failed run is the stale `c-cpp` entry on the code-scanning tool status page;
a repository administrator can delete that configuration there.

Selecting only changed languages on a pull request leaves configurations
present on main without matching results. GitHub then reports incomplete
coverage, even when the selected language analysis succeeds. Language
selection within a run must preserve the baseline categories.

The workflow has no path filter. It runs for every pull request and every
push to `main`, whatever files changed, so that every merged commit
carries a code-scanning result. A run that does not start leaves its commit
with no result, and a reader of the commit cannot tell that case from a
commit that was never analyzed. Go, Python and
JavaScript/TypeScript finish in about three minutes, most of it the Go builds.
Rust took 11 to 19 minutes under default setup and has a 50 minute limit; the
others have 30. The workflow is not a required check and runs beside the
others. `ops/ci/unfiltered-pr-workflows.txt` lists it with this reason.

A Go module that fails to build still emits a warning and is excluded from
extraction, as before. This change does not strengthen that existing behavior
or turn off the separately managed Code Quality scan.

The seed-retry fixture intentionally renders SQL and shell with HTML escaping
disabled. It produces no browser content. Its Jinja XSS alert is a false
positive; enabling HTML escaping would alter SQL literals and shell quoting.
