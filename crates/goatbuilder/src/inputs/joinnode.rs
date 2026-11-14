//! Every value `goatbuilder join-node <env> --site <site> --node <node>
//! --class <class>` uses, with its source: the command line, the groups and
//! inventories derived from the site, class and transport, what the site's
//! core group_vars declare (each at its line), the project's SSH user and AWS
//! profile, the values the phases fix, and the variables the phases read from
//! their environment when they run (credentials among them, by their source
//! only).
//!
//! The values are the ones `joinnode::plan` derives, by the same rules; the
//! test runs every class and both transports and finds every value of every
//! command line here.

use std::path::Path;

use super::standup::fixed;
use super::{push, yaml_key_line, Input, Source};
use crate::joinnode::{self, Options};

/// Reads the operator's environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn set(env: Env, name: &str) -> Option<String> {
    env(name).filter(|v| !v.is_empty())
}

/// The inputs of a join-node run.
pub fn inputs(repo: &Path, o: &Options, env: Env) -> Vec<Input> {
    let mut v: Vec<Input> = Vec::new();
    let site = o.site;
    let site_us = site.replace('-', "_");
    let upper = site_us.to_uppercase();
    let r = repo.display().to_string();
    let site_rule = |what: &str| Source::derived(format!("the site '{site}'"), what.to_string());

    // The command line.
    v.push(Input::new(
        "environment",
        o.env,
        Source::arg("<environment>"),
    ));
    v.push(Input::new("site", site, Source::arg("--site")));
    v.push(Input::new("node", o.node, Source::arg("--node")));
    v.push(Input::new("class", o.class, Source::arg("--class")));
    v.push(Input::new(
        "via",
        o.via,
        Source::arg("--via (default mesh)"),
    ));
    v.push(Input::new(
        "repo",
        r.clone(),
        Source::arg("-C, else the current directory"),
    ));
    match o.address {
        Some(a) => v.push(Input::new("address", a, Source::arg("--address"))),
        None => v.push(Input::unset(
            "address",
            "pass --address <the node's LAN address> (phase 1 says how to find it)",
        )),
    }
    let pass_var = match o.pass_var {
        Some(p) => {
            v.push(Input::new("pass_var", p, Source::arg("--pass-var")));
            p.to_string()
        }
        None => {
            let p = format!("{upper}_NODE_PASS");
            v.push(Input::new(
                "pass_var",
                p.clone(),
                site_rule("<SITE>_NODE_PASS, the variable phase 2 reads the node's password from"),
            ));
            p
        }
    };

    // The groups and inventories.
    let site_group = format!("{site_us}_hosts");
    let group = joinnode::group_of(site, o.class);
    v.push(Input::new(
        "site_group",
        site_group.clone(),
        site_rule("<site, - as _>_hosts, the group that carries the site's facts"),
    ));
    v.push(Input::new(
        "group",
        group.clone(),
        Source::derived(
            format!("the site '{site}' and the class '{}'", o.class),
            "<site>_hosts for core, else <site>_<class>_hosts",
        ),
    ));
    for i in joinnode::inventories_of(o.via)
        .split_whitespace()
        .filter(|w| *w != "-i")
    {
        v.push(Input::new(
            format!("inventory {i}"),
            i,
            Source::derived(
                format!("--via {}", o.via),
                "the base inventory and exactly one transport override",
            ),
        ));
    }
    for (class, suffix) in [
        ("breakglass", "breakglass-nodes"),
        ("network-jump", "network-jump"),
    ] {
        if o.class == class {
            let g = format!("{site_us}_{}_hosts", class.replace('-', "_"));
            v.push(Input::new(
                "rescue_group",
                g,
                site_rule(&format!("<site>_{}_hosts", class.replace('-', "_"))),
            ));
            v.push(Input::new(
                "rescue_inventory",
                format!("inventory/{site}-{suffix}.ini"),
                site_rule(&format!("inventory/<site>-{suffix}.ini")),
            ));
        }
    }

    // The project's SSH user and AWS profile.
    match crate::settings::ssh_user(repo) {
        Some(u) => {
            // settings::ssh_user's order: deploy.ssh_user when it names a user,
            // else lite.ssh_user.
            let src = super::setting_line(repo, &["deploy", "ssh_user"])
                .or_else(|| super::setting_line(repo, &["lite", "ssh_user"]))
                .unwrap_or_else(|| Source::derived("settings::ssh_user", "the project's SSH user"));
            v.push(Input::new("ssh_user", u.clone(), src));
            if let Some(a) = o.address {
                v.push(Input::new(
                    "ssh_target",
                    format!("{u}@{a}"),
                    Source::derived("ssh_user and address (above)", "<ssh_user>@<address>"),
                ));
            }
        }
        None => v.push(Input::unset(
            "ssh_user",
            "set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml",
        )),
    }
    let aws_src = super::aws_profile_source(repo, env);
    v.push(Input::new(
        "aws_profile",
        crate::settings::aws_profile(repo),
        aws_src,
    ));
    v.push(Input::new("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES", Source::default("set on every play, so Ansible's forked workers run on macOS; it changes nothing elsewhere")));

    // What the site's core group_vars declare.
    let gv_rel = format!("ops/ansible/group_vars/{site_group}/vars.yml");
    let gv_path = repo.join(&gv_rel);
    let text = std::fs::read_to_string(&gv_path).unwrap_or_default();
    let at = |key: &str| Source::file(gv_rel.clone(), yaml_key_line(&text, key), key);
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).unwrap_or(serde_yaml::Value::Null);
    let plain = |key: &str| -> Option<String> {
        match doc.get(key)? {
            serde_yaml::Value::String(s) => Some(s.clone()),
            serde_yaml::Value::Bool(b) => Some(b.to_string()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let joiner = plain("goatnet_founder").as_deref() == Some("false");
    if joiner {
        v.push(Input::new(
            "goatnet_founder",
            "false",
            at("goatnet_founder"),
        ));
    }
    let founder = plain("join_founder_site").unwrap_or_default();
    if !founder.is_empty() {
        let f = founder.as_str();
        v.push(Input::new("join_founder_site", f, at("join_founder_site")));
        let from = || "join_founder_site (above)".to_string();
        v.push(Input::new(
            "founder_group",
            format!("{}_hosts", f.replace('-', "_")),
            Source::derived(from(), "<founder, - as _>_hosts"),
        ));
        v.push(Input::new(
            "founder_relay_inventory",
            format!("inventory/hosts-{f}.ini"),
            Source::derived(from(), "inventory/hosts-<founder>.ini"),
        ));
        v.push(Input::new(
            "founder_relay_alias_inventory",
            format!("inventory/{f}-goat-relay-alias.ini"),
            Source::derived(from(), "inventory/<founder>-goat-relay-alias.ini"),
        ));
        v.push(Input::new(
            "founder_rathole_play",
            crate::standup::site_play(repo, f, "rathole-client.yml"),
            Source::derived(
                from(),
                "<founder>-rathole-client.yml when it keeps one, else site-rathole-client.yml",
            ),
        ));
    }
    if plain("intra_site_pair").as_deref() == Some("true") {
        v.push(Input::new("intra_site_pair", "true", at("intra_site_pair")));
    }
    match plain("site_enrollment_dns") {
        Some(d) => {
            v.push(Input::new(
                "site_enrollment_dns",
                d.clone(),
                at("site_enrollment_dns"),
            ));
            v.push(Input::new(
                "NETBIRD_MANAGEMENT_URL",
                format!("https://{d}"),
                Source::derived(
                    "site_enrollment_dns (above)",
                    "https://<site_enrollment_dns>",
                ),
            ));
        }
        None => v.push(Input::unset(
            "site_enrollment_dns",
            format!("declare site_enrollment_dns in {gv_rel}"),
        )),
    }
    v.push(Input::new(
        "seed_play",
        crate::standup::site_play(repo, site, "seed-from-peer.yml"),
        site_rule("<site>-seed-from-peer.yml when it keeps one, else site-seed-from-peer.yml"),
    ));
    v.push(Input::new(
        "acceptance_values",
        format!("{r}/ops/validation/values-{site}.yaml"),
        site_rule("ops/validation/values-<site>.yaml, written by generate-values.py"),
    ));
    for i in fixed(site) {
        push(&mut v, i);
    }

    // What the phases read from their environment.
    let store = crate::secrets::available(repo);
    let store_name = if store.is_empty() {
        String::new()
    } else {
        crate::secrets::store(repo)
    };
    for (name, secret, when) in [
        (
            pass_var.clone(),
            true,
            "phase 2 (seed-ssh-key.sh), unless key auth already works",
        ),
        (
            format!("{upper}_NB_SETUP_KEY"),
            true,
            "phase 9; phase 8 mints it",
        ),
        (format!("{upper}_PG_PASS"), true, "phase 13 (core)"),
        (format!("{upper}_NB_ENCKEY"), true, "phase 13 (core)"),
        (format!("{upper}_BKP_KEY"), true, "phase 13 (core)"),
        (
            "OWNER_EMAIL".to_string(),
            false,
            "phase 13 (core): the bootstrap administrator's address",
        ),
    ] {
        let from_store = store.contains(&name).then(|| {
            Source::derived(
                format!("the declared secrets store ({store_name})"),
                "read into the phase's environment",
            )
        });
        let hint = if secret {
            crate::secrets::hint(repo, std::slice::from_ref(&name))
        } else {
            format!("export {name}")
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(joiner: bool) -> crate::testdir::Held<std::path::PathBuf> {
        let root = crate::testdir::path("goatbuilder-inputs-join-");
        let gv = root.join("ops/ansible/group_vars/zagros_b_hosts");
        std::fs::create_dir_all(&gv).unwrap();
        let mut vars =
            String::from("site_id: zagros-b\nsite_enrollment_dns: mesh.zagros.example.org\n");
        if joiner {
            vars.push_str("goatnet_founder: false\njoin_founder_site: zagros-a\n");
        }
        std::fs::write(gv.join("vars.yml"), vars).unwrap();
        std::fs::create_dir_all(root.join("ops/goatbuilder")).unwrap();
        std::fs::write(
            root.join("ops/goatbuilder/goatbuilder.yaml"),
            "schema: goatbuilder/v1\ndeploy:\n  ssh_user: operator\n",
        )
        .unwrap();
        // The obs plays the obs class needs in the tree.
        std::fs::write(root.join("ops/ansible/utility-host-bootstrap.yml"), "[]\n").unwrap();
        root
    }

    /// The SSH user is sourced to the line that gives it: an empty
    /// `deploy.ssh_user` gives nothing, so `lite.ssh_user`'s line is named.
    #[test]
    fn the_ssh_user_names_the_line_that_gives_it() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("ops/goatbuilder/goatbuilder.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        let none = |_: &str| None;
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            node: "mgmt-zagros-b-02",
            class: "mesh",
            address: None,
            pass_var: None,
            via: "mesh",
            from: 1,
            to: 15,
            dry_run: true,
        };
        let src = |text: &str| {
            std::fs::write(&cfg, text).unwrap();
            inputs(d.path(), &o, &none)
                .into_iter()
                .find(|i| i.name == "ssh_user")
                .map(|i| (i.value().map(String::from), i.source))
                .unwrap()
        };
        let y = "ops/goatbuilder/goatbuilder.yaml";
        assert_eq!(
            src("schema: goatbuilder/v1\ndeploy:\n  ssh_user: \"\"\nlite:\n  ssh_user: op\n"),
            (Some("op".into()), Source::file(y, Some(5), "lite.ssh_user"))
        );
        assert_eq!(
            src("schema: goatbuilder/v1\ndeploy:\n  ssh_user: ops\nlite:\n  ssh_user: op\n"),
            (
                Some("ops".into()),
                Source::file(y, Some(3), "deploy.ssh_user")
            )
        );
        assert_eq!(
            src("schema: goatbuilder/v1\nlite:\n  ssh_user: op\n"),
            (Some("op".into()), Source::file(y, Some(3), "lite.ssh_user"))
        );
    }

    /// Check (b), for join-node, on the library: for every class, founder and
    /// joiner, mesh and door, every value of every command line has an entry in
    /// `inputs` with a source.
    #[test]
    fn every_value_in_a_join_node_command_has_a_sourced_input() {
        let env = |_: &str| None;
        for joiner in [false, true] {
            let repo = checkout(joiner);
            for class in joinnode::CLASSES {
                for via in joinnode::VIAS {
                    let o = Options {
                        env: "zagros",
                        site: "zagros-b",
                        node: "mgmt-zagros-b-02",
                        class,
                        address: Some("192.0.2.30"),
                        pass_var: None,
                        via,
                        from: 1,
                        to: 15,
                        dry_run: true,
                    };
                    let inputs = inputs(&repo, &o, &env);
                    let mut missing = Vec::new();
                    for c in joinnode::plan_commands(&repo, &o).unwrap() {
                        for (from_env, val) in super::super::command_values(
                            &c,
                            &["--obs", "--peer", "--values", "--pass-var"],
                        ) {
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
                    assert!(missing.is_empty(), "joiner={joiner} class={class} via={via}: values with no sourced input: {missing:?}");
                }
            }
        }
    }

    #[test]
    fn join_node_values_name_their_line_and_hold_no_credential() {
        let repo = checkout(true);
        let env = |k: &str| (k == "ZAGROS_B_NODE_PASS").then(|| "hunter2".to_string());
        let o = Options {
            env: "zagros",
            site: "zagros-b",
            node: "mgmt-zagros-b-02",
            class: "core",
            address: Some("192.0.2.30"),
            pass_var: None,
            via: "mesh",
            from: 1,
            to: 15,
            dry_run: true,
        };
        let inputs = inputs(&repo, &o, &env);
        let render = |n: &str| {
            inputs
                .iter()
                .find(|i| i.name == n)
                .map(|i| i.source.render())
                .unwrap_or_else(|| panic!("{n}"))
        };
        assert_eq!(
            render("join_founder_site"),
            "ops/ansible/group_vars/zagros_b_hosts/vars.yml:4 join_founder_site"
        );
        assert_eq!(
            render("site_enrollment_dns"),
            "ops/ansible/group_vars/zagros_b_hosts/vars.yml:2 site_enrollment_dns"
        );
        assert_eq!(
            render("ssh_user"),
            "ops/goatbuilder/goatbuilder.yaml:3 deploy.ssh_user"
        );
        assert_eq!(super::super::dangling(&inputs), Vec::<String>::new());
        // A value derived from two inputs follows both, each indented under it.
        let line = |n: &str| inputs.iter().find(|i| i.name == n).unwrap().line();
        assert_eq!(
            super::super::why(&inputs, "operator@192.0.2.30"),
            [
                line("ssh_target"),
                format!("  {}", line("ssh_user")),
                format!("  {}", line("address")),
            ]
        );
        assert_eq!(render("node"), "argument --node");
        let pass = inputs
            .iter()
            .find(|i| i.name == "ZAGROS_B_NODE_PASS")
            .unwrap();
        assert!(pass.secret && pass.value().is_none());
        assert!(!serde_json::to_string(&inputs).unwrap().contains("hunter2"));
    }
}
