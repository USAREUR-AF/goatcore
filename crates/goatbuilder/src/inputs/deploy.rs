//! Every value `goatbuilder deploy --env <goatnet> --component <c>` uses, with
//! its source: the command line, what the pins declare about the component and
//! the goatnet (each at its line in ops/pins/fleet-pins.yaml), the play and how
//! it is targeted, the transport and its inventories, each group's declared
//! ref (at its line in group_vars/<group>/pins.yml), the AWS profile, and the
//! variables the play templates from the environment (by their source only).
//!
//! The values are the ones `push::plan` resolves; this module says where each
//! came from. The test runs a deploy plan and finds every value of every
//! command line the run would start (the play per group and the ledger read)
//! here.

use std::path::Path;

use anyhow::Result;

use super::{rel, yaml_key_line, yaml_line, Input, Seg, Source};
use crate::push::{self, Options, Plan};

/// Reads the operator's environment.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn set(env: Env, name: &str) -> Option<String> {
    env(name).filter(|v| !v.is_empty())
}

const PINS: &str = "ops/pins/fleet-pins.yaml";

/// The inputs of a deploy.
pub fn inputs(repo: &Path, o: &Options, env: Env) -> Result<Vec<Input>> {
    let p = push::plan(repo, o)?;
    Ok(plan_inputs(repo, o, &p, env))
}

/// The inputs of a deploy whose plan is `p`.
pub fn plan_inputs(repo: &Path, o: &Options, p: &Plan, env: Env) -> Vec<Input> {
    let mut v: Vec<Input> = Vec::new();
    let r = &p.resolved;
    let pins = std::fs::read_to_string(repo.join(PINS)).unwrap_or_default();
    let pin = |path: &[Seg], key: String| Source::file(PINS, yaml_line(&pins, path), key);
    let c = o.component;

    // The command line.
    v.push(Input::new("component", c, Source::arg("--component")));
    v.push(Input::new("goatnet", o.goatnet, Source::arg("--env")));
    v.push(Input::new(
        "repo",
        repo.display().to_string(),
        Source::arg("-C, else the current directory"),
    ));
    v.push(Input::new(
        "name",
        p.name.clone(),
        Source::derived("goatnet (above)", "the goatnet with _ as -"),
    ));
    if !o.ack.trim().is_empty() {
        v.push(Input::new("ack", o.ack, Source::arg("--ack")));
    }

    // What the pins declare.
    use Seg::Key;
    v.push(Input::new(
        "var",
        r.var.clone(),
        pin(
            &[Key("components"), Key(c), Key("var")],
            format!("components.{c}.var"),
        ),
    ));
    let ledger_line = yaml_line(&pins, &[Key("components"), Key(c), Key("ledger_component")]);
    v.push(Input::new(
        "ledger",
        r.ledger.clone(),
        match ledger_line {
            Some(n) => Source::file(PINS, Some(n), format!("components.{c}.ledger_component")),
            None => {
                Source::default("the component's own name, when it declares no ledger_component")
            }
        },
    ));
    let members_src = match o.only_group {
        Some(_) => Source::arg("--group"),
        None => pin(
            &[Key("goatnets"), Key(o.goatnet), Key("members")],
            format!("goatnets.{}.members", o.goatnet),
        ),
    };
    for g in &r.members {
        v.push(Input::new(
            format!("group {g}"),
            g.clone(),
            members_src.clone(),
        ));
    }
    for g in &r.skipped {
        v.push(Input::new(
            format!("skipped {g}"),
            g.clone(),
            pin(
                &[
                    Key("components"),
                    Key(c),
                    Key("deviations"),
                    Key(g),
                    Key("absent"),
                ],
                format!("components.{c}.deviations.{g}.absent"),
            ),
        ));
    }

    // The play and how it is targeted.
    let play = r
        .play
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let declared_play = yaml_line(&pins, &[Key("components"), Key(c), Key("deploy_playbook")]);
    v.push(Input::new(
        "play",
        play.clone(),
        match declared_play {
            Some(n) => Source::file(PINS, Some(n), format!("components.{c}.deploy_playbook")),
            None => Source::derived(
                format!("the component '{c}' and its ledger '{}'", r.ledger),
                "<ledger>-deploy.yml, else <component>-deploy.yml, the first in ops/ansible",
            ),
        },
    ));
    let hosts_play = rel(repo, &r.hosts_play);
    let hosts_text = std::fs::read_to_string(&r.hosts_play).unwrap_or_default();
    let hosts_line = hosts_text
        .lines()
        .position(|l| l.trim_start().starts_with("hosts:"))
        .map(|i| i + 1);
    v.push(Input::new(
        "target_flag",
        r.target_flag.trim().to_string(),
        Source::derived(
            Source::file(hosts_play, hosts_line, "hosts").render(),
            "a templated hosts: is targeted by -e <var>=<group>, a literal one by -l <group>",
        ),
    ));
    if !r.write_role.is_empty() {
        v.push(Input::new(
            "single_homed_role",
            r.write_role.clone(),
            pin(
                &[Key("components"), Key(c), Key("single_homed_role")],
                format!("components.{c}.single_homed_role"),
            ),
        ));
    }

    // The transport and its inventories.
    for i in &p.transport.inventories {
        v.push(Input::new(
            format!("inventory {i}"),
            i.clone(),
            Source::derived(
                format!("the transport: {}", p.transport.detail),
                "the base inventory, and the mesh or door override",
            ),
        ));
    }

    // Each group's declared ref.
    for (g, w) in &p.wants {
        let f = format!("ops/ansible/group_vars/{g}/pins.yml");
        let text = std::fs::read_to_string(repo.join(&f)).unwrap_or_default();
        v.push(Input::new(
            format!("{} ({g})", r.var),
            w.clone(),
            Source::file(f, yaml_key_line(&text, &r.var), r.var.clone()),
        ));
    }

    // The AWS profile and the ledger-read and play words the run fixes.
    let aws_src = super::aws_profile_source(repo, env);
    v.push(Input::new(
        "aws_profile",
        crate::settings::aws_profile(repo),
        aws_src,
    ));
    v.push(Input::new("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES", Source::default("set on every play, so Ansible's forked workers run on macOS; it changes nothing elsewhere")));
    v.push(Input::new(
        "ledger_read",
        format!(
            "tail -n 1 /var/lib/goat/{}/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER",
            r.ledger
        ),
        Source::derived(
            "ledger (above)",
            "the newest row of /var/lib/goat/<ledger>/deploy-history.jsonl on each host",
        ),
    ));

    // What the play templates from the environment.
    let store = crate::secrets::available(repo);
    let store_name = if store.is_empty() {
        String::new()
    } else {
        crate::secrets::store(repo)
    };
    for (var, from) in &p.secrets {
        let from_store = store.contains(from).then(|| {
            Source::derived(
                format!("the declared secrets store ({store_name})"),
                "read into the play's environment",
            )
        });
        let hint = crate::secrets::hint(repo, std::slice::from_ref(from));
        v.push(Input::reads_env(
            from.clone(),
            true,
            set(env, from),
            from_store,
            format!("the play templates {var} from it; {hint}"),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &str = "ghcr.io/example-org/portal@sha256:1111111111111111111111111111111111111111111111111111111111111111";

    fn checkout() -> crate::testdir::Held<std::path::PathBuf> {
        let root = crate::testdir::path("goatbuilder-inputs-deploy-");
        let w = |p: &str, t: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        w(PINS, &format!("goatnets:\n  dev:\n    members: [dev_hosts, dev_edge_hosts]\ncomponents:\n  portal:\n    var: portal_image\n    fleet: \"{REF}\"\n    deviations:\n      dev_edge_hosts:\n        absent: true\n"));
        w(
            "ops/ansible/portal-deploy.yml",
            "- name: portal\n  hosts: \"{{ target }}\"\n  tasks: []\n",
        );
        w(
            "ops/ansible/inventory/mgmt-hosts.ini",
            "[dev_hosts]\ndev-core-a ansible_host=192.0.2.11\n",
        );
        w(
            "ops/ansible/group_vars/dev_hosts/pins.yml",
            &format!("# the declared ref\nportal_image: \"{REF}\"\n"),
        );
        root
    }

    /// The command lines a deploy starts, as `push::deploy` builds them: the
    /// play per group (its DRY row prints the words) and the ledger read.
    fn commands(repo: &Path, o: &Options, p: &Plan) -> Vec<String> {
        let report = push::deploy(repo, o).unwrap();
        let mut v: Vec<String> = report
            .checks
            .iter()
            .filter_map(|c| {
                c.detail
                    .split_once(": (cd ops/ansible && ")
                    .map(|(_, rest)| {
                        rest.rsplit_once(") > ")
                            .map(|x| x.0)
                            .unwrap_or(rest)
                            .to_string()
                    })
            })
            .collect();
        for g in &p.resolved.members {
            let words =
                push::ledger_read_args(&push::inventory_args(&p.transport), g, &p.resolved.ledger);
            v.push(format!(
                "../scripts/with-aws {} -- ansible {}",
                crate::settings::aws_profile(repo),
                shlex::try_join(words.iter().map(String::as_str)).unwrap()
            ));
        }
        v
    }

    /// Check (b), for deploy, on the library: every value of every command line
    /// a deploy starts has an entry in `inputs` with a source.
    #[test]
    fn every_value_in_a_deploy_command_has_a_sourced_input() {
        let repo = checkout();
        let o = Options {
            component: "portal",
            goatnet: "dev",
            only_group: None,
            dry_run: true,
            verify_only: false,
            ack: "",
        };
        let env = |_: &str| None;
        let p = push::plan(&repo, &o).unwrap();
        let cmds = commands(&repo, &o, &p);
        assert_eq!(cmds.len(), 2, "{cmds:?}");
        let inputs = plan_inputs(&repo, &o, &p, &env);
        let mut missing = Vec::new();
        for c in &cmds {
            for (from_env, val) in super::super::command_values(c, &["-a"]) {
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
            // `-e target=<group>` is one word in the play's arguments; the line
            // is quoted as a shell reads it (runner::line_of), so it is read back
            // as one.
            let words = shlex::split(c).unwrap_or_else(|| c.split_whitespace().map(String::from).collect());
            for w in words.iter().filter_map(|w| w.strip_prefix("-e target=").or_else(|| w.strip_prefix("target="))) {
                assert!(
                    inputs.iter().any(|i| i.value() == Some(w) && i.sourced()),
                    "{w}"
                );
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "values with no sourced input: {missing:?}\n{cmds:#?}"
        );
    }

    #[test]
    fn deploy_values_name_their_line() {
        let repo = checkout();
        let o = Options {
            component: "portal",
            goatnet: "dev",
            only_group: None,
            dry_run: true,
            verify_only: false,
            ack: "",
        };
        let env = |_: &str| None;
        let inputs = inputs(&repo, &o, &env).unwrap();
        let render = |n: &str| {
            inputs
                .iter()
                .find(|i| i.name == n)
                .map(|i| i.source.render())
                .unwrap_or_else(|| panic!("{n}: {inputs:#?}"))
        };
        assert_eq!(
            render("var"),
            "ops/pins/fleet-pins.yaml:6 components.portal.var"
        );
        assert_eq!(
            render("group dev_hosts"),
            "ops/pins/fleet-pins.yaml:3 goatnets.dev.members"
        );
        assert_eq!(
            render("skipped dev_edge_hosts"),
            "ops/pins/fleet-pins.yaml:10 components.portal.deviations.dev_edge_hosts.absent"
        );
        assert_eq!(
            render("portal_image (dev_hosts)"),
            "ops/ansible/group_vars/dev_hosts/pins.yml:2 portal_image"
        );
        assert!(
            render("play").starts_with("derived from the component 'portal'"),
            "{}",
            render("play")
        );
        assert_eq!(render("target_flag"), "derived from ops/ansible/portal-deploy.yml:2 hosts by a templated hosts: is targeted by -e <var>=<group>, a literal one by -l <group>");
        assert_eq!(
            render("ledger"),
            "default: the component's own name, when it declares no ledger_component"
        );
        // Every input a derived value names is listed, so `why` follows it.
        assert_eq!(super::super::dangling(&inputs), Vec::<String>::new());
        assert_eq!(
            super::super::why(&inputs, "name"),
            [
                "name=dev  derived from goatnet (above) by the goatnet with _ as -",
                "  goatnet=dev  argument --env"
            ]
        );
    }
}
