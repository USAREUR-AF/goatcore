# Configuration compatibility (gate row A2)

A configuration file that a goatbuilder release accepted is accepted by every
later build, and one it refused is refused the same way. This directory holds
the files that make that a test: `configs/manifest.yaml` lists, for each
published release since `goatbuilder/v0.4.1`, every configuration file that
release shipped, with the outcome its own published binary gave it.
`../config_compat.rs` runs this build's `validate` over each and refuses a
different outcome.

## What a release's set holds

From the release tag's own tree:

- every `goatbuilder.yaml` under `ops/goatbuilder/fixtures/`: the full preset,
  the lite preset (goatlite), and the fixtures that declare peerings;
- every refusal fixture under `crates/goatbuilder/tests/fixtures/configs/`;
- every whole example in `docs/`: a fenced YAML block that carries
  `schema: goatbuilder/v1`, including the goatlite tutorial's.

Each file's content is stored once under `configs/files/`, named by its
SHA-256, however many releases shipped it.

Each entry's fields:

- `expect`: `valid` (exit 0), `refused` (exit 1, a checks document) or `error`
  (any other exit, with its error document);
- `code`: the catalog code that release printed;
- `named`: the code the release's documents give the refusal (GB-025, "the
  config does not validate"), added by hand. A build may print the code its
  release printed or a named one (one code, or a list). The not-YAML fixture
  names GB-245, "the config file is not YAML", which goatbuilder raises at its
  source with exit 2; GB-025 is a different fault and is not accepted for it.

`goatbuilder/v0.5.0` and `goatbuilder/v0.5.2` were tagged and never published,
so they have no binary and no set; the manifest names them with that reason.

The files hold synthetic values: documentation addresses, example domains,
and the LAN address `192.168.1.50` that goatlite's own example uses, kept as
the release published it.

## When a release is published

```bash
v=X.Y.Z
curl -fsSLO https://github.com/USAREUR-AF/goatcore/releases/download/goatbuilder/v$v/goatbuilder-linux-amd64
curl -fsSLO https://github.com/USAREUR-AF/goatcore/releases/download/goatbuilder/v$v/SHA256SUMS
chmod +x goatbuilder-linux-amd64
python3 crates/goatbuilder/tests/compat/record.py --tag goatbuilder/v$v --bin ./goatbuilder-linux-amd64 --sums SHA256SUMS
```

`record.py` takes the binary only when its SHA-256 is the
`goatbuilder-linux-amd64` entry of that SHA256SUMS and it reports the tag's
version. It refuses a release that is already recorded, and never changes an
earlier release's set. The reviewer compares SHA256SUMS with the release page.
A tag that was never published is recorded with
`record.py --tag goatbuilder/vX.Y.Z --unpublished "<why>"`.

## Not covered

- `goatnet.toml` is the goat client's file, not goatbuilder's. `validate` does
  not read it, and the goat client has its own releases.
- Peering fragments in the documents, which are not a whole file, are covered
  by the fixtures that declare peerings, not as fragments.
