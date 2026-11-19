# id-console: operator keys are tied to their holder (upgrade note)

This change to goat-id-console ties every per-person operator key to the person who holds it, and refuses the key once that person is offboarded, deactivated or demoted. It closes the gap where an admin who was removed from a goatnet kept working through the machine realm (`/api/*`).

## What changes on upgrade

- **The live machine-realm gate.** On every `/api/*` request made with a per-person operator key, the console reads the key holder's identity from Kratos. It refuses the key (403, audited as `operator_key_owner_refused`) when any of these is true:
  - the identity is inactive;
  - the identity is missing;
  - the identity holds no group with admin access to the console;
  - the key cannot be tied to exactly one person.

  This is on when `ADMIN_LIVE_IDENTITY_CHECK` is true, which is the default.
- **Service keys are exempt.** A service key is one under one of these handles: `operator`, `onboard-api`, `id-console`, `onboard-runner`, `onboard-runner-<site>`, or any handle listed in the new `ONBOARD_SERVICE_OPERATOR_HANDLES` (comma-separated).
- **Keys are bound to an identity id.** A key collected after the upgrade has its holder's Kratos identity id stored on the row, in the new column `operator_keys.holder_identity_id`. The gate reads that identity by id. A key collected before the upgrade is tied through its key row's email, or failing that through the settled onboarding records under its handle. When that evidence names two people, the key is refused.
- **Offboards revoke keys.** Deactivating, deleting or demoting an identity from the console, or offboarding its onboarding record, revokes the person's operator keys in the same operation.
- **A handle names one person.**
  - `/api/onboard/intake` and `/api/onboard/intake/batch` refuse a manifest whose `identity.handle` is already held by a live onboarding record for a different address (409 `handle_taken`, or a batch finding).
  - The public intake form never took a handle, and the submit endpoint now ignores one sent in the JSON. The handle is derived from the email.
- **The operator door needs a roster person.** A key requested through `/api/onboard/operator-keys` with an operator key must name a person on the onboarding roster. The row is filed under that person's handle and email.

## Action before upgrading

- **Check `ONBOARD_OPERATOR_API_KEYS`.** Every per-person key pinned there must be filed under its holder's onboarding handle, exactly as the roster spells it. A key whose handle names no person stops working on upgrade.
  - After the upgrade, the console logs one warning per such key at startup.
  - The list is also served unauthenticated at `GET /healthz/operator-keys`, as `{"unrostered_operator_keys": [...], "count": n}`.
  - Either rename the key to the holder's roster handle, or, if it belongs to a service, list its handle in `ONBOARD_SERVICE_OPERATOR_HANDLES`.
- **Check for duplicate handles.** Look for two live onboarding records that share a handle for different people. Until an admin settles which record holds the handle (archive or reject the other), a key under that handle collected before the upgrade is refused. A key collected after it is not affected, because it is bound by identity id.

## Schema

- **New column:** `operator_keys.holder_identity_id TEXT NOT NULL DEFAULT ''`.
- **Existing databases:** the console adds the column at startup.
- **SQLite migration:** the SQLite schema used by `storemigrate` carries it too.
