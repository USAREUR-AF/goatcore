---
heading: For someone contributing
title: Changed: a documents-only pull request runs only the documents' configuration examples
---

`goatbuilder-pr` decides from the pull request's own diff whether it changes
documents only (`ops/ci/goatbuilder-pr-scope.sh`). A documents-only pull
request then runs the check that every whole configuration example in the
documents validates, and not the whole build, test and macOS jobs; a change
to the tutorials or to anything else still runs everything (#685).
