//! What a project declares about how it is deployed, read once and in one order
//! (docs/design/goatbuilder-portability.md): the environment variable a person
//! can set for one command, then `ops/goatbuilder/goatbuilder.yaml`, then a value
//! that names nobody's project.

use std::path::Path;

use crate::config;

fn declared(repo: &Path) -> Option<serde_json::Value> {
    config::load_value(&repo.join("ops/goatbuilder/goatbuilder.yaml")).ok()
}

fn deploy_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get("deploy")?.get(key)?.as_str().filter(|s| !s.is_empty()).map(String::from)
}

/// The AWS profile a run uses: `GOAT_AWS_PROFILE`, then `deploy.aws_profile`,
/// then `default`, the name the AWS tools themselves fall back to.
pub fn aws_profile(repo: &Path) -> String {
    std::env::var("GOAT_AWS_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| declared(repo).and_then(|v| deploy_field(&v, "aws_profile")))
        .unwrap_or_else(|| "default".to_string())
}

/// The account a new machine is first reached as: `deploy.ssh_user`, then the
/// goatlite `lite.ssh_user`. `None` when the project declares neither: a plan
/// prints a placeholder and a real run refuses, because no name is right for
/// every project's machines.
pub fn ssh_user(repo: &Path) -> Option<String> {
    let v = declared(repo)?;
    deploy_field(&v, "ssh_user").or_else(|| v.get("lite")?.get("ssh_user")?.as_str().filter(|s| !s.is_empty()).map(String::from))
}

/// Where this machine keeps a project's local state (CA keys, run logs) as the
/// project declares it, `~` and all: `deploy.state_dir`, else `~/.goat/state`.
pub fn state_dir(repo: &Path) -> String {
    declared(repo).and_then(|v| deploy_field(&v, "state_dir")).unwrap_or_else(|| "~/.goat/state".to_string())
}

/// `state_dir` with `~` expanded, for a path this process opens.
pub fn state_dir_abs(repo: &Path) -> String {
    expand(&state_dir(repo))
}

/// The append-only audit log the signing tools write (`cert-sign`,
/// `intent-sign`, `name-snapshot-sign`), as the project declares its state
/// directory, `~` and all: `ca-audit.jsonl` beside `state_dir`. Every call
/// names it with `--audit-log`, so the file does not depend on a default
/// built into the tool (`ops/scripts/project-setting.sh ca-audit-log` is the
/// same rule for the shell drivers).
pub fn ca_audit_log(repo: &Path) -> String {
    beside_state(&state_dir(repo))
}

fn beside_state(state: &str) -> String {
    match state.trim_end_matches('/').rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/ca-audit.jsonl"),
        None => "./ca-audit.jsonl".to_string(),
    }
}

fn expand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

/// The values a verb reads where it has no checkout path in hand, read once at
/// start from the checkout the run uses.
static GLOBAL: std::sync::OnceLock<(Option<String>, String)> = std::sync::OnceLock::new();

pub fn init(repo: &Path) {
    let _ = GLOBAL.set((ghcr_org(repo), state_dir(repo)));
}

/// `ghcr_org` for the checkout `init` read. `GHCR_ORG` still wins.
pub fn ghcr_org_global() -> Option<String> {
    #[cfg(test)]
    init(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    std::env::var("GHCR_ORG").ok().filter(|s| !s.is_empty()).or_else(|| GLOBAL.get().and_then(|g| g.0.clone()))
}

/// `state_dir_abs` for the checkout `init` read.
pub fn state_dir_abs_global() -> String {
    #[cfg(test)]
    init(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    expand(GLOBAL.get().map_or("~/.goat/state", |g| g.1.as_str()))
}

/// `ca_audit_log` with `~` expanded, for the checkout `init` read.
pub fn ca_audit_log_abs_global() -> String {
    beside_state(&state_dir_abs_global())
}

/// The registry namespace images are published under (`repo.image_namespace`).
pub fn image_namespace(repo: &Path) -> String {
    declared(repo)
        .and_then(|v| v.get("repo")?.get("image_namespace")?.as_str().map(String::from))
        .unwrap_or_else(|| "<repo.image_namespace>".to_string())
}

/// The declared AWS profile alone, for a caller that resolves the environment itself.
pub fn declared_aws_profile(repo: &Path) -> Option<String> {
    declared(repo).and_then(|v| deploy_field(&v, "aws_profile"))
}

/// The organisation in a `ghcr.io/<org>` image namespace: `GHCR_ORG`, then
/// `repo.image_namespace`. `None` for another registry, whose references have
/// no `{{ ghcr_org }}` form to write.
pub fn ghcr_org(repo: &Path) -> Option<String> {
    std::env::var("GHCR_ORG").ok().filter(|s| !s.is_empty()).or_else(|| {
        let v = declared(repo)?;
        v.get("repo")?.get("image_namespace")?.as_str()?.strip_prefix("ghcr.io/").map(String::from)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(yaml: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("ops/goatbuilder")).unwrap();
        std::fs::write(t.path().join("ops/goatbuilder/goatbuilder.yaml"), yaml).unwrap();
        t
    }

    #[test]
    fn the_ssh_user_is_what_the_project_declares_and_nothing_when_it_declares_none() {
        assert_eq!(ssh_user(checkout("deploy: {ssh_user: ops}\n").path()).as_deref(), Some("ops"));
        assert_eq!(ssh_user(checkout("lite: {ssh_user: pat}\n").path()).as_deref(), Some("pat"));
        assert_eq!(ssh_user(checkout("deploy: {ssh_user: ops}\nlite: {ssh_user: pat}\n").path()).as_deref(), Some("ops"));
        assert_eq!(ssh_user(checkout("repo: {source: a/b}\n").path()), None);
        assert_eq!(ssh_user(Path::new("/no/such/checkout")), None);
    }

    #[test]
    fn the_state_dir_is_declared_or_neutral() {
        assert_eq!(state_dir(checkout("deploy: {state_dir: ~/.acme/state}\n").path()), "~/.acme/state");
        assert_eq!(state_dir(checkout("repo: {source: a/b}\n").path()), "~/.goat/state");
        assert!(state_dir_abs(checkout("deploy: {state_dir: /srv/s}\n").path()) == "/srv/s");
    }

    #[test]
    fn the_audit_log_sits_beside_the_state_dir() {
        assert_eq!(ca_audit_log(checkout("deploy: {state_dir: ~/.acme/state}\n").path()), "~/.acme/ca-audit.jsonl");
        assert_eq!(ca_audit_log(checkout("repo: {source: a/b}\n").path()), "~/.goat/ca-audit.jsonl");
        assert_eq!(beside_state("/srv/s/"), "/srv/ca-audit.jsonl");
        assert_eq!(beside_state("state"), "./ca-audit.jsonl");
    }

    // The two environment overrides are read in one test: tests share a process.
    #[test]
    fn the_environment_wins_then_the_declaration_then_a_neutral_name() {
        let t = checkout("repo: {image_namespace: ghcr.io/acme}\ndeploy: {aws_profile: acme-prod}\n");
        let other = checkout("repo: {image_namespace: registry.example.org/acme}\n");
        std::env::remove_var("GOAT_AWS_PROFILE");
        std::env::remove_var("GHCR_ORG");
        assert_eq!(aws_profile(t.path()), "acme-prod");
        assert_eq!(aws_profile(other.path()), "default");
        assert_eq!(ghcr_org(t.path()).as_deref(), Some("acme"));
        assert_eq!(ghcr_org(other.path()), None);
        std::env::set_var("GOAT_AWS_PROFILE", "just-this-once");
        std::env::set_var("GHCR_ORG", "elsewhere");
        assert_eq!(aws_profile(t.path()), "just-this-once");
        assert_eq!(ghcr_org(other.path()).as_deref(), Some("elsewhere"));
        std::env::remove_var("GOAT_AWS_PROFILE");
        std::env::remove_var("GHCR_ORG");
    }
}
