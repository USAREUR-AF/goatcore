# External accounts for offboarding

`ops/scripts/offboard.sh` removes a person from every goatnet and from the
accounts outside the goatnets that a deployment declares here. Decision:
ADR 1272. Design: `docs/design/offboarding-one-driver.md`.

| File | What it is |
|---|---|
| `accounts.yaml` | your deployment's declaration (start from `accounts.yaml.example`) |
| `plugins/<type>` | one executable per account type |
| `plugins/pagerduty`, `plugins/aws-iam` | shipped plug-ins; each file's header shows its declaration |
| `test_plugins.py` | the shipped plug-ins against synthetic peers |

## Writing a plug-in

A plug-in is any executable named for its type. It takes one verb and one JSON
object on standard input, and prints one JSON object.

```
plugins/<type> check  < {"account": {<the declaration>}, "identities": ["<login>", ...]}
plugins/<type> remove < the same
-> {"held": true, "items": ["user 7", "2 devices"], "detail": "one line"}
```

| Rule | Detail |
|---|---|
| `check` reads only | it is called before anything is removed, and again after `remove` |
| `remove` removes everything `check` reports | then the driver calls `check`; anything still held fails the run |
| Match exactly | compare each identity for equality with the service's own login or id field. Never match on a display name or a pattern |
| Fail loudly | exit non-zero with one line on standard error when the service cannot be reached, the credential is refused, or a list is incomplete. A non-zero exit fails the run |
| Credentials | read them from the environment under the names in `credential_env`. Never print them, never write them |
| `items` | short labels of what is held, without the person's login. `held: false` must come with no items |

A plug-in must be safe to run twice: `remove` on an account with nothing held
succeeds and reports `held: false`.
