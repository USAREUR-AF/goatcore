---
heading: For someone deploying
title: "Added: join-node sets up a joining site's front door when signal federation is off"
---

`goatbuilder join-node` now sets up the front door of a joining site that
declares signal federation off, in phase 13 after the data router, for the
site's core group limited to the node. A site that declares
`dashboard_cert_delivery_mode: acme` gets its public certificate and routes
(`traefik-acme.yml`); any other site gets its LAN routes
(`lan-traefik-routes.yml`, from the internal-CA leaf the phase mints before
the router). Before, only signal federation set up the front door, so such a
site had none. With federation on, nothing changes.
