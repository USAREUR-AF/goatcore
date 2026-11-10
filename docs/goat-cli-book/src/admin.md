# Administration

`goat admin` is the operator surface: onboarding and revoking participants, rotating certificates, cutting a misbehaving peer off the mesh, provisioning devices, and LAN-side triage. It sits apart from the everyday commands on purpose.

> **Operator-only.** These commands are for operators who hold admin credentials. If you're a participant publishing and subscribing on the fabric, this chapter isn't your day-to-day — your surface is **[Everyday tasks](everyday.md)** (`goat pub` / `sub` / `top` / `who` / `health`). `goat admin` is **inert for non-admins**: an unauthorized call changes nothing and tells you so.

## What "operator-only" means in practice

The authority isn't in the CLI — it's in *your credentials*. `goat` itself is a thin client; whether you may onboard an org or eject a peer is decided by what your operator profile and environment carry, and confirmed by the portal and the NetBird management API. So an admin command can stop in two honest places:

- **Up front, before anything is sent.** If your machine has no operator setup at all — no initialized profile and no operator environment — `goat admin` refuses immediately with `E_NOT_ADMIN`. Nothing leaves your laptop.
- **Server-side, at the action.** If you do have credentials but they aren't authorized for this action, the portal or NetBird returns a refusal. A refused *mutation* means nothing changed — and `goat` reports it as `E_NOT_ADMIN`, not as a vague token error.

Two error codes you'll want to tell apart:

| Code | Meaning | What to do |
|---|---|---|
| `E_NOT_ADMIN` | Your credential is **valid but not authorized** for this action (or no operator credential is configured). | Confirm you're on your operator profile and have sourced your operator environment. If you still can't, this action isn't for your role — contact your goat administrator. Re-onboarding won't change it. |
| `E_TOKEN_INVALID` | Your credential is **expired or bad** (HTTP 401). | Re-onboard to get a fresh token. |

Reading about admin is never gated: `goat admin --help` and `goat admin <cmd> --help` always work, in every language. Only *running* an action is checked.

## The commands

Add `--help` to any of these for the full flag set, or see the **[Command reference](reference.md)** — the details there are generated from the binary, so they always match what you're running.

```text
goat admin onboard <handle> [--env <goatnet>]      # enroll a participant from their manifest
goat admin onboard <handle> --revoke               # revoke one participant (reversible)
goat admin onboard --org <id> --revoke             # revoke a whole affiliation org (cascade)
goat admin rotate <anchor|alias>                   # re-issue a peer's cert in place (same identity)
goat admin org create <prefix>                     # register a new org in the portal
goat admin org delete <prefix>                     # revoke an org
goat admin peer ls                                 # list mesh peers
goat admin peer cut <selector>                     # eject a peer (the authoritative mesh-first cut)
goat admin client onboard <handle>                 # mint a device (goat-client) bundle
goat admin operator onboard <handle>               # provision the backend-operator tier
goat admin lan scan | doctor                       # LAN-side triage for a relocated mgmt host
```

A few notes:

- **`lan` is the break-glass diagnostic.** `scan` and `doctor` mutate nothing, need no credentials, and are built to work when the mesh and every service are down — so they're the one part of `goat admin` that isn't refused for a missing operator environment.
- **`rotate` keeps identity fixed.** It re-issues the leaf certificate for the same anchor, alias, write namespace, and ACL — only the cert and key are new. Use it for renewal or re-keying, not for changing who someone is.
- **`peer cut` is the real cut.** On the brokerless fabric a certificate is inert without a mesh path, so removing the *peer* is what stops a flooder — `goat health` finds it, `goat admin peer cut` ejects it.
- **Destructive actions confirm first.** `org delete`, `peer cut`, and the `--org` cascades print what they're about to do and ask before acting (`--yes` to skip, `--dry-run` to preview).

## If you see `E_NOT_ADMIN`

Nothing was changed. Walk through it:

1. **Are you on your operator profile?** `goat context use <name>` selects it; `goat profile show` confirms which one is active.
2. **Have you sourced your operator environment?** Admin actions read operator credentials from the environment (for example `GOAT_OPERATOR_HANDLE` and the NetBird admin token) — never from the profile on disk.
3. **Should you have this access at all?** If you've checked the above and the portal still refuses, your role isn't authorized for this action. Contact your goat administrator — re-onboarding the same identity won't grant it.

## Your goatnet's ACL declaration

`goat acl test` and `goat warrant` check access against your goatnet's declaration: the two files `netbird_acl_taxonomy.yml` and `netbird_acl_matrix.yml`. goat reads them from the first of:

1. `--declaration <dir>` (`--acl-declaration <dir>` on `goat warrant apply`);
2. the directory `GOAT_ACL_DECLARATION_DIR` names;
3. the active context: the directory its `profile.toml` names as `acl_declaration_dir`, else its own `acl/` directory, which `goat context refresh` fills from `get.<site>/acl/` when the goatnet serves its declaration there;
4. `ops/ansible/` of the checkout or deployment overlay you run it in.

goat carries no deployment's declaration. With none of these, the command stops with `E_ACL_DECLARATION_MISSING` (`goat explain E_ACL_DECLARATION_MISSING`).

## The full operator workflow

This chapter orients you to the surface and how it behaves. The end-to-end operator workflow — the capability model, setting up your environment and certificates, and the onboard / revoke / rotate / audit loop — lives in the **admin-operator guide** that ships with your goatnet's operations docs, not in this book.
