---
heading: For someone contributing
title: Added: CI holds the deprecation period once the last release is 1.0
---

The compatibility check (`ops/ci/check-goatbuilder-compat.sh`, in
goatbuilder-pr) now enforces ADR 1228's deprecation period. Once the last
release is 1.0 or later, a deprecation entry excuses a removed command, flag
or value only when all of these hold: the build is a later major release
whose version is not yet tagged; the release the entry names shipped a
deprecation list that names the item, so its binary warned; two minor
releases, of any major, have been tagged since that release and before the
build's version; and 90 days have passed since that release was tagged. A refusal names the date it compared
against (today, UTC) and the date the 90 days end. Before 1.0 an entry
excuses a removal at once, as before. goatbuilder-pr fetches the release tags
for this check, and reads each named release's list from its tag (#800).
