# saltlick role tests

Parse-only smoke. Catches YAML + Jinja2 errors at PR-time. Does not pull the
image or start the directory; the role's own last tasks (`/healthz`, `/v1/spec`)
are the acceptance on a real host.

Run: `./syntax-check.sh`.
