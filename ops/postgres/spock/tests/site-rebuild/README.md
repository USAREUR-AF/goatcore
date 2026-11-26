# site-rebuild

The rebuild of a site whose Spock slots were invalidated
([docs/operations/spock-site-rebuild.md](../../../../../docs/operations/spock-site-rebuild.md)),
on three postgres-spock containers in a full mesh. One site is cut off and
writes rows of its own, while its peers write and delete. WAL is written
past a small slot cap until the slots are lost. Then the cut site is rebuilt
from the founder with the eight steps of `ops/scripts/spock-reseed.py`, the
helper `goatbuilder join-node --reseed` runs, and so is the third site, whose
subscription from the cut site was lost too. `run.sh`'s header says what is
written, what is asserted and what is recorded.

```bash
docker build -t postgres-spock:rebuild ops/postgres/spock
IMAGE=postgres-spock:rebuild ops/postgres/spock/tests/site-rebuild/run.sh
IMAGE=postgres-spock:rebuild CAP=128MB OUT=/path/to/record ops/postgres/spock/tests/site-rebuild/run.sh
```

A run writes a few hundred megabytes of WAL and takes a few minutes. It is
not in CI. Linux Docker only, as every multi-site test in this directory.
