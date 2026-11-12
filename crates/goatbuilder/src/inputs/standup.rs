//! Every value `goatbuilder standup <env> --site <site>` uses, with its source:
//! the site and the names derived from it, what the site's group_vars declare
//! (each at its line), the profile, the AWS profile, the goatlite declaration,
//! the state directory, the values each phase fixes, and the variables the
//! phases read from their environment when they run (credentials among them,
//! listed by their source only).
//!
//! The values are the ones `standup::read_site` derived; this module says where
//! each came from, by the same rules. The test runs every branch of the plan
//! and finds every value of every command line here.

use std::path::Path;

use anyhow::Result;

use super::{push, rel, yaml_key_line, yaml_line, Input, Seg, Source};
use crate::standup::{self, Options, Site};

/// Reads the operator's environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn set(env: Env, name: &str) -> Option<String> {
    env(name).filter(|v| !v.is_empty())
}

/// The inputs of a standup of `o.site`.
pub fn inputs(repo: &Path, o: &Options, env: Env) -> Result<Vec<Input>> {
    let s = standup::read_site(repo, o)?;
    Ok(site_inputs(repo, &s, o, env))
}

/// The site's group_vars file every value below is read from.
fn vars_rel(s: &Site) -> String {
    format!("ops/ansible/group_vars/{}/vars.yml", s.group)
}

/// `key` in the site's vars.yml, at its line.
fn gv(repo: &Path, s: &Site, key: &str) -> Source {
    let rel = vars_rel(s);
    let text = std::fs::read_to_string(repo.join(&rel)).unwrap_or_default();
    Source::file(rel, yaml_key_line(&text, key), key)
}

/// `lite.<key...>` in goatlite.yaml, at its line.
fn lite(repo: &Path, keys: &[&str]) -> Source {
    let text = std::fs::read_to_string(repo.join("goatlite.yaml")).unwrap_or_default();
    let mut path = vec![Seg::Key("lite")];
    path.extend(keys.iter().map(|k| Seg::Key(k)));
    Source::file(
        "goatlite.yaml",
        yaml_line(&text, &path),
        format!("lite.{}", keys.join(".")),
    )
}

/// `deploy.<key>` in goatbuilder.yaml at its line, else `default`.
fn deploy_or(repo: &Path, key: &str, default: &str) -> Source {
    let p = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    match yaml_line(&text, &[Seg::Key("deploy"), Seg::Key(key)]) {
        Some(n) => Source::file(rel(repo, &p), Some(n), format!("deploy.{key}")),
        None => Source::default(default.to_string()),
    }
}

/// The inputs of a standup whose site is `s` (read by `standup::read_site`).
pub fn site_inputs(repo: &Path, s: &Site, o: &Options, env: Env) -> Vec<Input> {
    let mut v: Vec<Input> = Vec::new();
    let site = &s.site;
    let g = &s.group;
    let site_rule = |what: &str| Source::derived(format!("the site '{site}'"), what.to_string());

    // The command line and the names every command derives from the site.
    v.push(Input::new(
        "environment",
        o.env,
        Source::arg("<environment>"),
    ));
    v.push(Input::new(
        "site",
        site.clone(),
        Source::arg("--site, else the goatlite's lite.name"),
    ));
    v.push(Input::new(
        "repo",
        s.repo.clone(),
        Source::arg("-C, else the current directory"),
    ));
    v.push(Input::new(
        "group",
        g.clone(),
        site_rule("<site, - as _>_hosts"),
    ));
    v.push(Input::new(
        "obs_group",
        s.obs_group.clone(),
        site_rule("<site, - as _>_obs_hosts"),
    ));
    v.push(Input::new(
        "first_mgmt_host",
        format!("mgmt-{site}-01*"),
        site_rule("mgmt-<site>-01*, the site's first management host"),
    ));
    for i in s.inv.split_whitespace().filter(|w| *w != "-i") {
        v.push(Input::new(
            format!("inventory {i}"),
            i,
            Source::default("the management inventories every play reads (standup::INV)"),
        ));
    }
    v.push(Input::new(
        "relay_inventory",
        format!("inventory/hosts-{site}.ini"),
        site_rule("inventory/hosts-<site>.ini, the relay inventory phase 7 generates"),
    ));
    for f in ["mgmt-hosts.ini", "mgmt-hosts-mesh.ini"] {
        v.push(Input::new(
            format!("inventory_path {f}"),
            format!("{}/ops/ansible/inventory/{f}", s.repo),
            Source::default(format!(
                "ops/ansible/inventory/{f}, the host preflight's inventory"
            )),
        ));
    }

    // The profile and the AWS profile.
    let profile_src = match o.profile {
        Some(_) => Source::arg("--profile"),
        None if s.lite => lite(repo, &["internet"]),
        None => {
            let src = gv(repo, s, "dashboard_cert_delivery_mode");
            match src {
                Source::File { line: Some(_), .. }
                    if standup::gv_matches(
                        &repo.join(vars_rel(s)),
                        "dashboard_cert_delivery_mode",
                        "acme",
                    ) =>
                {
                    Source::derived(src.render(), "acme is the internet profile")
                }
                _ => {
                    Source::default("lan, when the site's dashboard_cert_delivery_mode is not acme")
                }
            }
        }
    };
    v.push(Input::new("profile", s.profile.clone(), profile_src));
    // The word after with-aws, read as the shell reads it: a declared profile
    // that needs quoting is passed quoted, and its value is the unquoted word.
    // A `~/...` profile is listed as declared: the shell does not expand a
    // tilde inside the quoted word, and neither does this. A prefix that does
    // not parse (an unbalanced quote), or names no profile, is listed as
    // missing rather than as an empty profile.
    let aws_word = shlex::split(&s.ansible_pre).and_then(|w| {
        w.into_iter()
            .skip_while(|w| !w.ends_with("with-aws"))
            .nth(1)
            .filter(|p| p != "--")
    });
    let aws = aws_word.clone().unwrap_or_default();
    let aws_src = if set(env, "GOAT_AWS_PROFILE").is_some() {
        Source::env("GOAT_AWS_PROFILE")
    } else if s.lite && aws != crate::settings::aws_profile(repo) {
        let l = lite(repo, &["internet", "aws_profile"]);
        match l {
            Source::File { line: Some(_), .. } => l,
            _ => Source::derived(
                lite(repo, &["name"]).render(),
                "a goatlite grown to the internet uses its own name",
            ),
        }
    } else {
        deploy_or(
            repo,
            "aws_profile",
            "default, the profile the AWS tools themselves fall back to",
        )
    };
    match aws_word {
        Some(_) => v.push(Input::new("aws_profile", aws, aws_src)),
        None => v.push(Input::unset(
            "aws_profile",
            "the plays' command prefix names no AWS profile that reads as one word after with-aws (an unbalanced quote, or nothing there): declare deploy.aws_profile in ops/goatbuilder/goatbuilder.yaml, or set GOAT_AWS_PROFILE",
        )),
    }
    v.push(Input::new("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES", Source::default("set on every play, so Ansible's forked workers run on macOS; it changes nothing elsewhere")));
    if s.ansible_pre.contains("GOAT_AWS_MODE=none") {
        v.push(Input::new(
            "GOAT_AWS_MODE",
            "none",
            Source::derived(
                "a goatlite on the lan profile",
                "it declares no cloud, so no play reaches AWS",
            ),
        ));
    }

    // What the site's group_vars declare.
    if s.joiner {
        v.push(Input::new(
            "goatnet_founder",
            "false",
            gv(repo, s, "goatnet_founder"),
        ));
        v.push(Input::new(
            "join_founder_site",
            s.join_founder_site.clone(),
            gv(repo, s, "join_founder_site"),
        ));
        v.push(Input::new(
            "join_founder_group",
            s.join_founder_group.clone(),
            Source::derived("join_founder_site (above)", "<founder site, - as _>_hosts"),
        ));
        v.push(Input::new(
            "founder_relay_alias_inventory",
            format!("inventory/{}-goat-relay-alias.ini", s.join_founder_site),
            Source::derived(
                "join_founder_site (above)",
                "inventory/<founder>-goat-relay-alias.ini",
            ),
        ));
        v.push(Input::new(
            "founder_relay_inventory",
            format!("inventory/hosts-{}.ini", s.join_founder_site),
            Source::derived("join_founder_site (above)", "inventory/hosts-<founder>.ini"),
        ));
    }
    v.push(Input::new(
        "acl_site",
        s.acl_site.clone(),
        if s.joiner {
            Source::derived(
                "join_founder_site (above)",
                "a joiner is governed by its founder's NetBird ACL",
            )
        } else {
            site_rule("the site itself (a founder)")
        },
    ));
    v.push(Input::new(
        "token_env",
        format!(
            "{}_NB_API_TOKEN",
            s.acl_site.replace('-', "_").to_uppercase()
        ),
        Source::derived(
            "acl_site (above)",
            "<ACL SITE>_NB_API_TOKEN, the variable phase 12 reads the token from",
        ),
    ));
    if s.mgmt_fqdn.is_empty() {
        v.push(Input::unset(
            "public_mgmt_fqdn",
            format!("declare public_mgmt_fqdn in {}", vars_rel(s)),
        ));
    } else {
        v.push(Input::new(
            "public_mgmt_fqdn",
            s.mgmt_fqdn.clone(),
            gv(repo, s, "public_mgmt_fqdn"),
        ));
    }
    v.push(Input::new(
        "netbird_mgmt_url",
        format!("https://{}", s.mgmt_fqdn),
        Source::derived("public_mgmt_fqdn (above)", "https://<public_mgmt_fqdn>"),
    ));
    if s.intra_pair {
        v.push(Input::new(
            "intra_site_pair",
            "true",
            gv(repo, s, "intra_site_pair"),
        ));
    }
    v.push(Input::new(
        "obs",
        s.obs.to_string(),
        if s.obs {
            Source::derived(
                gv(repo, s, "has_obs_host").render(),
                format!(
                    "has_obs_host: true, with group_vars/{} and a host in its inventory group",
                    s.obs_group
                ),
            )
        } else {
            Source::derived(s.obs_why.clone(), "no observability leg")
        },
    ));

    // The owner, the state directory and the files under it.
    if s.owner_email != "$OWNER_EMAIL" {
        // A declared administrator reaches the bootstrap phases as data, in the
        // variable standup sets for them (standup::phase_env), not on the line.
        v.push(Input::new(
            standup::OWNER_VAR,
            s.owner_email.clone(),
            lite(repo, &["admin"]),
        ));
    } else {
        v.push(Input::reads_env(
            "OWNER_EMAIL",
            false,
            set(env, "OWNER_EMAIL"),
            None,
            "export OWNER_EMAIL, the bootstrap administrator's address, before phase 5",
        ));
    }
    v.push(Input::new(
        "state_dir",
        s.state_dir.clone(),
        deploy_or(
            repo,
            "state_dir",
            "~/.goat/state, the state directory when deploy.state_dir is not declared",
        ),
    ));
    let state_rule = |f: &str| {
        Source::derived(
            format!("state_dir {}", s.state_dir),
            format!("<state_dir>/{f}"),
        )
    };
    v.push(Input::new(
        "mesh_ips",
        format!("{}/standup-{site}-mesh-ips.json", s.state_dir),
        state_rule("standup-<site>-mesh-ips.json"),
    ));
    let state = crate::settings::state_dir(repo);
    v.push(Input::new(
        "ca_key_age",
        format!("{state}/{site}-ca/ca.key.age"),
        Source::derived(
            format!("deploy.state_dir {state}"),
            "<state_dir>/<site>-ca/ca.key.age",
        ),
    ));
    v.push(Input::new(
        "audit_log",
        crate::settings::ca_audit_log(repo),
        Source::derived(
            format!("deploy.state_dir {state}"),
            "ca-audit.jsonl beside the state directory",
        ),
    ));
    v.push(Input::new(
        "age_identity",
        "~/.config/age/keys.txt",
        Source::default("~/.config/age/keys.txt, the operator's age identity"),
    ));
    v.push(Input::new(
        "name_snapshot",
        format!("{}/ops/dns/{site}/name-snapshot.json", s.repo),
        site_rule("ops/dns/<site>/name-snapshot.json"),
    ));

    // The plays each phase runs for this site: its own file when it keeps one.
    let play = |name: &str, own: String| {
        let rule =
            format!("{own} when the site keeps one under ops/ansible, else the generic play");
        Input::new(
            name,
            own.clone(),
            Source::derived(format!("the site '{site}'"), rule),
        )
    };
    for (name, suffix) in [
        ("rathole_play", "rathole-client.yml"),
        ("seed_play", "seed-from-peer.yml"),
        ("oidc_play", "oidc-clients.yml"),
    ] {
        let chosen = standup::site_play(repo, site, suffix);
        let mut i = play(name, format!("{site}-{suffix}"));
        if chosen != format!("{site}-{suffix}") {
            i = Input::new(
                name,
                chosen,
                Source::default(format!(
                    "site-{suffix}, the generic play (the site keeps no {site}-{suffix})"
                )),
            );
        }
        v.push(i);
    }

    // Values a phase's own command fixes: the same for every site.
    for i in fixed(site) {
        push(&mut v, i);
    }
    // The rescue hosts of phase 14 (ADR 1183).
    let site_us = site.replace('-', "_");
    for suffix in ["breakglass", "network_jump"] {
        v.push(Input::new(
            format!("{suffix}_group"),
            format!("{site_us}_{suffix}_hosts"),
            site_rule(&format!("<site, - as _>_{suffix}_hosts")),
        ));
    }
    v.push(Input::new(
        "breakglass_inventory",
        format!("inventory/{site}-breakglass-nodes.ini"),
        site_rule("inventory/<site>-breakglass-nodes.ini"),
    ));
    v.push(Input::new(
        "network_jump_inventory",
        format!("inventory/{site}-network-jump.ini"),
        site_rule("inventory/<site>-network-jump.ini"),
    ));

    // What the phases read from their environment: the site's credentials
    // (by their source only) and the pub/sub mirror's token.
    let u = &s.upper;
    let store = crate::secrets::available(repo);
    let store_name = if store.is_empty() {
        String::new()
    } else {
        crate::secrets::store(repo)
    };
    for (name, secret, when) in [
        (format!("{u}_PG_PASS"), true, "phases 5, 6, 9, 10 and 12"),
        (format!("{u}_NB_ENCKEY"), true, "phases 5 and 10"),
        (format!("{u}_BKP_KEY"), true, "phase 5"),
        (format!("{u}_RELAY_SECRET"), true, "phase 7 (internet)"),
        (
            format!("CLOUDFLARE_API_TOKEN_{u}"),
            true,
            "phase 7 (internet)",
        ),
        (
            format!("{u}_NB_API_TOKEN"),
            true,
            "phase 12; phase 11 issues it",
        ),
        (
            "FORGEJO_PUBSUB_TOKEN".to_string(),
            true,
            "phase 12 (the pub/sub state mirror)",
        ),
    ] {
        let from_store = store.contains(&name).then(|| {
            Source::derived(
                format!("the declared secrets store ({store_name})"),
                "loaded into each phase's environment by ops/scripts/standup-phase-run",
            )
        });
        let hint = crate::secrets::hint(repo, std::slice::from_ref(&name));
        v.push(Input::reads_env(
            name.clone(),
            secret,
            set(env, &name),
            from_store,
            format!("read by {when}; {hint}"),
        ));
    }
    v
}

/// The values the phases' own commands fix, the same for every site, each
/// with why (standup and join-node share them).
pub(super) fn fixed(site: &str) -> Vec<Input> {
    let fixed = |name: &str, value: &str, why: &str| {
        Input::new(name, value, Source::default(why.to_string()))
    };
    vec![
        fixed("skip_tags after-ory", "after-ory", "standup phase 5 runs site-bootstrap's stack plays; the account seed waits for Ory"),
        fixed("skip_tags stack", "stack", "standup phase 6 runs the rest of site-bootstrap, after Ory"),
        fixed("participates_in_spock", "false", "a joiner's first bootstrap pass runs without Spock; pass 3 turns it on"),
        fixed("has_durable_independent_state", "true", "a joiner that is not an intra-site pair keeps its own durable state"),
        fixed("ory_spock_wire_subscriptions", "false", "pass 3 wires the Ory node and repset only; the subscriptions come after the founder's half"),
        fixed("spire_enabled", "true", "the SPIRE server runs on the founder"),
        fixed("lan_tags", "lan-dns,traefik,stun", "the LAN service surface (standup phase 10)"),
        fixed("mode", "enforce", "the declared NetBird ACL is applied in enforce mode (standup phase 12)"),
        fixed("confirm_cutover", "YES", "the ACL cutover is confirmed by running the phase (standup phase 12)"),
        fixed("relay_target", "goat_relay_hosts", "the relay ingress group a joiner is wired into"),
        fixed("target_site", site, "the site itself"),
        Input::new("VF", "$(mktemp -t goat-relay-vars.XXXXXX)", Source::derived("mktemp, when standup phase 7 runs", "a private file for the relay secrets, removed after the play")),
        fixed("acl_inventory", "localhost,", "the ACL plays run on this machine against the NetBird API"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words of a command line that the conformance test asks a source for
    /// (tests/legibility.rs, check b), and standup's own options.
    fn values(line: &str) -> Vec<(bool, String)> {
        super::super::command_values(
            line,
            &[
                "--obs",
                "--ips",
                "--group",
                "--token-env",
                "--snapshot",
                "--root-key",
                "--age-identity",
                "--audit-log",
                "--url",
            ],
        )
    }

    fn site(joiner: bool, profile: &str, lite: bool, obs: bool, intra_pair: bool) -> Site {
        let pre = if lite && profile == "lan" {
            "OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE=none ../scripts/with-aws default -- ../scripts/ansible-preflight.sh"
        } else {
            "OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws default -- ../scripts/ansible-preflight.sh"
        };
        Site {
            repo: "/r".into(),
            site: "zagros-b".into(),
            group: "zagros_b_hosts".into(),
            upper: "ZAGROS_B".into(),
            obs_group: "zagros_b_obs_hosts".into(),
            state_dir: "/h/.goat/state".into(),
            ansible_pre: pre.into(),
            inv: standup::INV,
            profile: profile.into(),
            joiner,
            join_founder_site: if joiner {
                "zagros-a".into()
            } else {
                String::new()
            },
            join_founder_group: if joiner {
                "zagros_a_hosts".into()
            } else {
                String::new()
            },
            acl_site: if joiner {
                "zagros-a".into()
            } else {
                "zagros-b".into()
            },
            mgmt_fqdn: "mgmt.zagros-b.example.org".into(),
            intra_pair,
            obs,
            obs_why: if obs {
                String::new()
            } else {
                "--skip-obs".into()
            },
            group_vars_exists: true,
            owner_email: if lite {
                "a.learner@example.org".into()
            } else {
                "$OWNER_EMAIL".into()
            },
            lite,
            nb_token_available: false,
        }
    }

    /// Check (b), for standup, on the library: in every branch of the plan
    /// (founder and joiner, lan and internet, goatlite, an obs leg), every value
    /// of every command line has an entry in `inputs` with a source.
    #[test]
    fn every_value_in_a_standup_command_has_a_sourced_input() {
        let repo = Path::new("/r");
        let env = |_: &str| None;
        for (joiner, profile, lite, obs, pair) in [
            (false, "lan", false, false, false),
            (false, "internet", false, true, false),
            (true, "internet", false, false, false),
            (true, "lan", false, false, true),
            (false, "lan", true, false, false),
        ] {
            let s = site(joiner, profile, lite, obs, pair);
            let o = Options {
                env: "zagros",
                site: "zagros-b",
                profile: Some(profile),
                from: 1,
                to: 14,
                skip_obs: !obs,
                obs_only: false,
                dry_run: true,
                parallel: false,
                check: true,
            };
            let inputs = site_inputs(repo, &s, &o, &env);
            let mut missing = Vec::new();
            for c in standup::plan_commands(repo, &s, &o) {
                for (from_env, val) in values(&c) {
                    let ok = inputs.iter().any(|i| {
                        i.sourced()
                            && if from_env {
                                i.name == val
                            } else {
                                i.value() == Some(val.as_str())
                            }
                    });
                    if !ok {
                        missing.push(val);
                    }
                }
            }
            missing.sort();
            missing.dedup();
            assert!(missing.is_empty(), "joiner={joiner} profile={profile} lite={lite} obs={obs}: values with no sourced input: {missing:?}");
        }
    }

    /// A declared AWS profile that has to be quoted on the command line is
    /// listed as the shell reads it, not as the quoted text.
    #[test]
    fn a_quoted_aws_profile_is_listed_as_the_shell_reads_it() {
        let mut s = site(false, "lan", false, false, false);
        let profile = "p$(x) 'q\"r";
        let quoted = format!("'{}'", profile.replace('\'', "'\\''"));
        s.ansible_pre = format!("OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {quoted} -- ../scripts/ansible-preflight.sh");
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            profile: Some("lan"),
            from: 1,
            to: 14,
            skip_obs: true,
            obs_only: false,
            dry_run: true,
            parallel: false,
            check: false,
        };
        let env = |_: &str| None;
        let inputs = site_inputs(Path::new("/r"), &s, &o, &env);
        let aws = inputs.iter().find(|i| i.name == "aws_profile").unwrap();
        assert_eq!(aws.value(), Some(profile));
        // And check (b)'s reading of the plan's commands finds it.
        let words = super::super::command_values(
            &format!("cd /r/ops/ansible && {} site.yml", s.ansible_pre),
            &[],
        );
        assert!(words.contains(&(false, profile.to_string())), "{words:?}");
    }

    /// A prefix the shell cannot read (an unbalanced quote), or one that names
    /// no profile, is listed as missing, never as an empty profile.
    #[test]
    fn an_unreadable_aws_profile_is_missing_not_empty() {
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            profile: Some("lan"),
            from: 1,
            to: 14,
            skip_obs: true,
            obs_only: false,
            dry_run: true,
            parallel: false,
            check: false,
        };
        let env = |_: &str| None;
        for pre in [
            "OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws 'p q -- ../scripts/ansible-preflight.sh",
            "OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws -- ../scripts/ansible-preflight.sh",
            "OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws",
        ] {
            let mut s = site(false, "lan", false, false, false);
            s.ansible_pre = pre.into();
            let inputs = site_inputs(Path::new("/r"), &s, &o, &env);
            let aws = inputs.iter().find(|i| i.name == "aws_profile").unwrap();
            assert!(!aws.sourced() && aws.value().is_none(), "{pre}: {aws:?}");
            assert!(aws.unset.as_deref().unwrap().contains("deploy.aws_profile"), "{pre}");
        }
    }

    /// A credential is listed by its source and never by its value, set or not.
    #[test]
    fn a_credential_shows_only_its_source() {
        let s = site(false, "lan", false, false, false);
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            profile: None,
            from: 1,
            to: 14,
            skip_obs: true,
            obs_only: false,
            dry_run: true,
            parallel: false,
            check: false,
        };
        let env = |k: &str| (k == "ZAGROS_B_PG_PASS").then(|| "correct-horse".to_string());
        let inputs = site_inputs(Path::new("/r"), &s, &o, &env);
        let pg = inputs
            .iter()
            .find(|i| i.name == "ZAGROS_B_PG_PASS")
            .unwrap();
        assert!(pg.secret && pg.value().is_none());
        assert_eq!(pg.source, Source::env("ZAGROS_B_PG_PASS"));
        let all = serde_json::to_string(&inputs).unwrap();
        assert!(!all.contains("correct-horse"));
        assert!(!super::super::rows(&inputs)
            .iter()
            .any(|r| r.detail.contains("correct-horse")));
        let bkp = inputs
            .iter()
            .find(|i| i.name == "ZAGROS_B_BKP_KEY")
            .unwrap();
        assert!(
            bkp.secret && bkp.value().is_none() && bkp.unset.is_some(),
            "{bkp:?}"
        );
    }

    /// On a checkout: what the site's vars.yml declares is named at its line,
    /// and the profile and AWS profile name the rule that chose them.
    #[test]
    fn group_vars_values_name_their_line() {
        let root = crate::testdir::path("goatbuilder-inputs-standup-");
        let repo = root.join("repo");
        let gv = repo.join("ops/ansible/group_vars/zagros_b_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        std::fs::write(gv.join("vars.yml"), "site_id: zagros-b\ngoatnet_founder: false\njoin_founder_site: zagros-a\npublic_mgmt_fqdn: mgmt.zagros-b.example.org\ndashboard_cert_delivery_mode: acme\n").unwrap();
        let s = site(true, "internet", false, false, false);
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            profile: None,
            from: 1,
            to: 14,
            skip_obs: true,
            obs_only: false,
            dry_run: true,
            parallel: false,
            check: false,
        };
        let env = |_: &str| None;
        let inputs = site_inputs(&repo, &s, &o, &env);
        let render = |n: &str| {
            inputs
                .iter()
                .find(|i| i.name == n)
                .map(|i| i.source.render())
                .unwrap_or_else(|| panic!("{n}"))
        };
        assert_eq!(
            render("join_founder_site"),
            "ops/ansible/group_vars/zagros_b_hosts/vars.yml:3 join_founder_site"
        );
        assert_eq!(
            render("public_mgmt_fqdn"),
            "ops/ansible/group_vars/zagros_b_hosts/vars.yml:4 public_mgmt_fqdn"
        );
        assert_eq!(render("profile"), "derived from ops/ansible/group_vars/zagros_b_hosts/vars.yml:5 dashboard_cert_delivery_mode by acme is the internet profile");
        assert!(
            render("aws_profile").starts_with("default: "),
            "{}",
            render("aws_profile")
        );
        assert_eq!(render("acl_site"), "derived from join_founder_site (above) by a joiner is governed by its founder's NetBird ACL");
    }
}
