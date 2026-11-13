# spock-hub-directives

Carries out the goatnet hub's directives at this site ([ADR 1205](../../../../docs/adr/1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md)).

The hub is the goatnet's highest-echelon site, named by `goatnet_hub_site` (a Spock node
name) in the goatnet's group_vars. It acts on other sites only by inserting rows into
`goat_hub_directives` in the `netbird` database, which spock-mesh-peering installs
(`ops/postgres/spock/hub-authority/netbird.sql`) and replicates. A 30-second timer runs
`spock-hub-directives.sh` at each site:

| Directive | What this site does |
|---|---|
| `sever_site <node>` | Disables its subscriptions from `<node>` in every Spock database. The hub's admin revokes the severed site's mesh peers through the ADR 1193 ledger by hand |
| `restore_site <node>` | Re-enables them, after the site was re-seeded from the hub |
| `identity_edit <id>` (`body.patch`) | Kratos `PATCH`, only if this site owns the identity |
| `identity_delete <id>` | Kratos `DELETE`, only if this site owns the identity |
| `identity_resave <id>` | Re-issues the identity's Kratos rows with the same ids (`goat_identity_resave`), only if this site owns it |
| `transfer_owner <id>` (`body.site`) | Sets `metadata_admin.owner_site`, only if this site owns it now |

A directive whose commit came from a node other than the hub is recorded `refused_origin`
and counted in `goat_hub_directives_refused_total`. One whose origin cannot be read yet is
left pending (`goat_hub_directives_pending`) and retried on the next run. Each site's verdicts are in its own
`goat_hub_directive_applied` table, which is not replicated:

```sql
SELECT d.kind, d.subject, a.verdict, a.origin, a.detail, a.at
  FROM goat_hub_directives d JOIN goat_hub_directive_applied a USING (id) ORDER BY a.at DESC;
```

Issue a directive at the hub only:

```sql
INSERT INTO goat_hub_directives (kind, subject, actor) VALUES ('sever_site', 'zagros_b', '<your handle>');
```

`site-bootstrap.yml` applies the role at every site that replicates with Spock and names
its hub; run it alone with `--tags hub-directives`. The role refuses to run without `goatnet_hub_site`. psql runs inside the local Postgres
container as `postgres`, so the env file holds no password. The partition harness
(`ops/postgres/spock/tests/partition-harness`) runs this same script.
