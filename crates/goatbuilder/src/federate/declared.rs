//! Run inputs a deployment declares in goatbuilder.yaml, beside the ones
//! [`super::preflight::derive_inputs`] reads from group_vars
//! (docs/design/federation-preflight.md, "Inputs declared in goatbuilder.yaml").
//!
//! Each value here is optional in the schema. A deployment that declares none
//! of them gets exactly the inputs it got before: [`overlay`] adds nothing. A
//! declared value fills an input that nothing else derived, or replaces one
//! derived only by convention (a default path); a value the edge group's
//! group_vars set still wins over it, and a variable set in the environment
//! wins over both ([`super::preflight::unset_derived`]).
//!
//! - `secrets.federation`: the paths of our package signing key, our CA
//!   certificate and our CA key (FED_SIGNING_KEY, FED_CA_CERT, FED_CA_KEY). A
//!   relative path is read inside the active goat context directory.
//! - `environments[].peering_directory`: the peering directory this
//!   environment's peerings exchange through, and the CA its TLS certificate
//!   chains to when that is not a public one (FED_DIRECTORY_URL, SSL_CERT_FILE).

use std::path::{Path, PathBuf};

use super::preflight::{input, Input};
use crate::config;

/// The active goat context directory, the way `goat` resolves it:
/// `$GOAT_PROFILE_DIR` (the driver sets it before this runs) else `~/.goat`,
/// then `$GOAT_CONTEXT`, else `current_context` in its config.toml, else
/// `default`.
pub fn context_dir() -> PathBuf {
    let profile = std::env::var("GOAT_PROFILE_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".goat"));
    let context = std::env::var("GOAT_CONTEXT")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            let text = std::fs::read_to_string(profile.join("config.toml")).ok()?;
            let doc: toml::Value = toml::from_str(&text).ok()?;
            doc.get("current_context")?
                .as_str()
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
        .unwrap_or_else(|| "default".to_string());
    profile.join("contexts").join(context)
}

/// A declared path as the run reads it: `~/` is the home directory, an
/// absolute path is itself, and anything else is inside the goat context.
fn in_context(declared: &str, context: &Path) -> String {
    if let Some(rest) = declared.strip_prefix("~/") {
        return format!("{}/{rest}", std::env::var("HOME").unwrap_or_default());
    }
    if Path::new(declared).is_absolute() {
        return declared.to_string();
    }
    context.join(declared).display().to_string()
}

/// The inputs this deployment declares in goatbuilder.yaml for the peering, in
/// the order preflight reports them. Empty when it declares none.
pub fn declared_inputs(cfg: &config::Config, peering: &config::Peering) -> Vec<Input> {
    let mut v = Vec::new();
    if let Some(fed) = cfg.secrets.as_ref().and_then(|s| s.federation.as_ref()) {
        let context = context_dir();
        let mut path = |name: &'static str, key: &str, value: &Option<String>| {
            if let Some(p) = value.as_deref().filter(|p| !p.is_empty()) {
                v.push(input(
                    name,
                    Some(in_context(p, &context)),
                    format!(
                        "goatbuilder.yaml secrets.federation.{key} (relative to the goat context {})",
                        context.display()
                    ),
                ));
            }
        };
        path("FED_SIGNING_KEY", "signing_key", &fed.signing_key);
        path("FED_CA_CERT", "ca_cert", &fed.ca_cert);
        path("FED_CA_KEY", "ca_key", &fed.ca_key);
    }
    let env = cfg
        .environments
        .iter()
        .find(|e| e.name == peering.environment);
    if let Some(dir) = env.and_then(|e| e.peering_directory.as_ref()) {
        let at = |k: &str| {
            format!(
                "goatbuilder.yaml environments[{}].peering_directory.{k}",
                peering.environment
            )
        };
        v.push(input("FED_DIRECTORY_URL", Some(dir.url.clone()), at("url")));
        if let Some(ca) = dir.ca_cert.as_deref().filter(|c| !c.is_empty()) {
            v.push(input(
                "SSL_CERT_FILE",
                Some(in_context(ca, &context_dir())),
                at("ca_cert"),
            ));
        }
    }
    v
}

/// Merge the declared inputs into the derived ones. A declared input replaces a
/// derived one that has no value or whose value came from anything but the
/// edge group's own group_vars; otherwise the group_vars value stays. A
/// declared input preflight did not list is appended.
pub fn overlay(inputs: &mut Vec<Input>, declared: Vec<Input>) {
    for d in declared {
        match inputs.iter_mut().find(|i| i.name == d.name) {
            Some(i) if i.value.is_some() && i.source.starts_with("group_vars/") => {}
            Some(i) => *i = d,
            None => inputs.push(d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: serde_json::Value) -> config::Config {
        let mut v = serde_json::json!({
            "schema": "goatbuilder/v1",
            "environments": [{"name": "dev", "converger": "push-ansible"}],
            "promotion": {"path": [{"name": "dev", "kind": "rung", "environment": "dev"}]},
            "peerings": [{"with": "xab", "environment": "dev", "site": "alpha", "edge_class": "edge"}],
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    fn get<'a>(v: &'a [Input], name: &str) -> Option<&'a Input> {
        v.iter().find(|i| i.name == name)
    }

    #[test]
    fn nothing_declared_adds_nothing() {
        let c = cfg(serde_json::json!({"secrets": {"store": "env-file"}}));
        assert!(declared_inputs(&c, &c.peerings[0]).is_empty());
        let c = cfg(serde_json::json!({}));
        assert!(declared_inputs(&c, &c.peerings[0]).is_empty());
    }

    #[test]
    fn declared_paths_resolve_absolute_home_and_in_the_context() {
        let ctx = Path::new("/profile/contexts/zagros");
        assert_eq!(
            in_context("/srv/keys/signing.key", ctx),
            "/srv/keys/signing.key"
        );
        assert_eq!(
            in_context("federation/ca.pem", ctx),
            "/profile/contexts/zagros/federation/ca.pem"
        );
        let home = std::env::var("HOME").unwrap_or_default();
        assert_eq!(
            in_context("~/k/ca-key.pem", ctx),
            format!("{home}/k/ca-key.pem")
        );
    }

    #[test]
    fn the_signing_key_and_ca_come_from_secrets_federation() {
        let c = cfg(
            serde_json::json!({"secrets": {"store": "env-file", "federation": {
            "signing_key": "/srv/fed/signing.key", "ca_cert": "/srv/fed/ca.pem", "ca_key": "/srv/fed/ca-key.pem"}}}),
        );
        let d = declared_inputs(&c, &c.peerings[0]);
        assert_eq!(
            d.iter().map(|i| i.name).collect::<Vec<_>>(),
            ["FED_SIGNING_KEY", "FED_CA_CERT", "FED_CA_KEY"]
        );
        assert_eq!(
            get(&d, "FED_CA_KEY").unwrap().value.as_deref(),
            Some("/srv/fed/ca-key.pem")
        );
        assert!(get(&d, "FED_SIGNING_KEY")
            .unwrap()
            .source
            .contains("secrets.federation.signing_key"));
        // Only what is declared: an undeclared CA key stays to the other sources.
        let c = cfg(
            serde_json::json!({"secrets": {"store": "env-file", "federation": {"signing_key": "s.key"}}}),
        );
        let d = declared_inputs(&c, &c.peerings[0]);
        assert_eq!(d.len(), 1);
        assert!(d[0].value.as_deref().unwrap().ends_with("/s.key"));
    }

    #[test]
    fn the_directory_and_its_ca_come_from_the_peerings_environment() {
        let c = cfg(serde_json::json!({"environments": [
            {"name": "dev", "converger": "push-ansible",
             "peering_directory": {"url": "https://directory.example.org:8443", "ca_cert": "/srv/fed/directory-ca.pem"}},
            {"name": "prod", "converger": "push-ansible",
             "peering_directory": {"url": "https://other.example.org"}}]}));
        let d = declared_inputs(&c, &c.peerings[0]);
        assert_eq!(
            d.iter().map(|i| i.name).collect::<Vec<_>>(),
            ["FED_DIRECTORY_URL", "SSL_CERT_FILE"]
        );
        assert_eq!(
            d[0].value.as_deref(),
            Some("https://directory.example.org:8443")
        );
        assert_eq!(d[1].value.as_deref(), Some("/srv/fed/directory-ca.pem"));
        assert!(d[0]
            .source
            .contains("environments[dev].peering_directory.url"));
        // A directory with a public certificate declares no CA, and SSL_CERT_FILE is left alone.
        let mut c = c;
        c.peerings[0].environment = "prod".into();
        let d = declared_inputs(&c, &c.peerings[0]);
        assert_eq!(
            d.iter().map(|i| i.name).collect::<Vec<_>>(),
            ["FED_DIRECTORY_URL"]
        );
        // An environment that declares no directory gives nothing.
        let c = cfg(serde_json::json!({}));
        assert!(declared_inputs(&c, &c.peerings[0]).is_empty());
    }

    #[test]
    fn a_shell_unsafe_declared_path_is_not_used() {
        let c = cfg(
            serde_json::json!({"secrets": {"store": "env-file", "federation": {"ca_key": "/srv/$(x)/k"}}}),
        );
        let d = declared_inputs(&c, &c.peerings[0]);
        assert_eq!(d[0].value, None);
    }

    #[test]
    fn group_vars_win_and_a_declaration_beats_a_convention() {
        let mut inputs = vec![
            input(
                "FED_SIGNING_KEY",
                Some("/gv".into()),
                "group_vars/xab_edge: goat_federation_signing_key",
            ),
            input(
                "FED_CA_CERT",
                Some("/conv".into()),
                "ops/ansible/enrollment/ca/alpha/root-cert.pem",
            ),
            input(
                "FED_CA_KEY",
                None,
                "group_vars/xab_edge: goat_federation_ca_key",
            ),
        ];
        overlay(
            &mut inputs,
            vec![
                input("FED_SIGNING_KEY", Some("/decl-s".into()), "decl"),
                input("FED_CA_CERT", Some("/decl-c".into()), "decl"),
                input("FED_CA_KEY", Some("/decl-k".into()), "decl"),
                input("SSL_CERT_FILE", Some("/decl-d".into()), "decl"),
            ],
        );
        let val = |n: &str| get(&inputs, n).unwrap().value.clone().unwrap();
        assert_eq!(val("FED_SIGNING_KEY"), "/gv");
        assert_eq!(val("FED_CA_CERT"), "/decl-c");
        assert_eq!(val("FED_CA_KEY"), "/decl-k");
        assert_eq!(val("SSL_CERT_FILE"), "/decl-d");
        assert_eq!(inputs.len(), 4);
    }

    #[test]
    fn the_context_follows_goat_context_rules() {
        // Read with the process environment as it is: GOAT_CONTEXT unset or set,
        // the path always ends in contexts/<name>.
        let d = context_dir();
        assert_eq!(
            d.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str()),
            Some("contexts")
        );
    }
}
