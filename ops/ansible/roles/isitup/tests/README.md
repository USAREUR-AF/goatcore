# isitup role tests

Parse-only smoke. End-to-end browser-auth + active-link rendering is covered
by the goatnet's bringup runbook
(the first deploy of the role) and the operator-fired verification block in
that step.

Run: `./syntax-check.sh`.

`syntax-check.sh` also runs `row-list.py` (which rows reach the page, and the
list stamp in `meta.json`), `free-text-columns.sh`, `front-door-probe.sh`,
`list-freshness.sh` (the list-only play parses, the row list renders with no
sign-in secret, and `ops/scripts/isitup-list.py check` reports a page that is
behind its declaration) and `ops/scripts/test_isitup_list.py`.
`fixture/` is the one-host local inventory and row declaration
`list-freshness.sh` renders from.
