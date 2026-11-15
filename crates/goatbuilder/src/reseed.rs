//! `join-node <env> --site <site> --node <node> --class <core|phone> --reseed`:
//! seed a replicating node again after the founder invalidated its slot
//! (docs/design/goatbuilder-hardware-measurement.md, "How long a phone may be
//! off"; the procedure is measured on two postgres-spock containers there).
//!
//! Six steps, each one call of `ops/scripts/spock-reseed.py`, which says what
//! it checks and refuses with its own catalog code:
//!
//!   1  lost       the founder's slot for one of the node's subscriptions is lost (GB-345)
//!   2  caught-up  the founder has taken the node's own writes (GB-346)
//!   3  size       the copy is well under the node's own slot cap (GB-347)
//!   4  reseed     sub_drop, sub_create with no copy, sub_resync_table with truncate
//!   5  synced     every table copied, the subscription replicating, no slot lost (GB-348)
//!   6  converged  every copied table the same on both, by count and digest (GB-349)
//!
//! The node is reached at its mesh address and the founder at its site's first
//! host, as `inventory-hosts.py` reads them from the mesh inventory, each
//! through the Postgres container the replication role names
//! (`spock_mesh_postgres_container`). The helper journals each step in
//! `<state_dir>/join-node-<site>-<node>-reseed.json`; a run starts at the
//! first step the journal does not record as passed, so a stopped run resumes
//! where it stopped. A journal whose last step passed is kept beside it as
//! `...-reseed.<time>.done.json`, and the next re-seed starts at step 1.
//! `--dry-run` prints the six steps and runs nothing; `--from` and `--to`
//! narrow them.

use std::path::Path;

use anyhow::{Context, Result};

use crate::joinnode::Options;
use crate::runner::Cmd;
use crate::standup::{sh_quote, sh_word};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub const STEPS: [(&str, &str); 6] = [
    ("lost", "the founder's slot for the node is lost: there is something to seed again"),
    ("caught-up", "the founder has taken the node's own writes, so the truncate loses nothing"),
    ("size", "the copy is well under the node's own slot cap"),
    ("reseed", "drop and create each lost subscription with no copy, then copy each table again after a local truncate"),
    ("synced", "every table copied, the subscription replicating, no slot on the node lost"),
    ("converged", "every copied table the same on the node and the founder"),
];

/// The value of `key:` in a YAML file's top level, or "".
fn yaml_value(file: &Path, key: &str) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.strip_prefix(key)
                    .and_then(|r| r.strip_prefix(':'))
                    .map(|v| v.split_whitespace().next().unwrap_or("").replace(['"', '\''], ""))
            })
        })
        .unwrap_or_default()
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

pub fn journal_path(repo: &Path, site: &str, node: &str) -> String {
    format!("{}/join-node-{site}-{node}-reseed.json", crate::settings::state_dir_abs(repo))
}

/// Each step's outcome as the journal records it: (passed, code, line).
fn journaled(path: &str) -> std::collections::BTreeMap<String, (bool, String, String)> {
    let doc: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);
    let mut out = std::collections::BTreeMap::new();
    if let Some(steps) = doc["steps"].as_object() {
        for (k, v) in steps {
            out.insert(
                k.clone(),
                (v["ok"].as_bool().unwrap_or(false), v["code"].as_str().unwrap_or("").to_string(), v["line"].as_str().unwrap_or("").to_string()),
            );
        }
    }
    out
}

pub fn execute(repo: &Path, o: &Options) -> Result<Report> {
    let site_us = o.site.replace('-', "_");
    let group = format!("{site_us}_hosts");
    let gv = repo.join("ops/ansible/group_vars").join(&group).join("vars.yml");
    let founder_site = yaml_value(&gv, "join_founder_site");
    if founder_site.is_empty() {
        return Err(goat_errors::coded("GB-247", format!("--reseed at {}, which names no founder (join_founder_site in group_vars/{group}/vars.yml): a node is seeded again from the site that kept its slot, the founder its site joins", o.site)).into());
    }
    let founder_group = format!("{}_hosts", founder_site.replace('-', "_"));
    let mut container = yaml_value(&gv, "spock_mesh_postgres_container");
    if container.is_empty() {
        container = yaml_value(&repo.join("ops/ansible/roles/spock-mesh-peering/defaults/main.yml"), "spock_mesh_postgres_container");
    }
    if container.is_empty() {
        return Err(goat_errors::coded("GB-350", "--reseed: no spock_mesh_postgres_container in the site's group_vars or in ops/ansible/roles/spock-mesh-peering/defaults/main.yml, so the run cannot name the Postgres container it reaches").into());
    }
    let Some(user) = crate::settings::ssh_user(repo) else {
        return Err(goat_errors::coded("GB-251", "this checkout declares no SSH user: set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml; the re-seed reaches the node and the founder as that user").into());
    };
    let journal = journal_path(repo, o.site, o.node);
    let r = repo.display().to_string();
    let no_node = sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives {} no mesh address under [{group}]: a node is seeded again over the mesh, once it is enrolled", o.node));
    let no_site = sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives [{founder_group}] no host with a mesh address: the founder {founder_site} is reached over the mesh at its first host"));
    // Each address checked on its own: an empty or placeholder one stops the
    // step before the helper starts.
    let reach = format!(
        "cd '{r}/ops/ansible' && node=$(python3 ../scripts/inventory-hosts.py {group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {u} --host {n}) && site=$(python3 ../scripts/inventory-hosts.py {founder_group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {u} | head -n 1) || {{ echo {no_node}; exit 1; }}; case \"$node\" in ''|*@|*TBD_*) echo {no_node}; exit 1 ;; esac; case \"$site\" in ''|*@|*TBD_*) echo {no_site}; exit 1 ;; esac; ",
        u = sh_word(&user),
        n = sh_word(o.node),
    );
    let psql = |who: &str| format!("\"ssh ${who} sudo docker exec -i {} psql -U postgres\"", sh_word(&container));
    let cmd = |step: &str| format!("{reach}python3 ../scripts/spock-reseed.py {step} --node-psql {} --site-psql {} --journal {}", psql("node"), psql("site"), sh_quote(&journal));

    // A finished re-seed is kept, and this one starts again.
    let mut done = journaled(&journal);
    if !o.dry_run && done.get("converged").is_some_and(|d| d.0) {
        let kept = journal.trim_end_matches(".json").to_string() + &format!(".{}.done.json", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
        std::fs::rename(&journal, &kept).with_context(|| format!("cannot keep the finished journal {journal} as {kept}"))?;
        done.clear();
    }
    let first_open = STEPS.iter().position(|(s, _)| !done.get(*s).is_some_and(|d| d.0)).map_or(1, |i| i as u32 + 1);
    let from = o.from.max(first_open);

    let resume = format!("goatbuilder join-node {} --site {} --node {} --class {} --reseed", o.env, o.site, o.node, o.class);
    let mut checks = vec![row("banner", format!("join-node --reseed: site={}  node={}  founder={}  journal={journal}  {}", o.site, o.node, founder_site, if o.dry_run { "(DRY RUN — nothing executes)" } else { "" }))];
    if from > 1 && !o.dry_run {
        checks.push(row("resume", format!("steps 1 to {} passed in the journal; starting at step {from}", from - 1)));
    }
    let mut exit = EXIT_OK;
    for (i, (step, label)) in STEPS.iter().enumerate() {
        let n = i as u32 + 1;
        if n < from || n > o.to {
            continue;
        }
        let c = cmd(step);
        checks.push(row("step", format!("=== Step {n} {step}: {label} ===")));
        checks.push(row("cmd", c.clone()));
        if o.dry_run {
            continue;
        }
        crate::runner::phase(&n.to_string(), label, "");
        let status = Cmd::new("bash").args(["-euo", "pipefail", "-c", &c]).current_dir(repo).status().context("bash")?;
        if !status.success() {
            let (_, code, line) = journaled(&journal).get(*step).cloned().unwrap_or_default();
            let detail = if code.is_empty() {
                format!("!! step {n} ({step}) failed, exit {}: {}", status.code().unwrap_or(1), if line.is_empty() { "the node or the founder could not be reached; the output above says why".to_string() } else { line })
            } else {
                format!("{code}: {line}")
            };
            checks.push(Check { name: "failed".into(), ok: false, detail, fix: Some(format!("fix the cause, then run it again; it starts at step {n}: {resume}")) });
            exit = EXIT_REFUSED;
            break;
        }
    }
    let mut rep = Report::new("join-node", checks, EXIT_REFUSED);
    rep.exit = exit;
    rep.ok = exit == EXIT_OK;
    Ok(rep)
}
