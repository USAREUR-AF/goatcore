# The phone peer profile: a phone node that is not a replication member

| Field | Value |
|---|---|
| Status | **Designed, not built.** A later class value. Operator decision, 2026-11-15 (gate group D, D2): goatbuilder 1.0 keeps a phone a full `core` joiner, and this profile is among the first work after 1.0 |
| Design | [phone-node.md](phone-node.md) · [phone-node-offline-window.md](phone-node-offline-window.md) · [node-topologies-and-replication-mechanisms.md](node-topologies-and-replication-mechanisms.md) · [island-mode-multi-writer.md](island-mode-multi-writer.md) |
| Audience | whoever decides what a phone node carries, and whoever builds the class value |

## Why a second profile

A phone node today is a full `core` joiner (`converges_as: core` in
`ops/ansible/group_vars/class_phone.yml`). It holds a copy of the goatnet's
control-plane databases, and every replicating site keeps a slot for it. A
phone that is often off pays for that:

- every site that keeps a slot for it holds up to a cap of WAL for it;
- after an absence longer than the cap covers, it must be seeded again
  ([phone-node-offline-window.md](phone-node-offline-window.md)), and until
  then it silently receives nothing;
- under island mode it is a writer that rejoins late, the case the partition
  drill (gate row I5) and the write-path table (I6) must prove safe.

A peer profile removes the phone from replication. It is for phones whose
normal state is to be off or out of reach for long stretches.

## How it is asked for

```
goatbuilder join-node <node> --class phone --profile peer --via usb
```

`--profile` is a new value on an existing class. Without it, `--class phone`
means the full member, as in 1.0, so adding the value later keeps the 1.x
compatibility promise. The class file gains a second shape: `converges_as:
peer` for the profile, `core` for the default.

## What it runs, and what it does not

| Service | Full member (today) | Peer profile |
|---|---|---|
| NetBird client (the node's mesh address) | yes | yes |
| Zenoh router, and the node-scoped health publishers | yes | yes: the fabric needs no local database, and data crosses sites by Zenoh |
| Local services that keep their own state on the node | as deployed | as deployed |
| Postgres with Spock, and a slot for it on every replicating site | yes | **no** |
| NetBird management and signal | yes | **no**: the node is a NetBird peer of a site's management, like any client |
| Kratos and Hydra | yes | **no**: sign-in goes to a site's identity service over the mesh |
| The admission audit | yes | **no**: it audits the admissions the node no longer makes |
| Traefik | yes | only when a local service needs it |

## What it costs, and what it rules out

| | |
|---|---|
| What the founder saves | the slot and its WAL, the re-seed after a long absence, and a late-rejoining writer under island mode |
| What the founder still carries | one NetBird peer, the phone's management and signal sessions, and its sign-ins: small and stateless on the founder's side |
| What the phone cannot do while cut off | admit people, enrol machines, issue setup keys or sign anyone in. It is a node, not a site. Mesh tunnels it already has, and local Zenoh traffic, are expected to keep working; that is not measured |
| What the phone still does while cut off | run its own router and local services, and carry fabric data again when it returns. Zenoh has no slot to lose |
| What has to change | the class file and the end-state catalog gain the second shape; `join-node` converges the peer set; the H rows that are built on the full member (H3 to H5, H7) run again for the peer |

## Not built

The `--profile` value, the second shape in the class file and the end-state
catalog, the peer's play set, and the H row runs for it. The table above is
the design; no row of it is measured.
