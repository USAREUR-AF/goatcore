---
heading: For someone deploying
title: "Added: `promote` and `fastlane` take a release component"
---

`promote open`, `promote hotfix` and `fastlane` take a release component
(`artifact_type: release`) at its whole pin,
`<owner>/<repository>@<tag>@sha256:<digest>`, and refuse any other ref; a release
pin is refused for an image component. `promote check` refuses an open record or
a graduation-set member of the wrong kind. `fastlane` fetches and verifies the
release before it moves anything (GB-381).

A file has no container, so a `runtime-health` gate cannot be captured for one:
`promote graduate` says so, and passes it only with `--override-runtime-health
"<reason>" --ack "<name>"`, recorded on the record. `graduate.py` and
`fastlane.sh` do the same, in the same words.
