---
heading: For someone deploying
title: Fixed: federate status resumes from the first phase it cannot show passed
---

`goatbuilder federate status` named the next command from the phase the last
run ended on, plus one. On a journal a later goatbuilder wrote, with phases
recorded as an outcome this build does not know (for example `deferred`), it
advised a `--from` past those phases, although such an outcome never counts as
passed. It now advises `--from` the first phase the journal does not show
passed. After a failure or an operator step, the advice is unchanged.
