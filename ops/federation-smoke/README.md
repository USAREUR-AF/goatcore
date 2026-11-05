# Cross-sovereign federation — smoke test

An end-to-end proof that two environments with **separate certificate authorities** can exchange
data across a boundary that a partner cannot get past.

Canon: [ADR 1157](../../docs/adr/1157-cross-sovereign-federation-edge-terminated.md) ·
design ·
F-424

Sovereign **A** is us. Sovereign **B** is a synthetic partner with its own root CA. They share
nothing — no CA, no key material, no configuration.

## Two tiers, and what has actually been run

| Tier | Proves | Needs | Run? |
|---|---|---|---|
| **1 — PKI** (`assert-pki.sh`) | The certificate facts the design rests on, including F-424 itself | `openssl` only | **Yes — 7/7 pass** |
| **1b — handshake** (`assert-handshake.sh`) | That a real TLS listener requiring a client certificate **accepts or refuses** each party — isolation enforced before authorization is consulted | `openssl` only | **Yes — 6/6 pass** |
| **2 — fabric** (live routers) | That the trust-store isolation holds on a real fabric, and that the edge's ACL is what we think it is | a container runtime, a pinned Zenoh image | **Partly — 3/3 TLS admission run ; message-level denial still unrun** |

> Tier 2 needs a running container runtime. Run under podman against
> `docker.io/eclipse/zenoh@sha256:157965d7…`.
>
> **What it proved.** With the partner-facing listener trusting only sovereign B's CA:
> B's agreed identity is admitted; **B's certificate bearing our own router's Common Name is
> also admitted at TLS** — correctly, because B's CA is trusted there, which is precisely why
> the ACL is a second gate; and **our own operator certificate is refused**, because A's CA is
> not trusted on that listener. 3/3.
>
> **What it did not prove.** That a message is then denied. The upstream image ships only
> `zenohd` — no `z_put`/`z_sub` — so there is no publisher to test with, and a third router
> establishes a session without generating traffic. What *is* verified is the edge's ACL as the
> router itself reports it at startup: `default_permission: deny`, one subject, `partner-svn`,
> bound to the agreed Common Name. Configuration confirmed from the running process; enforcement
> of a message not yet exercised.

Tier 2 is honest about not having been run. Do not cite it as evidence until it has been.

## Tier 1 — run it now

```bash
./gen-cas.sh      # mint two independent CAs and the leaves each would issue
./assert-pki.sh   # 7 assertions
```

It proves, in two `openssl verify` calls, the thing the whole design turns on:

- **B's certificate bearing `CN=goat-zenoh-router` — our own router's name — is chain-valid**
  once a router concatenates both roots into one trust store. Zenoh's ACL matches on Common Name
  and cannot read the issuer, so at that point B's certificate and ours are indistinguishable to
  authorization.
- **The same certificate is refused outright** when the trust store holds only A's CA. That is why
  each partner gets a dedicated router process rather than a second listener.

`gen-cas.sh` deliberately mints `b-impersonate`. Nothing stops a partner doing this; the design
assumes they can.

## Tier 1b — the handshake

```bash
./gen-cas.sh
./assert-handshake.sh
```

Stands up a real TLS listener that requires a client certificate, and asserts who gets through.
This is a step closer to the fabric than `assert-pki.sh`: not merely that a chain validates, but
that a listener **refuses** a foreign peer — before authorization is consulted at all. That is why
one trust store per partner is the control, and why the ACL's blindness to the issuer is
survivable.

> **Two harness traps, both found the hard way, both worth knowing if you write a test like this.**
> First, do not key on `s_client`'s exit code: in TLS 1.3 the server's rejection of a client
> certificate reaches the client asynchronously, so `s_client` can exit 0 on a session the server
> already refused. Assert on data actually flowing. Second, **`s_server -Verify N` alone logs a
> client-certificate failure and then continues** — its verify callback returns 1. Without
> `-verify_return_error`, a wrong-CA client sails through and the test silently proves nothing.
> Both are properties of the openssl test tool, not of TLS and not of the fabric.

## Tier 2 — the live fabric

`ZENOH_IMAGE` is **required and must be digest-pinned** — there is no default. A floating tag is
mutable, so a harness that passed against one would prove nothing about what you actually run
(ADR 0703; `ops/ci/check-zenoh-pinning.sh` enforces this repo-wide).

**`gen-cas.sh` mints FRESH authorities every run.** Recreate the containers after running it, or
the routers hold the previous generation's key material and every handshake fails `DecryptError`
— which cost twenty minutes to diagnose the first time.

```bash
./gen-cas.sh

# Either the image this repo already pins (see roles/goat-zenoh/defaults/main.yml):
export ZENOH_IMAGE='ghcr.io/<org>/zenoh-pqc:sha-36b4e91@sha256:d838142b61cb338043efae6ac0916a86315c005566446a68e0c83ae735eb7042'
# …or pin your own upstream digest:
#   docker buildx imagetools inspect eclipse/zenoh:1.9.0 --format '{{.Manifest.Digest}}'
#   export ZENOH_IMAGE="eclipse/zenoh@sha256:<that digest>"

docker compose up -d
./test-fabric.sh
docker compose down -v
```

`core-a` trusts A's CA only. `edge-a` trusts **B's CA only**, dials inward to the core, and holds
no read rule on it. Five cases:

1. B's agreed identity `CN=peer-svn` is admitted and may write `from/svn/**` — *acceptance*.
2. B's `CN=goat-zenoh-router` is refused — **the F-424 case**.
3. A CN nobody agreed to matches no subject, so default-deny applies.
4. B cannot write outside its landing area.
5. The edge cannot read the core's fabric.

**Every negative case is asserted on the router log, never on an empty result.** Zenoh denial is
silent at the sender: an empty result and a refusal look identical from outside, so
"nothing came back" proves nothing.

## What this does not prove

- **It is not two organisations.** Both sides run on one host under one operator. That gap is
  governance, not mechanism — but it is real, and a genuine second-environment proof is Phase 4 of
  the implementation plan.
- **No SPIRE.** These are static openssl CAs. The automated bundle exchange and rotation that the
  design depends on are the `spire_federates_with` path, whose own acceptance test is written and
  has never been run either.
- **No FIPS claim by default.** Tier 2 runs whatever `ZENOH_IMAGE` you pin. The standing FIPS claim attaches
  to the `zenoh-pqc-fips` image (compliance package);
  set `ZENOH_IMAGE` to it to exercise that path, and note the claim is scoped to routers running it.
- **No transport boundary.** There is no tunnel here — the two routers meet on a Docker network.
  Reachability and its firewall rules are design §5.3.
