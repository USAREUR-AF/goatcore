# Example decoy templates

These templates show the format. They are served only by the synthetic goatnet and by tests.
A goatnet supplies its own templates in its overlay: answers built from these public files
would be recognizable.

| File | Used for |
|---|---|
| `http-*.tmpl` | The responder's answer to a request |
| `forbidden-*.tmpl` | The responder's answer when a decoy credential is presented |
| `doc-<name>.tmpl` | A planted false-documentation file (`template: <name>` in the declaration) |
| `cred-<name>.tmpl` | A planted decoy-credential file |
| `words.txt` | Host words, one DNS label per line |

An `http` or `forbidden` template may begin with `#! status=<code> type=<media type>`.

Fields: `{{host1}}` `{{host2}}` `{{host3}}` (decoy host names), `{{path1}}` `{{path2}}` `{{path3}}`,
`{{token}}` (a decoy credential), `{{user}}`, `{{role}}`, `{{self}}` (the host that was asked),
`{{zone}}`, `{{stop_notice}}`.
