//! `join-node <env> --site <site> --node <node> --class <core|phone> --reseed`:
//! rebuild a replicating node from its founder after a slot it needs was
//! invalidated (docs/operations/spock-site-rebuild.md; the procedure is
//! measured on postgres-spock containers in
//! docs/design/goatbuilder-hardware-measurement.md, "The rebuild of a site").
//!
//! Eight steps, each one call of `ops/scripts/spock-reseed.py`, which says
//! what it checks and refuses with its own catalog code:
//!
//!   1  lost       a slot the node needs is lost (GB-345; GB-390 when a provider is not reached)
//!   2  caught-up  the founder has taken the node's own writes, or its slot on the node is lost (GB-346)
//!   3  export     rows only the node holds are written to a file, accepted by digest (GB-388, GB-389, GB-390)
//!   4  size       the copy is well under the node's own slot cap (GB-347)
//!   5  reseed     lost subscriptions re-created both ways; one truncate, then each table copied again
//!   6  synced     every table copied, every subscription replicating, no slot lost (GB-348)
//!   7  replay     the exported rows written again here, or handed back, accepted by digest (GB-391)
//!   8  converged  every copied table the same on the node and the founder (GB-349)
//!
//! The node is reached at its mesh address, the founder at its site's first
//! host and each `--peer` at its site's first host, as `inventory-hosts.py`
//! reads them from the mesh inventory, each through the Postgres container the
//! replication role names (`spock_mesh_postgres_container`). The helper
//! journals each step in `<state_dir>/join-node-<site>-<node>-reseed.json`,
//! with the export and hand-back files beside it; a run starts at the first
//! step the journal does not record as passed, so a stopped run resumes where
//! it stopped. A journal whose last step passed is kept beside it as
//! `...-reseed.<time>.done.json`, and the next rebuild starts at step 1.
//! `--accept <digest>` is passed to every step; only export and replay read
//! it. `--dry-run` prints the eight steps and runs nothing; `--from` and `--to`
//! narrow them. `--slots` is the detector, which reads only.

use std::path::Path;

use anyhow::Result;

use crate::config;
use crate::joinnode::Options;
use crate::targets;
use crate::runner::Cmd;
use crate::standup::{sh_quote, sh_word};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

pub const STEPS: [(&str, &str); 8] = [
    ("lost", "a slot the node needs is lost: there is something to rebuild"),
    ("caught-up", "the founder has taken the node's own writes, or can no longer take them"),
    ("export", "the rows only the node holds are exported, and accepted by their digest"),
    ("size", "the copy is well under the node's own slot cap"),
    ("reseed", "re-create the lost subscriptions both ways, truncate once, copy each table again from the founder"),
    ("synced", "every table copied, every subscription replicating, no slot on the node lost"),
    ("replay", "the exported rows written again here, or handed back"),
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
                    .map(|v| {
                        v.split_whitespace()
                            .next()
                            .unwrap_or("")
                            .replace(['"', '\''], "")
                    })
            })
        })
        .unwrap_or_default()
}

fn row(name: &str, detail: String) -> Check {
    Check {
        name: name.into(),
        ok: true,
        detail,
        fix: None,
    }
}

pub fn journal_path(repo: &Path, site: &str, node: &str) -> String {
    format!(
        "{}/join-node-{site}-{node}-reseed.json",
        crate::settings::state_dir_abs(repo)
    )
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
                (
                    v["ok"].as_bool().unwrap_or(false),
                    v["code"].as_str().unwrap_or("").to_string(),
                    v["line"].as_str().unwrap_or("").to_string(),
                ),
            );
        }
    }
    out
}

/// How a run reaches the node, its founder and any peers: a bash prefix that
/// sets `$node`, `$site` and `$peer<i>` to their SSH targets from the mesh
/// inventory, stopping when one has no address, and the helper's arguments
/// that reach each one's Postgres through `docker exec`.
struct Reach {
    founder_site: String,
    prefix: String,
    sides: String,
}

/// `spock_mesh_postgres_container` for a site: its group_vars, else the
/// replication role's default.
fn container_of(repo: &Path, site: &str) -> String {
    let gv = repo
        .join("ops/ansible/group_vars")
        .join(format!("{}_hosts", site.replace('-', "_")))
        .join("vars.yml");
    let c = yaml_value(&gv, "spock_mesh_postgres_container");
    if !c.is_empty() {
        return c;
    }
    yaml_value(
        &repo.join("ops/ansible/roles/spock-mesh-peering/defaults/main.yml"),
        "spock_mesh_postgres_container",
    )
}

fn reach(repo: &Path, o: &Options, peers: &[String], flag: &str) -> Result<Reach> {
    let site_us = o.site.replace('-', "_");
    let group = format!("{site_us}_hosts");
    let gv = repo
        .join("ops/ansible/group_vars")
        .join(&group)
        .join("vars.yml");
    let founder_site = yaml_value(&gv, "join_founder_site");
    if founder_site.is_empty() {
        return Err(goat_errors::coded("GB-247", format!("{flag} at {}, which names no founder (join_founder_site in group_vars/{group}/vars.yml): a node is seeded again from the site that kept its slot, the founder its site joins", o.site)).into());
    }
    let founder_group = format!("{}_hosts", founder_site.replace('-', "_"));
    let mut names = vec![o.site.to_string(), founder_site.clone()];
    names.extend(peers.iter().cloned());
    let mut containers = Vec::new();
    for n in &names {
        let c = container_of(repo, n);
        if c.is_empty() {
            return Err(goat_errors::coded("GB-350", format!("{flag}: no spock_mesh_postgres_container in the group_vars of {n} or in ops/ansible/roles/spock-mesh-peering/defaults/main.yml, so the run cannot name the Postgres container it reaches")).into());
        }
        containers.push(c);
    }
    for p in peers {
        if p == o.site || *p == founder_site {
            return Err(goat_errors::coded("GB-247", format!("{flag}: --peer {p} is the node's own site or its founder; a peer is another site the node replicates with")).into());
        }
    }
    let Some(user) = crate::settings::ssh_user(repo) else {
        return Err(goat_errors::coded("GB-251", "this checkout declares no SSH user: set deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml; the re-seed reaches the node and the founder as that user").into());
    };
    let r = repo.display().to_string();
    let u = sh_word(&user);
    let no_node = sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives {} no mesh address under [{group}]: a node is seeded again over the mesh, once it is enrolled", o.node));
    let no_site = |s: &str, g: &str| {
        sh_quote(&format!("inventory/mgmt-hosts-mesh.ini gives [{g}] no host with a mesh address: {s} is reached over the mesh at its first host"))
    };
    // Each address checked on its own: an empty or placeholder one stops the
    // step before the helper starts.
    let mut prefix = format!(
        "cd '{r}/ops/ansible' && node=$(python3 ../scripts/inventory-hosts.py {group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {u} --host {n}) && site=$(python3 ../scripts/inventory-hosts.py {founder_group} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {u} | head -n 1) || {{ echo {no_node}; exit 1; }}; case \"$node\" in ''|*@|*TBD_*) echo {no_node}; exit 1 ;; esac; case \"$site\" in ''|*@|*TBD_*) echo {ns}; exit 1 ;; esac; ",
        n = sh_word(o.node),
        ns = no_site(&format!("the founder {founder_site}"), &founder_group),
    );
    let psql = |who: &str, container: &str| {
        format!(
            "\"ssh ${who} sudo docker exec -i {} psql -U postgres\"",
            sh_word(container)
        )
    };
    let mut sides = format!(
        "--node-psql {} --site-psql {}",
        psql("node", &containers[0]),
        psql("site", &containers[1])
    );
    for (i, p) in peers.iter().enumerate() {
        let g = format!("{}_hosts", p.replace('-', "_"));
        let ns = no_site(&format!("the peer {p}"), &g);
        prefix.push_str(&format!(
            "peer{i}=$(python3 ../scripts/inventory-hosts.py {gw} --inventory inventory/mgmt-hosts-mesh.ini --ssh-target --user {u} | head -n 1) || {{ echo {ns}; exit 1; }}; case \"$peer{i}\" in ''|*@|*TBD_*) echo {ns}; exit 1 ;; esac; ",
            gw = sh_word(&g)
        ));
        let pc = psql(&format!("peer{i}"), &containers[i + 2]);
        sides.push_str(&format!(" --peer-psql {}{}", sh_word(&format!("{p}=")), pc));
    }
    Ok(Reach {
        founder_site,
        prefix,
        sides,
    })
}

/// Only a node that replicates is a Spock member, and the environment, site
/// and class must be declared, as for `join-node` itself (joinnode::declared
/// makes the same checks).
fn member(repo: &Path, o: &Options, flag: &str, lacks: &str) -> Result<()> {
    let targets = targets::load(repo)?;
    if targets.get(o.class).is_some_and(|t| t.converges_as() != "core") {
        return Err(goat_errors::coded("GB-247", format!("{flag} is for a node that replicates (class core or phone); a {} node is not a Spock member and {lacks}", o.class)).into());
    }
    let cfg = config::load(&repo.join("ops/goatbuilder/goatbuilder.yaml"))?;
    let Some(env) = cfg.environments.iter().find(|e| e.name == o.env) else {
        return Err(goat_errors::coded("GB-241", format!("unknown environment \"{}\"; declared: {:?}", o.env, cfg.environments.iter().map(|e| e.name.as_str()).collect::<Vec<_>>())).into());
    };
    if !env.sites.is_empty() {
        let Some(site) = env.sites.iter().find(|s| s.name == o.site) else {
            return Err(goat_errors::coded("GB-247", format!("site '{}' is not declared for environment '{}' (declared: {})", o.site, o.env, env.sites.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))).into());
        };
        if !site.classes.is_empty() && !site.classes.contains_key(o.class) {
            return Err(goat_errors::coded("GB-247", format!("class '{}' is not declared at site '{}' (declared: {})", o.class, o.site, site.classes.keys().cloned().collect::<Vec<_>>().join(", "))).into());
        }
    }
    let t = targets.require(o.class)?;
    targets.check_family(repo, t, o.site, o.node)?;
    targets.declarations(repo, t, o.site, o.node).map(|_| ())
}

/// `join-node ... --slots`: the detector. Lists every Spock subscription and
/// slot of the node, with each slot's wal_status read where it lives, and
/// names the ones that cannot recover. Reads only; it needs no `--yes`.
pub fn slots(repo: &Path, o: &Options, peers: &[String]) -> Result<Report> {
    member(repo, o, "--slots", "has no slots")?;
    let rc = reach(repo, o, peers, "--slots")?;
    let c = format!(
        "{}python3 ../scripts/spock-reseed.py slots {} --json",
        rc.prefix, rc.sides
    );
    let mut checks = vec![
        row(
            "banner",
            format!(
                "join-node --slots: site={}  node={}  founder={}{}  {}",
                o.site,
                o.node,
                rc.founder_site,
                if peers.is_empty() {
                    String::new()
                } else {
                    format!("  peers={}", peers.join(","))
                },
                if o.dry_run {
                    "(DRY RUN — nothing executes)"
                } else {
                    ""
                }
            ),
        ),
        row("cmd", c.clone()),
    ];
    let mut rep_exit = EXIT_OK;
    if !o.dry_run {
        let out = Cmd::new("bash")
            .args(["-euo", "pipefail", "-c", &c])
            .current_dir(repo)
            .output()
            .map_err(|e| goat_errors::coded("GB-320", format!("could not start bash: {e}")))?;
        let doc: serde_json::Value =
            serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
        let Some(found) = doc["slots"].as_array() else {
            let err = String::from_utf8_lossy(&out.stderr);
            let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), err);
            checks.push(Check {
                name: "failed".into(),
                ok: false,
                detail: format!(
                    "!! the detector did not run, exit {}: {}",
                    out.status.code().unwrap_or(1),
                    said.trim()
                ),
                fix: Some(
                    "the node or a side could not be reached; the line above says why".into(),
                ),
            });
            let mut rep = Report::new("join-node", checks, EXIT_REFUSED);
            rep.exit = EXIT_REFUSED;
            rep.ok = false;
            return Ok(rep);
        };
        let mut lost = Vec::new();
        for f in found {
            let s = |k: &str| f[k].as_str().unwrap_or("").to_string();
            let line = format!(
                "{} {} {}  {} -> {}  slot {} at {}  status {}  wal_status {}: {}",
                s("db"),
                s("direction"),
                if s("subscription").is_empty() {
                    "(subscription not read)".into()
                } else {
                    s("subscription")
                },
                s("provider"),
                if s("subscriber").is_empty() {
                    "?".into()
                } else {
                    s("subscriber")
                },
                s("slot"),
                s("slot_at"),
                s("status"),
                s("wal_status"),
                s("verdict")
            );
            let ok = s("wal_status") != "lost";
            if !ok {
                lost.push(format!("{} {} (at {})", s("db"), s("slot"), s("slot_at")));
            }
            checks.push(Check {
                name: "slot".into(),
                ok,
                detail: line,
                fix: None,
            });
        }
        if !lost.is_empty() {
            checks.push(Check {
                name: "failed".into(),
                ok: false,
                detail: format!("GB-387: {} Spock slot(s) read wal_status = lost and can never be read again: {}", lost.len(), lost.join(", ")),
                fix: Some(format!(
                    "rebuild the node: goatbuilder join-node {} --site {} --node {} --class {} --reseed --dry-run (docs/operations/post-partition-convergence-runbook.md, A)",
                    o.env, o.site, o.node, o.class
                )),
            });
            rep_exit = EXIT_REFUSED;
        }
    }
    let mut rep = Report::new("join-node", checks, EXIT_REFUSED);
    rep.exit = rep_exit;
    rep.ok = rep_exit == EXIT_OK;
    Ok(rep)
}

/// `join-node ... --reseed` for a site that replicates with its founder alone
/// (no `--peer`, no `--accept`), as joinnode::reseed calls it.
pub fn execute(repo: &Path, o: &Options) -> Result<Report> {
    rebuild(repo, o, &[], None)
}

/// `join-node ... --reseed`: the rebuild, one helper step at a time (STEPS),
/// with each site the node replicates with (`--peer`) and the digest the
/// operator accepted (`--accept`).
pub fn rebuild(repo: &Path, o: &Options, peers: &[String], accept: Option<&str>) -> Result<Report> {
    member(repo, o, "--reseed", "has no slot to lose")?;
    let rc = reach(repo, o, peers, "--reseed")?;
    let founder_site = rc.founder_site.clone();
    let journal = journal_path(repo, o.site, o.node);
    let cmd = |step: &str| {
        format!(
            "{}python3 ../scripts/spock-reseed.py {step} {} --journal {}{}",
            rc.prefix,
            rc.sides,
            sh_quote(&journal),
            accept.map_or(String::new(), |d| format!(" --accept {}", sh_quote(d)))
        )
    };

    // A finished re-seed is kept, and this one starts again.
    let mut done = journaled(&journal);
    if !o.dry_run && done.get("converged").is_some_and(|d| d.0) {
        let kept = journal.trim_end_matches(".json").to_string()
            + &format!(".{}.done.json", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
        std::fs::rename(&journal, &kept).map_err(|e| {
            goat_errors::coded(
                "GB-323",
                format!("cannot keep the finished re-seed journal {journal} as {kept}: {e}"),
            )
        })?;
        done.clear();
    }
    let first_open = STEPS
        .iter()
        .position(|(s, _)| !done.get(*s).is_some_and(|d| d.0))
        .map_or(1, |i| i as u32 + 1);
    let from = o.from.max(first_open);

    let resume = format!(
        "goatbuilder join-node {} --site {} --node {} --class {} --reseed",
        o.env, o.site, o.node, o.class
    );
    let mut checks = vec![row(
        "banner",
        format!(
            "join-node --reseed: site={}  node={}  founder={}  journal={journal}  {}",
            o.site,
            o.node,
            founder_site,
            if o.dry_run {
                "(DRY RUN — nothing executes)"
            } else {
                ""
            }
        ),
    )];
    if from > 1 && !o.dry_run {
        checks.push(row(
            "resume",
            format!(
                "steps 1 to {} passed in the journal; starting at step {from}",
                from - 1
            ),
        ));
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
        let status = Cmd::new("bash")
            .args(["-euo", "pipefail", "-c", &c])
            .current_dir(repo)
            .status()
            .map_err(|e| goat_errors::coded("GB-320", format!("could not start bash: {e}")))?;
        if !status.success() {
            let (_, code, line) = journaled(&journal).get(*step).cloned().unwrap_or_default();
            let detail = if code.is_empty() {
                format!(
                    "!! step {n} ({step}) failed, exit {}: {}",
                    status.code().unwrap_or(1),
                    if line.is_empty() {
                        "the node or the founder could not be reached; the output above says why"
                            .to_string()
                    } else {
                        line
                    }
                )
            } else {
                format!("{code}: {line}")
            };
            checks.push(Check {
                name: "failed".into(),
                ok: false,
                detail,
                fix: Some(format!(
                    "fix the cause, then run it again; it starts at step {n}: {resume}"
                )),
            });
            exit = EXIT_REFUSED;
            break;
        }
    }
    let mut rep = Report::new("join-node", checks, EXIT_REFUSED);
    rep.exit = exit;
    rep.ok = exit == EXIT_OK;
    Ok(rep)
}
