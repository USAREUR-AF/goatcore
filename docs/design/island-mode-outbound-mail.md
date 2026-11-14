# Island mode: the outbound mail queue

| Field | Value |
|---|---|
| Status | **Designed, not built.** Step 7 of the island-mode program ([ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md)) |
| Scope | `ops/goat-id-console` (the identity console), the only component that sends mail to a person |
| Related | [island-mode-multi-writer.md](island-mode-multi-writer.md), [island-mode-write-inventory.md](island-mode-write-inventory.md) (X13, and the outbound effects of §4) |

## 1. The job

A site that is cut off from the rest of the goatnet keeps admitting people and enrolling
machines (island mode). Some of what it does sends mail to a person: an invitation, a stage
notice, and the welcome package once a person is onboarded. While the site is cut off, its
mail relay may be unreachable.

Today each send is attempted once. When it fails, the console records the failure and the
mail is lost: nothing retries it, and an operator learns of it only from the record. When it
succeeds while the operator has also handed the person a package by hand, the person gets two.

**Done when** (the program's own words): a cut-off site holds welcome mail and drains it on
reconnect, without duplicating a hand-delivered package.

## 2. What sends mail today

Every send goes through `internal/mailer` (`Send`, `SendMultipart`, `SendInvite`):

| Mail | Sent from | Carries a credential |
|---|---|---|
| Invitation link | `onboarding.go` (two sites), `onboard_api.go` | yes: a one-time invitation link |
| Welcome package | `onboard_welcome.go` (`sendWelcomeEmail`, and the resend) | yes: a recovery link and code, and the setup key, collaboration link or portal key when the run produced them |
| Stage notice | `notify.go` | only the `onboarded` event's setup key |
| Access-change and reference notices | `access_change.go` | no |
| Repository-access request | `github_request.go` | no |
| Operator notice | `onboard_notify_ops.go` | no |

Two sends already have an out-of-band path: the invitation as a `.eml` download
(`POST /onboarding/invite/email/eml`), and the welcome package as a resend
(`POST /onboarding/{id}/welcome/resend`). A hand-delivered package is today invisible to the
mail path.

## 3. The design

### 3.1 One queue, one row per mail

A table `outbound_mail` in the console's own store, one row per mail the console decided to
send:

| Column | Meaning |
|---|---|
| `id` | the row |
| `dedupe_key` | what makes two sends the same mail: the mail kind and its subject, for example `welcome:<proposal id>` or `invite:<invite id>`. Unique |
| `kind` | `invite`, `welcome`, `notice`, `access`, `repository`, `operator` |
| `recipient` | the address |
| `state` | `unsent`, `sent` or `delivered_out_of_band` |
| `attempts`, `last_error`, `next_attempt_at` | the drain's own bookkeeping |
| `created_at`, `sent_at`, `delivered_by` | when it was decided, when it left, and who handed it over by hand |

A send becomes: write the row as `unsent`, then try it at once. Success makes it `sent`; a
failure leaves it `unsent` with the error, and the drain retries it. A mail that cannot be
sent because the mailer is not configured is `unsent` with that reason, as today's record
already says.

### 3.2 What the row does not hold

**No credential is written to the queue.** The row holds what to send and to whom, never the
rendered message. Where a mail carries a credential, the drain renders it again when it sends,
from the console's own state, the way the resend path already does:

- an invitation link is read again from the invitation, and the row is dropped as superseded
  if the invitation was used, revoked or has expired;
- a welcome package mints its recovery link at the moment it is sent, so the link names the
  site that sends it and is fresh (write inventory, X13);
- the setup key, collaboration link and portal key are passed through from the run today and
  never stored. A welcome that needs one of them and cannot get it again stays `unsent`, with
  the reason "needs its setup key", exactly as today's held welcome does; an operator's resend
  with the key delivers it.

So a cut-off site holds the intent to send, never a credential at rest, and the queue adds no
secret to what the console stores.

### 3.3 The queue stays at its site

`outbound_mail` is site-local: it is in no replication set. A site drains only the mail it
decided to send, through its own relay. Another site never sends this site's mail, so a
person never gets the same mail from two sites after a heal, and no recipient list crosses
sites.

### 3.4 Delivered out of band

An operator who hands a person the package by hand (the `.eml` download, a printed package,
a chat message) marks the row `delivered_out_of_band`, with their name, from the console's
onboarding page: one action, `POST /onboarding/outbound-mail/{id}/delivered`. The `.eml`
download marks its own row. The drain never sends a row in that state, so the person gets the
package once, whichever way it reached them. Marking a row that is already `sent` is refused
with the time it was sent: the person has the mail.

### 3.5 The drain

A loop in the console, every minute, takes the `unsent` rows whose `next_attempt_at` has
passed, oldest first, and tries each once. A failure doubles the row's wait, from one minute up
to one hour. The loop sends nothing while the mailer is unconfigured. Reconnecting needs no
signal: the first attempt after the relay answers again sends the mail held since the cut.

A row is never sent twice: the drain takes it with a row lock and marks it `sent` in the same
transaction that the relay accepted it in. A relay that accepted a mail and then failed before
the row was marked is the one case that can send twice; the design accepts it, because the
alternative is mail that is lost.

### 3.6 Seeing the queue

- The console's onboarding page lists the `unsent` rows: recipient, kind, age, last error.
- `goatbuilder endstate` and `doctor` read the count through the console's health surface:
  GB-291 when mail has been held longer than its threshold (default one hour), GB-292 when a
  held welcome needs its setup key and only an operator's resend can deliver it.

## 4. What is not in this step

- Mail is not sent through another site's relay while this site is cut off.
- Retrying a mail the relay accepted but did not deliver (a bounce) is the relay's job.
- The queue does not hold the portal's or other services' mail; the console is the only sender.

## 5. Tests, first

- A send while the relay is down leaves one `unsent` row and sends nothing; once the relay
  answers, the drain sends it once and marks it `sent`.
- The same mail decided twice (the same `dedupe_key`) is one row and one send.
- A row marked `delivered_out_of_band` is never sent, and marking a `sent` row is refused.
- No row holds a setup key, recovery link or code, collaboration link or portal key: a test
  scans every column of every row a full onboarding writes.
- A welcome whose setup key cannot be had stays `unsent` with "needs its setup key", and the
  resend with the key sends it.
- A partition scenario in the harness: a site cut off onboards a person, the welcome is held;
  on heal it is sent once, from that site.

## 6. Status of each part

| Part | Status |
|---|---|
| The queue table, the drain and the out-of-band mark | designed |
| No credential at rest in the queue | designed |
| GB-291 and GB-292 through the console's health surface | designed |
| The partition scenario | designed |
