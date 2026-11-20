# Fieldkit Gatepost

Who may use this pod, managed on site with no backhaul. One page, `gatepost.html`, served at `/` by the pod API
(`ops/goat-pod-api`) on the pod's mesh address and on localhost. It signs in with the person's goat certificate or card
(the browser presents it; the pod checks it against its cached CA roots) and calls only the pod API
([`docs/design/pod-carriage-and-provenance.md`](../../../docs/design/pod-carriage-and-provenance.md) §7.4).

| Button | Does | Same as | Pod route |
|---|---|---|---|
| Status | Connected or cut off, rule age, sources sending now, rules in force | `goat pod status` | `GET /v1/status` (+ `GET /v1/rules`, `goat pod rules ls`) |
| Let in | Let a source send through this pod for 4, 24 or 72 hours, with a reason. The source is named by its certificate (`<slot>.g<N>`); a bare slot works only for a source the pod has carried before | `goat pod rules allow <source> --until <time> --reason <text>` | `POST /v1/rules` |
| Shut out | Stop a source sending through this pod at once | `goat pod rules revoke <source> --reason <text>` | `POST /v1/sources/{slot}/revoke` |
| Extend | Give a rule a later end time | `goat pod rules extend <rule> --until <time> --reason <text>` | `POST /v1/rules/{id}/extend` |
| Bring back | Turn an expired or archived rule on again | `goat pod rules reactivate <rule> --until <time> --reason <text>` | `POST /v1/rules/{id}/reactivate` |
| Waiting | Changes made here, waiting for an org lead | `goat pod pending` | `GET /v1/pending` |

Every change shows its result and its command first, asks once, and then shows the pod's answer, including the pod's warning when the change is recorded but its router went back to the last good rules (HTTP 202). The pod enforces the
limits (local authority only, a reason, an end time within the longest rule length); the page holds no rule of its own,
and its 4, 24 and 72 hour choices above the pod's limit are greyed out. A person who is not named as local authority sees
the same page with the change buttons off.

## Run it locally

```bash
python3 ops/fieldkit/gatepost/mock_pod_api.py              # http://127.0.0.1:8790/, cut off, you are local authority
python3 ops/fieldkit/gatepost/mock_pod_api.py --look-only  # signed in as someone who may only look
python3 ops/fieldkit/gatepost/mock_pod_api.py --connected  # the pod reports connected
```

The mock answers the same routes, JSON shapes and error codes as `ops/goat-pod-api`, from synthetic data, and refuses what the pod refuses.

## Edit it

Change `gatepost.html` below its Bleating Edge blocks; change the design system in `../bleating-edge/` and run
`python3 ops/fieldkit/inline.py`. A new action needs a `goat pod` verb first and a row in
`ops/ci/fieldkit-cli-allowlist.txt`; `ops/ci/check-fieldkit-scope.py` refuses it otherwise.
