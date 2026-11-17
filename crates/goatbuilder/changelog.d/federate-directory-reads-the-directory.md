---
heading: For someone deploying
title: "Added: `goatbuilder federate directory ls|show|check` reads the peering directory"
---

`goatbuilder federate directory` reads the peering directory (saltlick) and
changes nothing. `ls` lists the directory's release and every party with the
files it hosts; `show <party>` gives each hosted file's size and sha256;
`check <party> --for <counterparty>` checks each file a peering reads and says,
for a missing one, whether the party has not uploaded it (`NOT HOSTED`: the
party runs phase 5) or the directory's release does not serve the route
(`ROUTE MISSING`: its operator moves the directory to a newer release). The
URL is `--url` or `FED_DIRECTORY_URL`. New codes GB-370, GB-371 and GB-372.
