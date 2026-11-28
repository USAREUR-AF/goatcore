---
heading: For someone deploying
title: "Fixed: `federate allocate` says when the pin it reads is declared absent for the peering's environment"
---

When a site value such as `goat_fed_republisher_image` cannot be derived,
GB-232 lists where `goatbuilder federate allocate` looked. A component that a
deviation declares absent for the peering's environment gives no value, and a
goatlite's render declares every component above its tier absent, the
republisher among them. GB-232 listed only that the component's pin was read,
which looked like a value it should have found. It now says the component is
declared absent for that environment, and names the group_vars in which to set
the variable to run it for the peering.
