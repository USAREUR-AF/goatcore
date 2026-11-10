//! A peering's federation.toml, generated for a peering declared
//! `federation_toml: rendered` instead of started by hand from the template
//! `goat federate init` prints.
//!
//! ONE OWNER PER FIELD. The environment's own federation.toml,
//! `ops/federation/<environment>/federation.toml`, stays the owner of what this
//! party declares: `sovereign_id`, `term_days`, `cut_contact`, the release sets
//! (`[data] release`), the accept limits and the measured path MTU. They are
//! copied from it. The release sets are narrowed by the peering's `offer:`, and
//! an offer naming a set the environment's file does not offer is refused
//! (GB-129), as before. `[transport]` is filled from the allocated values: the
//! tunnel interface, the proposed link subnet, and the endpoint the partner
//! dials (the forwarder's public port when a forwarder relays, else the
//! environment's endpoint host with the allocated listen port).
//!
//! Written to `ops/federation/<environment>/peerings/<with>/federation.toml`.

use std::path::Path;

use anyhow::Result;

use super::alloc::PeeringValues;
use super::declaration;
use super::peering_files::{Rendered, MARKER};
use crate::config::{Config, Peering};

/// The environment's own federation.toml, relative to the repository.
pub fn source_path(environment: &str) -> String {
    format!("ops/federation/{environment}/federation.toml")
}

/// Where a peering's generated federation.toml goes, relative to the repository.
pub fn target_path(environment: &str, with: &str) -> String {
    format!("ops/federation/{environment}/peerings/{with}/federation.toml")
}

fn val(v: &toml::Value) -> String {
    v.to_string()
}

fn s(x: &str) -> String {
    toml::Value::String(x.to_string()).to_string()
}

/// The generated file for one peering, from the environment's federation.toml
/// (parsed) and the allocated values.
pub fn render_one(
    p: &Peering,
    v: &PeeringValues,
    source: &toml::Value,
    source_rel: &str,
) -> Result<String> {
    let refuse = |why: String| -> anyhow::Error {
        goat_errors::coded(
            "GB-233",
            format!(
                "peering '{}' declares federation_toml: rendered, and {source_rel} {why} (GB-233). Fix the environment's federation.toml, the one owner of these values, or declare federation_toml: hand",
                p.with
            ),
        )
        .into()
    };
    let top = |k: &str| source.get(k).cloned();
    let transport = source.get("transport");
    let data = source.get("data");
    let mut out = format!(
        "{MARKER}: the federation.toml of peering {} on environment {}.\n\
         # Do not edit. sovereign_id, term_days, cut_contact, [data] and measured_mtu are\n\
         # copied from {source_rel} (their one owner), [data] release narrowed by the\n\
         # peering's offer:; [transport] is filled from the allocated values. Change the\n\
         # source or goatbuilder.yaml, and run `goatbuilder federate allocate` again.\n\n",
        v.peering, v.environment
    );
    for k in ["sovereign_id", "term_days", "cut_contact"] {
        match top(k) {
            Some(x) => out.push_str(&format!("{k} = {}\n", val(&x))),
            None => return Err(refuse(format!("sets no {k}"))),
        }
    }
    // [transport], from the allocation.
    out.push_str("\n[transport]\n");
    out.push_str(&format!("interface            = {}\n", s(&v.wg_interface)));
    match (&p.forwarder, v.public_port) {
        (Some(f), Some(port)) => {
            out.push_str(&format!(
                "endpoint             = {}\n",
                s(&format!("{}:{port}", f.endpoint))
            ));
            out.push_str("listener             = true\n");
        }
        _ => {
            let host = transport
                .and_then(|t| t.get("endpoint"))
                .and_then(|e| e.as_str())
                .map(|e| e.rsplit_once(':').map(|(h, _)| h).unwrap_or(e).to_string());
            let listener = transport
                .and_then(|t| t.get("listener"))
                .and_then(|l| l.as_bool())
                .unwrap_or(false);
            if let Some(h) = host {
                out.push_str(&format!(
                    "endpoint             = {}\n",
                    s(&format!("{h}:{}", v.wg_listen_port))
                ));
            }
            out.push_str(&format!("listener             = {listener}\n"));
        }
    }
    out.push_str(&format!("proposed_link_subnet = {}\n", s(&v.link_subnet)));
    if let Some(mtu) = transport.and_then(|t| t.get("measured_mtu")) {
        out.push_str(&format!("measured_mtu         = {}\n", val(mtu)));
    }
    // [data], copied, the release sets narrowed by the offer.
    let declared: Option<Vec<String>> = data
        .and_then(|d| d.get("release"))
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        });
    let release = match (&p.offer, &declared) {
        (Some(offer), _) => Some(declaration::narrow(offer, declared.as_deref()).map_err(|why| {
            goat_errors::coded(
                "GB-129",
                format!(
                    "peering '{}' on '{}': {why} ({source_rel}) (GB-129). Add the set to [data] release in {source_rel}, the one owner of the offered sets, or remove it from offer:",
                    p.with, p.environment
                ),
            )
        })?),
        (None, Some(d)) => Some(d.clone()),
        (None, None) => None,
    };
    let accepts: Vec<(&str, toml::Value)> = [
        "accept_hz",
        "accept_bytes_per_message",
        "accept_distinct_keys",
    ]
    .iter()
    .filter_map(|k| data.and_then(|d| d.get(*k)).map(|x| (*k, x.clone())))
    .collect();
    if release.is_some() || !accepts.is_empty() {
        out.push_str("\n[data]\n");
        if let Some(r) = release {
            let arr = toml::Value::Array(r.into_iter().map(toml::Value::String).collect());
            out.push_str(&format!("release = {}\n", val(&arr)));
        }
        for (k, x) in accepts {
            out.push_str(&format!("{k} = {}\n", val(&x)));
        }
    }
    Ok(out)
}

/// Every generated federation.toml, for the peerings declared
/// `federation_toml: rendered`. Refuses (GB-233) when the environment's own
/// federation.toml is missing or unreadable, naming it.
pub fn render(repo: &Path, cfg: &Config, values: &[PeeringValues]) -> Result<Vec<Rendered>> {
    let mut out = Vec::new();
    for (p, v) in cfg.peerings.iter().zip(values.iter()) {
        if p.federation_toml.as_deref() != Some("rendered") {
            continue;
        }
        let rel = source_path(&p.environment);
        let text = std::fs::read_to_string(repo.join(&rel)).map_err(|e| {
            goat_errors::coded(
                "GB-233",
                format!(
                    "peering '{}' declares federation_toml: rendered, and the environment's federation.toml, {rel}, cannot be read: {e} (GB-233). Write it once for environment '{}' (sovereign_id, term_days, cut_contact, [data], [transport] measured_mtu), or declare federation_toml: hand",
                    p.with, p.environment
                ),
            )
        })?;
        let source: toml::Value = toml::from_str(&text).map_err(|e| {
            goat_errors::coded(
                "GB-233",
                format!("peering '{}' declares federation_toml: rendered, and {rel} is not TOML: {e} (GB-233). Fix it", p.with),
            )
        })?;
        out.push(Rendered {
            path: target_path(&v.environment, &v.peering),
            body: render_one(p, v, &source, &rel)?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::federate::alloc::{allocate_all, AllocContext};
    use serde_json::json;

    fn cfg(peerings: serde_json::Value) -> Config {
        serde_json::from_value(json!({
            "schema": "goatbuilder/v1",
            "preset": "full",
            "repo": {"source": "example-org/example-repo", "image_namespace": "ghcr.io/example-org"},
            "components": {},
            "environments": [{"name": "zagros", "converger": "push-ansible", "sites": [{"name": "alpha", "role": "founder", "classes": {"core": {}, "edge": {}}}]}],
            "promotion": {"path": [{"name": "zagros", "kind": "rung", "environment": "zagros"}]},
            "peerings": peerings,
        }))
        .unwrap()
    }

    fn p(with: &str) -> serde_json::Value {
        json!({"with": with, "environment": "zagros", "site": "alpha", "edge_class": "edge", "federation_toml": "rendered"})
    }

    const SOURCE: &str = "sovereign_id = \"xaa\"\nterm_days = 91\ncut_contact = \"fed-ops@example.org\"\n\n\
                          [transport]\ninterface = \"wg-fed0\"\nendpoint = \"edge.example.org:51820\"\nlistener = true\n\
                          proposed_link_subnet = \"169.254.10.0/31\"\nmeasured_mtu = 1380\n\n\
                          [data]\nrelease = [\"release/all/**\", \"release/xab/**\", \"release/xac/**\"]\n\
                          accept_hz = 100\naccept_bytes_per_message = 1048576\naccept_distinct_keys = 5000\n";

    fn repo(source: Option<&str>) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        if let Some(text) = source {
            let f = d.path().join(source_path("zagros"));
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    /// The copied values, the narrowed release sets and [transport] from the
    /// allocation, for a listening edge and for one behind a forwarder; a
    /// peering left `hand` gets nothing.
    #[test]
    fn a_peerings_federation_toml_comes_from_the_environments_and_the_allocation() {
        let mut b = p("xab");
        b["offer"] = json!(["all", "xab"]);
        let mut c = p("xac");
        c["forwarder"] = json!({"endpoint": "203.0.113.7"});
        let mut h = p("xad");
        h["federation_toml"] = json!("hand");
        let conf = cfg(json!([b, c, h]));
        let vs = allocate_all(&conf, &AllocContext::default()).unwrap();
        let d = repo(Some(SOURCE));
        let out = render(d.path(), &conf, &vs).unwrap();
        assert_eq!(out.len(), 2);
        let xab = &out[0];
        assert_eq!(
            xab.path,
            "ops/federation/zagros/peerings/xab/federation.toml"
        );
        let t: toml::Value = toml::from_str(&xab.body).unwrap();
        assert_eq!(t["sovereign_id"].as_str(), Some("xaa"));
        assert_eq!(t["term_days"].as_integer(), Some(91));
        assert_eq!(t["transport"]["interface"].as_str(), Some("wg-fed0"));
        assert_eq!(
            t["transport"]["endpoint"].as_str(),
            Some("edge.example.org:51823"),
            "the allocated listen port"
        );
        assert_eq!(
            t["transport"]["proposed_link_subnet"].as_str(),
            Some("169.254.10.0/31")
        );
        assert_eq!(t["transport"]["measured_mtu"].as_integer(), Some(1380));
        let rel: Vec<&str> = t["data"]["release"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert_eq!(
            rel,
            vec!["release/all/**", "release/xab/**"],
            "narrowed by offer, in the source's order"
        );
        assert_eq!(t["data"]["accept_hz"].as_integer(), Some(100));
        assert!(xab.body.starts_with(MARKER));
        let xac: toml::Value = toml::from_str(&out[1].body).unwrap();
        assert_eq!(xac["transport"]["interface"].as_str(), Some("wg-fed1"));
        assert_eq!(
            xac["transport"]["endpoint"].as_str(),
            Some("203.0.113.7:51820"),
            "the forwarder's public port"
        );
        assert_eq!(xac["transport"]["listener"].as_bool(), Some(true));
        assert_eq!(
            xac["data"]["release"].as_array().unwrap().len(),
            3,
            "no offer: every declared set"
        );
    }

    /// An offer naming a set the environment's file does not offer is refused
    /// (GB-129); a missing or incomplete environment file is refused naming it (GB-233).
    #[test]
    fn widening_and_a_missing_source_are_refused() {
        let code = |e: anyhow::Error| e.downcast_ref::<goat_errors::Coded>().unwrap().code.clone();
        let mut b = p("xab");
        b["offer"] = json!(["xaz"]);
        let conf = cfg(json!([b]));
        let vs = allocate_all(&conf, &AllocContext::default()).unwrap();
        assert_eq!(
            code(render(repo(Some(SOURCE)).path(), &conf, &vs).unwrap_err()),
            "GB-129"
        );
        let conf = cfg(json!([p("xab")]));
        let vs = allocate_all(&conf, &AllocContext::default()).unwrap();
        let e = render(repo(None).path(), &conf, &vs).unwrap_err();
        assert!(
            e.to_string()
                .contains("ops/federation/zagros/federation.toml"),
            "{e}"
        );
        assert_eq!(code(e), "GB-233");
        let e = render(repo(Some("term_days = 1\n")).path(), &conf, &vs).unwrap_err();
        assert!(e.to_string().contains("sovereign_id"), "{e}");
    }
}
