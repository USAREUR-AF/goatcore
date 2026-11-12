// End-to-end golden tests for the `goatbuilder` binary, on goat-cli's model
// (crates/goat-cli/tests/cli_golden.rs): `trycmd` drives the built binary
// against the `.toml` cases in `tests/cmd/`.
//
// Every case is read-only and independent of the live pins: it runs against
// the conformance fixtures (ops/goatbuilder/fixtures/*), the configs under
// tests/fixtures/, or a refusal that fires before any repository state is
// read. Cases that mutate a fixture live in tests/flows.rs, on a temp copy.
//
// Add a case: drop `tests/cmd/<name>.toml`. Regenerate after an intended
// output change: `TRYCMD=overwrite cargo test -p goatbuilder --test cli_golden`.

#[path = "common/test_home.rs"]
mod test_home;

#[test]
fn cli_golden() {
    let t = trycmd::TestCases::new();
    for (k, v) in test_home::vars() {
        t.env(k, v);
    }
    t.case("tests/cmd/*.toml");
    // Four cases run the binary against this checkout's own deployment
    // (`-C ../..`) and expect refusals that quote its declaration. A public
    // clone of goatcore declares no deployment, so there is nothing to quote.
    let declares_a_deployment = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/goatbuilder/goatbuilder.yaml")
        .is_file();
    if !declares_a_deployment {
        for c in [
            "deploy_unknown_component",
            "federate_undeclared_is_refused",
            "join_node_unknown_class",
            "join_node_unknown_env",
        ] {
            t.skip(format!("tests/cmd/{c}.toml"));
        }
    }
}
