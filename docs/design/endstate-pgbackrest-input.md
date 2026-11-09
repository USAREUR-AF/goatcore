# Backup acceptance applicability

Status: built. Declare pgBackRest in the endstate service catalog so acceptance
follows `has_durable_independent_state`, the same declaration that controls the
role in `site-bootstrap.yml`. A site that explicitly disables independent backup
state marks the pgBackRest acceptance token absent; a site with backups, including
the default configuration, still has to pass the backup acceptance row.

The three regression tests exercise endstate generation and the acceptance
runner's absent-token selection. This change does not resolve credentials, run a
backup, restore data or prove that an alternative durability mechanism works.
No live deployment is claimed.
