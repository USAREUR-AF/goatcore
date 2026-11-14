---
heading: For someone deploying
title: Changed: container memory limits and Postgres memory are settable, and cron is installed
---

Every container memory limit and reservation in the roles and compose files
is now a variable whose default is the value it had, so a small host can
lower them in its group_vars. Postgres `shared_buffers` and
`max_connections` keep the image's values unless
`mgmt_postgres_shared_buffers` or `mgmt_postgres_max_connections` is set;
`mgmt-host.yml` refuses an `mgmt_postgres_max_connections` that differs
from `postgres_max_connections`. The roles that schedule jobs with cron
(`apt-cacher-ng`, `docker-registry-mirror`, `oci-substrate-t2-sync`,
`isitup`, `operator-idp-postgres` and the observability play) now install
`cron` first, so a minimal Debian host runs those jobs (#615).
