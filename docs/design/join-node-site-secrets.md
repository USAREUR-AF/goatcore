# A joining site's secrets: `join-node` takes the shared ones from the founder and mints the rest

| Field | Value |
|---|---|
| Status | **Designed, not built.** Today a joining site's secrets are written by hand before `join-node` reaches phase 13 ([goatbuilder.md](goatbuilder.md), the `join-node` table row "The site's secrets"). |
| Gate row | I3 in [goatbuilder-v1-gate.md](goatbuilder-v1-gate.md): `join-node` alone builds every service a joining site declares. A step a person does by hand before phase 13 is a step outside the command. |
| Composes with | [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) (credentials at rest are references only), [ADR 1204](../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md) (the declared secrets posture), [host-site-node-credentialing.md](../operations/host-site-node-credentialing.md) §1 (copy the cluster-shared class, generate the per-site class), [newsite-secret-generation.md](../operations/newsite-secret-generation.md) |

## 1. The problem

A joining site replicates the founder's NetBird, Spock and Ory databases. Some of its secrets therefore have to be the founder's values, and the others have to be its own.

- **Shared with the founder.** If a joiner gets a fresh value, it looks healthy and cannot take part. NetBird cannot decrypt the replicated rows, which were encrypted with the founder's `DataStoreEncryptionKey`. Spock does not replicate. Kratos and Hydra cannot read the replicated identity data, which was encrypted with the founder's cipher and system secrets.
- **The site's own.** If a joiner copies one of these, two sites share a credential, and a compromise or a rotation at one site forces a change at the other.

Today `join-node` uses both classes and creates neither. Phase 13 reads `<SITE>_PG_PASS`, `<SITE>_NB_ENCKEY` and `<SITE>_BKP_KEY` from the environment, through the play's env lookup after `with-secrets` has resolved them (`ev_secrets` in `crates/goatbuilder/src/joinnode.rs`). The input listing names them as required (`crates/goatbuilder/src/inputs/joinnode.rs`). A person writes them first. `newsite-secrets.sh` cannot do it for a joiner: it mints every value fresh, and its header warns that a join must reuse the cluster's shared secrets.

## 2. The two classes

The class of each name has one home: a fourth field on each line of `newsite-secrets.sh`'s `SPEC`, `cluster` for the shared class. The field is empty for everything else. `--spec-json` returns it, and `join-node` reads the list from there, so the generator and `join-node` cannot disagree.

| Name (`<SITE>_` prefix) | Class | Why |
|---|---|---|
| `NB_ENCKEY` | cluster | NetBird's `DataStoreEncryptionKey`; the replicated NetBird rows are encrypted with it. It has no rotation path. |
| `REPLICATOR_PASS` | cluster | the Spock and streaming replicator role that both sides connect as |
| `RELAY_SECRET` | cluster | NetBird relay authentication, shared by the goatnet's relays and every site's management |
| `ORY_SHARED_NID` | cluster | Ory's network id, stored in every replicated Kratos and Hydra row |
| `KRATOS_CIPHER` | cluster | encrypts Kratos data in the replicated database |
| `HYDRA_SYSTEM` | cluster | encrypts Hydra data in the replicated database |
| every other name in the `SPEC` | site | Postgres superuser, pgBackRest cipher, service database roles, cookie and CSRF secrets, MinIO, the per-surface client secrets |

The six cluster names are the ones [host-site-node-credentialing.md](../operations/host-site-node-credentialing.md) §1 already lists. Cookie secrets (`KRATOS_COOKIE`, `HYDRA_COOKIE`, `UI_COOKIE_SECRET`, `UI_CSRF_SECRET`) stay per site, because each site serves its identity pages under its own names (`oauth2.<site_id>.<zone>`), so a cookie never crosses sites. The admin surfaces' OAuth2 client secrets stay per site and are not used at a joiner until gate rows I1 and I4 turn those surfaces on there; when they are, that change decides their class.

## 3. The step

A new step in `join-node`'s phase 3 for a joining site's core node (`goatnet_founder: false`, `--class core`). It runs on the operator's machine. It changes the operator's secret store and reference files only, never the node. Phase 3 holds the checks that run before the node changes (the config preflight and the clock); before it, phase 2 only seeds the SSH key. So a missing founder value stops the run before phase 4 changes the node.

In order:

1. **Take the shared values from the founder.** For each cluster name, the joiner's name `<SITE>_X` is made to resolve to the founder's `<FOUNDER>_X` (`join_founder_site`). How depends on the declared posture:
   - In a store of references (`1password`, `openbao`, `vaultwarden`, `custom`), the step appends `<SITE>_X=<the founder's reference>` to the joiner's reference file (`.envrc.local.d/30-site-services.refs`, or `.envrc.local.d/openbao/` for OpenBao). The founder's reference is the one the reference files already hold for `<FOUNDER>_X`. No value is read, copied or stored a second time, so the store keeps one entry per secret, and a rotation of the founder's value reaches the joiner.
   - In a store of values (`env-file`, `sops`, `git-crypt`), the step reads the founder's value through the posture and writes it under the joiner's name with `posture.set_values`. It is never printed, never on a command line, never in a log.
   - Under the `env` store nothing persists, and the step is refused, as `newsite-secrets.sh` refuses it.
2. **Mint the site's own values.** The step runs `newsite-secrets.sh <site>`. The generator already skips every name that is present in a reference file, a legacy assignment or the environment, and never rotates. Because step 1 has just written the cluster names, it mints only the site class.
3. **Check equality.** Under `with-secrets`, the step compares the SHA-256 digest of each cluster name's resolved value at the joiner with the founder's. It prints the names and `same` or `differs`, never a value or a digest.

What the step never does:

- It never overwrites a value the joiner already holds. A joiner that already has a different value for a cluster name, for example one written by hand before this step existed, is refused with that name. The person decides which value is right; for `NB_ENCKEY`, a joiner whose NetBird database already holds rows encrypted with the wrong key has to be rebuilt.
- It never mints a cluster value. A founder whose value cannot be resolved stops the run.
- It never runs at a founder site. A founder's values come from `standup`.

## 4. What the plan and the refusals say

The dry-run plan prints one line per cluster name and one line for the generator, by name:

```
[core, joiner] site secrets: ZAGROS_B_NB_ENCKEY takes ZAGROS_A_NB_ENCKEY (the founder's reference)
...
[core, joiner] site secrets: newsite-secrets.sh zagros-b mints the site's own values it does not hold yet
```

Two new refusals, each with a code from `ops/errors/catalog.yaml` (to be allotted when this is built):

- The founder's value for a cluster name is unset or unresolved after `with-secrets`. The message names `<FOUNDER>_X` and says to store it under the declared posture (`ops/secrets/posture.py hint <FOUNDER>_X`), then resume with `--from 3`.
- The joiner already holds a value for a cluster name that differs from the founder's. The message names `<SITE>_X` and `<FOUNDER>_X` and says that the step does not overwrite it.

The input listing (`inputs/joinnode.rs`) names the cluster names as "taken from the founder in phase 3" and the site class as "minted in phase 3 when missing", in place of "required".

## 5. The policy question this leaves

In a references store, the joiner's deploy identity must be able to read the founder's entries for the six cluster names. With 1Password this is the same vault. With OpenBao each site's values sit in their own partition (`goat/goat-site-services/goat-site-<site-id>-services`), so the identity that deploys the joiner needs read access to six paths in the founder's partition. That is an access grant, which belongs to the operator, not to this design. Without it, step 1 still writes the reference and step 3 refuses with the first refusal above.

## 6. Tests and the gate

- Unit tests of the step against an `env-file` store and a `custom` store fixture: a joiner with no values gets the six references (or values) and its own fresh ones; a second run changes nothing; a founder value missing is the first refusal, and nothing is written; a joiner value that differs is the second refusal, and nothing is written; no value appears in the plan, the output or the step's command line.
- A test holds the `SPEC`'s `cluster` names to the six in §2 and to [host-site-node-credentialing.md](../operations/host-site-node-credentialing.md) §1.
- The two-site join of gate row I3 (the federation end-to-end unit that stands up a founder and a single-host joiner) runs the step with an `env-file` store. Its measure is that Spock replicates, NetBird decrypts its replicated rows at the joiner, and Kratos and Hydra serve at the joiner.

## 7. Not covered

- Rotating a cluster value. `NB_ENCKEY` has no rotation path; the others rotate at the founder and reach a joiner through its reference.
- A joiner's phone or obs class. Only the core node uses these secrets.
- The admin surfaces' client secrets at a joiner (gate rows I1 and I4).
