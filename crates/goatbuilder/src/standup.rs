//! `standup`: a site from nothing to verified — the verb that replaces
//! ops/scripts/goat-standup.sh behind the parity gate, and the one-hour
//! goatnet's measuring stick (goatbuilder.md §"Standing up a goatnet in an hour").
//!
//! The real path is the script's phases, in its order, with its words: the
//! fail-closed gates in phase 1, the host baseline, secrets and certs, the
//! mgmt stack in two passes around Ory, the internet-only relay and TLS
//! phases, the two operator-fired pauses (8, 11), the declared ACL first in
//! phase 12, the service surface, the acceptance gate, and the rescue classes
//! plus the cross-goatnet surfaces in phase 14 (ADR 1183). Phases are 1..14,
//! whole numbers, no gaps. A site that declares
//! an observability host gets it built in PARALLEL legs that join before the
//! phase that needs them (P11). `--dry-run` prints every phase and command and
//! runs nothing; `--from`/`--to` resume; every phase is idempotent.
//!
//! On a fixture (`--mock`) the scripted converger stands in for the hosts: every
//! component declared for the site is published class by class in the
//! doctrine's phase order, observed until its record names the ref, and the
//! time from declared to verified is reported — the number the one-hour target
//! is measured against once the same shape runs on hardware.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::config;
use crate::converger::Converger;
use crate::verbs::Fixture;
use crate::{Check, Report, EXIT_NOT_CONVERGED, EXIT_OK, EXIT_REFUSED};

pub struct Options<'a> {
    pub env: &'a str,
    pub site: &'a str,
    pub profile: Option<&'a str>,
    pub from: u32,
    pub to: u32,
    pub skip_obs: bool,
    pub obs_only: bool,
    pub dry_run: bool,
    pub parallel: bool,
}

/// One phase, as the script's `step <n> <kind> <label> <command>`.
#[derive(Debug, Clone)]
struct Phase {
    n: u32,
    kind: &'static str,
    label: String,
    cmd: String,
    /// The one phase whose success ends the run so the operator can reload
    /// the environment it just wrote (the Forgejo bot token).
    stop_after: bool,
}

/// The plan in the script's order: phases, the background legs and their
/// joins, and the rescue-class skips that print outside the phase gate.
#[derive(Debug, Clone)]
enum Event {
    Phase(Phase),
    LegStart { leg: &'static str, label: String, cmd: String, log: String },
    LegJoin { leg: &'static str, at: u32 },
    /// Every pinned image pulled on the site's hosts in the background
    /// (standup-image-pull.yml), started after phase 2 and joined before phase 5.
    /// Its banners are `phase` rows, so the table prints them as section titles.
    PullStart { label: String, cmd: String, log: String },
    PullJoin { at: u32 },
    Skip { label: String, lines: Vec<String> },
}

/// What the script reads from the site's group_vars before any phase, and the
/// derived names every command string carries.
struct Site {
    repo: String,
    site: String,
    group: String,
    upper: String,
    obs_group: String,
    state_dir: String,
    ansible_pre: String,
    inv: &'static str,
    profile: String,
    joiner: bool,
    join_founder_site: String,
    join_founder_group: String,
    acl_site: String,
    mgmt_fqdn: String,
    intra_pair: bool,
    obs: bool,
    obs_why: String,
    group_vars_exists: bool,
    /// Who the bootstrap belongs to: a goatlite's declared `lite.admin`, else
    /// `$OWNER_EMAIL` for the phase's own shell to expand as it always has.
    owner_email: String,
    /// A goatlite run (ADR 1213): `<repo>/goatlite.yaml` is `preset: lite` and
    /// names this site. Phase 1 renders the config surface first, phase 4 mints
    /// the CA when the manifest's root is missing, the profile is LAN, no obs host.
    lite: bool,
    /// `<UPPER>_NB_API_TOKEN` is available under the declared secrets posture,
    /// so a LAN site needs no phase-11 first-login step.
    nb_token_available: bool,
}

const INV: &str = "-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini";

/// The site a standup runs for: `--site` when given; otherwise, on a goatlite
/// run, the lite goatnet's own name — from the `--mock` fixture's config when
/// there is one, else `<repo>/goatlite.yaml` — when its `lite.name` or derived
/// environment name is `env`. Anything else still needs `--site`, in the words
/// clap would have used.
pub fn site_for(repo: &Path, fixture: Option<&config::Config>, env: &str, site: Option<&str>) -> Result<String> {
    if let Some(s) = site {
        return Ok(s.to_string());
    }
    let lite_name = |c: &config::Config| {
        c.preset.as_deref() == Some("lite")
            && c.lite.as_ref().map(|l| l.name == env || l.env_name() == env).unwrap_or(false)
    };
    if let Some(c) = fixture.filter(|c| lite_name(c)) {
        return Ok(c.lite.as_ref().expect("checked").name.clone());
    }
    let root = repo.join("goatlite.yaml");
    if fixture.is_none() && root.is_file() {
        // A file that does not load is the cause; a missing --site would be a wrong answer.
        let c = config::load(&root).map_err(|e| anyhow!("{e:#}\n\n(the site is derived from goatlite.yaml when --site is left out, so fix that file or pass --site)"))?;
        if lite_name(&c) {
            return Ok(c.lite.expect("checked").name);
        }
    }
    Err(anyhow!("the following required arguments were not provided:\n  --site <SITE>\n\n(--site may be omitted only on a goatlite run: {} with preset lite and lite.name {env})", if fixture.is_some() { "the fixture's goatbuilder.yaml" } else { "goatlite.yaml at the checkout root" }))
}

/// The goatlite config for a run (ADR 1213): `<repo>/goatlite.yaml`, when it
/// exists, is `preset: lite` and its `lite.name` is this site. Read with the
/// same loader as any config, so a file that does not validate refuses the run
/// rather than being skipped.
pub fn resolve_lite(repo: &Path, site: &str) -> Result<Option<config::Config>> {
    let path = repo.join("goatlite.yaml");
    if !path.is_file() {
        return Ok(None);
    }
    // A goatlite.yaml that is not lite, or names another goatnet, is not this
    // run's: it is left alone, whatever its state. One that names this site is
    // validated in full, and a malformed one refuses the run.
    let raw = config::load_value(&path)?;
    let names_this_site = raw.get("preset").and_then(|p| p.as_str()) == Some("lite")
        && raw.get("lite").and_then(|l| l.get("name")).and_then(|n| n.as_str()) == Some(site);
    if !names_this_site {
        return Ok(None);
    }
    Ok(Some(config::load(&path)?))
}

/// `grep -qE '^[[:space:]]*<key>:[[:space:]]*<value>'` over a file.
fn gv_matches(file: &Path, key: &str, value: &str) -> bool {
    std::fs::read_to_string(file)
        .map(|t| t.lines().any(|l| gv_line_value(l, key).map(|v| v.starts_with(value)).unwrap_or(false)))
        .unwrap_or(false)
}

/// The first `<key>:` line's second word, quotes stripped — the script's
/// `grep | head -1 | awk '{print $2}' | tr -d '"\''`.
fn gv_first(file: &Path, key: &str) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|t| t.lines().find_map(|l| gv_line_value(l, key).map(|v| v.split_whitespace().next().unwrap_or("").replace(['"', '\''], ""))))
        .unwrap_or_default()
}

fn gv_line_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let t = line.trim_start();
    let rest = t.strip_prefix(key)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.trim_start())
}

/// `gv_value` in the script: one group_vars value read as YAML, folded
/// scalars included, whitespace collapsed; None when the key is absent.
fn gv_value(file: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let doc: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let v = doc.get(key)?;
    let s = match v {
        serde_yaml::Value::Bool(b) => if *b { "True".to_string() } else { "False".to_string() },
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => return None,
        other => serde_yaml::to_string(other).unwrap_or_default(),
    };
    Some(s.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `python3 ops/scripts/inventory-hosts.py <groups...> [--ssh-target]`, one line per host.
fn inventory_hosts(repo: &Path, groups: &[&str], ssh_target: bool) -> Vec<String> {
    let mut c = Command::new("python3");
    c.arg(repo.join("ops/scripts/inventory-hosts.py")).args(groups);
    if ssh_target {
        c.arg("--ssh-target")
            .arg("--inventory").arg(repo.join("ops/ansible/inventory/mgmt-hosts.ini"))
            .arg("--inventory").arg(repo.join("ops/ansible/inventory/mgmt-hosts-mesh.ini"));
    }
    c.output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Why the obs leg cannot run in this tree, when its utility play is absent.
pub(crate) fn obs_play_missing(repo: &Path) -> Option<String> {
    if repo.join("ops/ansible/utility-host-bootstrap.yml").is_file() {
        return None;
    }
    Some("has_obs_host: true, but ops/ansible/utility-host-bootstrap.yml is not in this tree, so the obs leg cannot run.".to_string())
}

fn read_site(repo: &Path, o: &Options) -> Result<Site> {
    let site = o.site.to_string();
    let site_us = site.replace('-', "_");
    let group = format!("{site_us}_hosts");
    let obs_group = format!("{site_us}_obs_hosts");
    let upper = site_us.to_uppercase();
    let aws = crate::settings::aws_profile(repo);
    let gv = repo.join("ops/ansible/group_vars").join(&group).join("vars.yml");
    let lite_cfg = resolve_lite(repo, &site)?;
    let lite = lite_cfg.is_some();
    // A goatlite declares its administrator (lite.admin), so the run uses that
    // rather than an OWNER_EMAIL the person must remember to export: phase 5
    // expanded "$OWNER_EMAIL" in its arguments and died before Ansible, and
    // phase 11 named a file on the host after it.
    let owner_email = lite_cfg.as_ref().and_then(|c| c.lite.as_ref()).map(|l| l.admin.clone())
        .unwrap_or_else(|| "$OWNER_EMAIL".to_string());

    let (obs, obs_why) = if o.skip_obs {
        (false, "--skip-obs".to_string())
    } else if lite {
        (false, "goatlite: a lite site has no observability host (ADR 1213)".to_string())
    } else if !gv_matches(&gv, "has_obs_host", "true") {
        (false, format!("has_obs_host is not true in group_vars/{group}/vars.yml"))
    } else if !repo.join("ops/ansible/group_vars").join(&obs_group).is_dir() {
        (false, format!("has_obs_host: true but no config surface at group_vars/{obs_group}/ (scaffold: standup-config-preflight.py {obs_group} --scaffold-plan)"))
    } else if inventory_hosts(repo, &[&obs_group], false).is_empty() {
        (false, format!("has_obs_host: true but no host in inventory group [{obs_group}] yet"))
    } else {
        (true, String::new())
    };
    if o.obs_only && !obs {
        return Err(anyhow!("--obs-only: no obs leg to run — {obs_why}"));
    }
    // Refuse here rather than at phase 2, after the core leg has started beside it.
    if obs {
        if let Some(missing) = obs_play_missing(repo) {
            return Err(anyhow!("{missing} Re-run with --skip-obs to stand up the site without its obs host."));
        }
    }

    let profile = match o.profile {
        Some(p) => p.to_string(),
        None if lite => "lan".to_string(),
        None => if gv_matches(&gv, "dashboard_cert_delivery_mode", "acme") { "internet".to_string() } else { "lan".to_string() },
    };
    if profile != "lan" && profile != "internet" {
        return Err(anyhow!("bad --profile '{profile}' (want lan|internet)"));
    }
    // A LAN site pauses for the first login (phase 11) only while the bootstrap
    // NetBird token is not available under the declared posture (ADR 1204).
    let nb_token_available = profile == "lan" && crate::secrets::available(repo).contains(&format!("{upper}_NB_API_TOKEN"));
    let joiner = gv_matches(&gv, "goatnet_founder", "false");
    let join_founder_site = if joiner { gv_first(&gv, "join_founder_site") } else { String::new() };
    let join_founder_group = if join_founder_site.is_empty() { String::new() } else { format!("{}_hosts", join_founder_site.replace('-', "_")) };
    let acl_site = if joiner { join_founder_site.clone() } else { site.clone() };
    Ok(Site {
        repo: repo.display().to_string(),
        group_vars_exists: repo.join("ops/ansible/group_vars").join(&group).is_dir(),
        owner_email,
        site,
        group,
        upper,
        obs_group,
        state_dir: crate::settings::state_dir_abs(repo),
        // A goatlite reaches its one box directly at its inventory address and
        // declares no cloud, so no play needs AWS. with-aws defaults to
        // aws-vault whenever aws-vault is installed (ADR 1204), so without this
        // every play asked the operator's keychain, once per play.
        ansible_pre: format!("OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES {}../scripts/with-aws {aws} -- ../scripts/ansible-preflight.sh",
                             if lite { "GOAT_AWS_MODE=none " } else { "" }),
        inv: INV,
        profile,
        joiner,
        join_founder_site,
        join_founder_group,
        acl_site,
        mgmt_fqdn: gv_first(&gv, "public_mgmt_fqdn"),
        intra_pair: gv_matches(&gv, "intra_site_pair", "true"),
        obs,
        obs_why,
        lite,
        nb_token_available,
    })
}

fn or_q(s: &str) -> &str {
    if s.is_empty() { "?" } else { s }
}

/// The plan, in the script's order. `resume` is how the operator re-enters
/// this verb (printed in the manual steps' instructions).
/// The playbook a phase runs for `site`: the site's own `<site>-<suffix>` when
/// it keeps one under ops/ansible, else the generic `site-<suffix>`.
fn site_play(repo: &Path, site: &str, suffix: &str) -> String {
    let own = format!("{site}-{suffix}");
    if repo.join("ops/ansible").join(&own).is_file() {
        own
    } else {
        format!("site-{suffix}")
    }
}

fn plan(repo: &Path, s: &Site, o: &Options, resume: &str) -> Vec<Event> {
    let r = &s.repo;
    let g = &s.group;
    let site = &s.site;
    let u = &s.upper;
    let oe = &s.owner_email;
    let pre = &s.ansible_pre;
    let inv = s.inv;
    let og = &s.obs_group;
    let p = |n: u32, kind: &'static str, label: String, cmd: String| Event::Phase(Phase { n, kind, label, cmd, stop_after: false });

    let leg_o1 = format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={og} -e site_id={site} && {pre} {inv} tang-server.yml -e target={og} && {pre} {inv} mgmt-host-luks.yml -e target={og} -e site_id={site}");
    let leg_o2 = format!("cd '{r}/ops/ansible' && {pre} {inv} utility-host-bootstrap.yml -e target={og}");
    let leg_o3 = format!("cd '{r}/ops/ansible' && {pre} {inv} obs-host-bootstrap.yml -e target={og}");

    let mut ev = Vec::new();
    if o.obs_only {
        ev.push(p(2, "A", "[obs] observability host: baseline + LUKS".into(), leg_o1));
        ev.push(p(2, "A", "[obs] observability host: utility (Forgejo)".into(), leg_o2));
        ev.push(p(9, "A", "[obs] observability host: stack (Prometheus/Alertmanager/Grafana)".into(), leg_o3));
        return ev;
    }

    let obs_arg = if s.obs { format!("--obs {og}") } else { String::new() };
    if s.lite {
        // goatlite (ADR 1213 D3): the preset derives the file set the engines read
        // — inventory group, group_vars, cert manifest, pins row, door entry — into
        // the checkout FIRST, then the unchanged phases run over it.
        ev.push(p(1, "GATE", "goatlite: render the config surface from goatlite.yaml (idempotent; an edited file is reported, never overwritten)".into(), format!("python3 '{r}/ops/scripts/goatlite-render.py' --site {site}")));
        // Then the env-file store every later phase reads, created here rather than by
        // hand: it needs the rendered groups, and the phases after it load it (LP-007).
        // standup-phase-run runs these two lines without loading the store.
        ev.push(p(1, "GATE", "goatlite: create the local secrets store (strong random values; a value already there is kept)".into(), format!("python3 '{r}/ops/secrets/posture.py' init --if-env-file --env {site}")));
    }
    ev.push(p(1, "GATE", "config preflight (fail-closed)".into(), format!("python3 '{r}/ops/scripts/standup-config-preflight.py' {g} {obs_arg}")));
    // Up to the manual step only: phase 11 issues the service token phase 12
    // reads, so scanning all fourteen phases here refused every first run.
    ev.push(p(1, "GATE", "every input the phases before the manual step will reach is set (one static scan of the plays; changes nothing)".into(), format!("python3 '{r}/ops/scripts/standup-required-inputs.py' {site} --profile {} --to 11", s.profile)));
    ev.push(p(1, "GATE", format!("the NetBird ACL for {} is declared, with no Default All->All (ADR 1185)", or_q(&s.acl_site)), format!("python3 '{r}/ops/scripts/netbird-policy-posture.py' '{}' --declared", s.acl_site)));
    ev.push(p(1, "GATE", "establish the env-door (off-profile transport)".into(), format!("python3 '{r}/ops/scripts/generate-door-inventory.py' {site}")));
    let obs_grp = if s.obs { og.as_str() } else { "" };
    // What the preflight is told about this run: phase 2 still ahead (its sysctls,
    // presync and container engine are then "set by phase 2", not failures), and
    // the site's own disk-encryption declaration (goatlite's VM shape declares
    // luks_volumes_mgmt_host: [] — an unencrypted root is then the declared
    // exclusion). goat-standup.sh computes the same two from the same facts.
    let pending = if o.from <= 2 { " --baseline-pending" } else { "" };
    ev.push(p(1, "GATE", "host preflight (measure the targets; changes nothing)".into(), format!(
        "hosts=$(python3 '{r}/ops/scripts/inventory-hosts.py' {g} {obs_grp});    enc=$(grep -qE '^luks_volumes_mgmt_host:[[:space:]]*\\[\\]' '{r}/ops/ansible/group_vars/{g}/vars.yml' 2>/dev/null && echo '--disk-encryption declared-off');    if [ -z \"$hosts\" ]; then echo '    no hosts in inventory for {g} yet — nothing to measure (normal before phase 1)';    else python3 '{r}/ops/scripts/standup-host-preflight.py' $hosts $enc{pending}         --inventory '{r}/ops/ansible/inventory/mgmt-hosts.ini'         --inventory '{r}/ops/ansible/inventory/mgmt-hosts-mesh.ini'; fi")));
    ev.push(p(2, "A", "host baseline + LUKS (this site's nodes only)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} mgmt-host-baseline.yml -e target={g} -e site_id={site} && {pre} {inv} tang-server.yml -e target={g} && {pre} {inv} mgmt-host-luks.yml -e target={g} -e site_id={site}")));
    if s.obs {
        ev.push(Event::LegStart { leg: "host", label: "observability host: baseline + LUKS, then utility (Forgejo)".into(), cmd: format!("{leg_o1} && {leg_o2}"), log: format!("{}/standup-{site}-obs-host.log", s.state_dir) });
    }
    // Every pinned image, pulled on the box in parallel with phases 3 and 4 and
    // joined before phase 5 (goatlite.md §11 measure 2). Only when phase 5 is in
    // range: a resume past it pulls inside its plays, as before.
    let pull = o.from <= 5 && o.to >= 5;
    if pull {
        ev.push(Event::PullStart {
            label: format!("every pinned image for {g}, pulled on its hosts in parallel (joined before phase 5)"),
            cmd: format!("cd '{r}/ops/ansible' && {pre} {inv} standup-image-pull.yml -e target={g}"),
            log: format!("{}/standup-{site}-images.log", s.state_dir),
        });
    }
    ev.push(p(3, "T", "generate per-instance secrets".into(), format!("'{r}/ops/scripts/newsite-secrets.sh' {site}")));
    // goatlite (goatlite.md §5 row 5): no CA root is handed out of band; phase 4
    // mints one when the manifest's root does not exist, then the leaves.
    let mint_ca = if s.lite { " --mint-ca-if-missing" } else { "" };
    ev.push(p(4, "T", "mint leaf certs".into(), format!("'{r}/ops/scripts/mint-site-certs.sh' {site}{mint_ca}")));
    let state = crate::settings::state_dir(repo);
    ev.push(p(4, "T", "name-fallback snapshot (generate + sign)".into(), format!(
        "if grep -qE 'name_snapshot_enabled:[[:space:]]*true' '{r}/ops/ansible/group_vars/{g}/vars.yml' 2>/dev/null; then      python3 '{r}/ops/dns/generate-mesh-hosts.py';      if [ -f {state}/{site}-ca/ca.key.age ]; then        ( cd '{r}/ops/enrollment' && go run ./cmd/name-snapshot-sign --snapshot '{r}/ops/dns/{site}/name-snapshot.json' --root-key {state}/{site}-ca/ca.key.age --age-identity ~/.config/age/keys.txt );      else echo 'name_snapshot: no {state}/{site}-ca/ca.key.age — generate ran, SIGN manually with ops/enrollment/cmd/name-snapshot-sign'; fi;    else echo 'name_snapshot_enabled not true for {g} — skipping (ADR 1082)'; fi")));

    let ev_bootstrap = format!("-e site_id={site} -e bootstrap_owner_email=\"{oe}\" -e postgres_superuser_password=\"$(printenv {u}_PG_PASS)\" -e netbird_encryption_key=\"$(printenv {u}_NB_ENCKEY)\" -e pgbackrest_cipher_pass=\"$(printenv {u}_BKP_KEY)\"");
    let ev_p1 = if s.joiner {
        if s.intra_pair { format!("{ev_bootstrap} -e participates_in_spock=false") } else { format!("{ev_bootstrap} -e participates_in_spock=false -e has_durable_independent_state=true") }
    } else {
        ev_bootstrap.clone()
    };
    if pull {
        ev.push(Event::PullJoin { at: 5 });
    }
    // Phase 5 runs site-bootstrap's `stack` plays only. The account seed waits
    // for the netbird schema, which management-a writes only once it can reach
    // Hydra, so the seed and every play after it run in phase 6, after
    // ory-deploy. Any refusal here is a real one and fails the phase.
    ev.push(p(5, "A", "mgmt-stack (site-bootstrap up to the account seed; the seed needs Ory and runs in phase 6)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} site-bootstrap.yml {ev_p1} --skip-tags after-ory")));
    let internet = s.profile == "internet";
    if internet {
        // site-rathole-client.yml is the generic play; a site that keeps its
        // own <site>-rathole-client.yml gets that one instead.
        let rathole_play = site_play(repo, site, "rathole-client.yml");
        ev.push(p(5, "A", format!("[internet] relay-connect (rathole-client, {rathole_play})"), format!("cd '{r}/ops/ansible' && {pre} -i inventory/hosts-{site}.ini {inv} {rathole_play} -e rathole_client_target={g}")));
    }
    ev.push(p(6, "A", "ory (kratos/hydra/ui)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} ory-deploy.yml --limit {g} -e postgres_superuser_password=\"$(printenv {u}_PG_PASS)\"")));
    let jfs = &s.join_founder_site;
    let jfg = &s.join_founder_group;
    if s.joiner {
        ev.push(p(6, "A", format!("[joiner] seed-from-peer (pass 2 — data-only copy from founder {}; guarded)", or_q(jfs)), format!("cd '{r}/ops/ansible' && {{ [ -n '{jfs}' ] || {{ echo 'joiner: group_vars/{g}/vars.yml must set join_founder_site (the founder site-id to seed from)'; exit 1; }}; }} && {{ ls {site}-seed-from-peer.yml >/dev/null 2>&1 || {{ echo 'missing {site}-seed-from-peer.yml — clone it from any site that has <site>-seed-from-peer.yml (node-addition.md §4)'; exit 1; }}; }} && {pre} {inv} {site}-seed-from-peer.yml -e site_id={site}")));
    }
    // The rest of site-bootstrap, now that Ory is up: the seed's schema wait is
    // bounded (netbird_schema_wait_seconds) and names its cause. A joiner runs
    // the whole file again: its pass 3 turns Spock on, which the stack plays set.
    if s.joiner {
        ev.push(p(6, "A", "[joiner] re-run site-bootstrap with Spock on (pass 3; prints + saves the setup key)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} site-bootstrap.yml {ev_bootstrap}")));
    } else {
        ev.push(p(6, "A", "the rest of site-bootstrap: wait for the netbird schema, seed the account, print + save the setup key".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} site-bootstrap.yml {ev_bootstrap} --skip-tags stack")));
    }
    if s.joiner {
        ev.push(p(6, "A", format!("[joiner] founder-side mirror (spock-mesh-wire on {}, peer={site} only — both-direction subscriptions)", or_q(jfg)), format!("cd '{r}/ops/ansible' && {{ [ -n '{jfg}' ] || {{ echo 'joiner: join_founder_site not set in group_vars/{g}/vars.yml'; exit 1; }}; }} && {pre} {inv} spock-mesh-wire.yml --limit {jfg} -e '{{\"inter_site_spock_peers\":[\"{site}\"]}}'")));
    }
    if internet {
        ev.push(p(7, "A", "[internet] public TLS certificate (traefik-acme)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} traefik-acme.yml -e target={g} -e cf_api_token=\"$(printenv CLOUDFLARE_API_TOKEN_{u})\"")));
        ev.push(p(7, "T", "[internet] generate relay inventory from terraform".into(), format!("'{r}/ops/ansible/inventory/generate-relays.sh' {site} terraform-{site}")));
        ev.push(p(7, "A", "[internet] relay tier (coturn + rathole + TLS renewal)".into(), format!("cd '{r}/ops/ansible' && umask 077 && VF=$(mktemp -t goat-relay-vars.XXXXXX) && printf 'nb_relay_auth_secret: \"%s\"\\nrelay_secret: \"%s\"\\ncloudflare_api_token: \"%s\"\\n' \"$(printenv {u}_RELAY_SECRET)\" \"$(printenv {u}_RELAY_SECRET)\" \"$(printenv CLOUDFLARE_API_TOKEN_{u})\" > \"$VF\" && {pre} -i inventory/hosts-{site}.ini relay-server.yml -e @\"$VF\"; rc=$?; rm -f \"$VF\"; exit $rc")));
        if s.joiner {
            ev.push(p(7, "A", format!("[joiner] wire {site} into the founder ({}) relay ingress + re-render", or_q(jfs)), format!("cd '{r}/ops/ansible' && {{ [ -n '{jfs}' ] || {{ echo 'joiner: join_founder_site not set in group_vars/{g}/vars.yml'; exit 1; }}; }} && {{ ls inventory/hosts-{jfs}.ini >/dev/null 2>&1 || {{ echo 'founder relay inventory inventory/hosts-{jfs}.ini missing — generate it first: inventory/generate-relays.sh {jfs} terraform-{jfs}'; exit 1; }}; }} && {pre} -i inventory/hosts-{jfs}.ini -i inventory/{jfs}-goat-relay-alias.ini relay-ingress-add-site.yml -e target=goat_relay_hosts -e founder_site={jfs} -e join_site={site}")));
        }
        ev.push(p(8, "MANUAL", "[internet] mesh-enroll the host + re-mint the router cert".into(), format!("see docs/operations/goatnet-standup.md §'Phase 8' for the exact block; run ops/preflight-enrollment.sh first (it PRINTS the netbird up line, rosenpass flags included); confirm 'netbird status' shows Management: Connected + a 100.x IP BEFORE re-minting; then fill the mesh IP into inventory + the cert manifest and re-mint: mint-site-certs.sh {site} --only router (delete the phase-4 router pair first, or the mint is a no-op)")));
    }
    ev.push(p(9, "A", "zenoh router".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} zenoh-deploy.yml -e target={g}")));
    ev.push(p(9, "A", "SPIRE server on the founder (workload identity)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} spire-deploy.yml -e target={g} -l 'mgmt-{site}-01*' -e spire_enabled=true -e postgres_superuser_password=\"$(printenv {u}_PG_PASS)\"")));
    if s.obs {
        ev.push(Event::LegStart { leg: "stack", label: "observability host: stack (Prometheus/Alertmanager/Grafana) — needs obs_scrape_targets".into(), cmd: leg_o3, log: format!("{}/standup-{site}-obs-stack.log", s.state_dir) });
    }
    if !internet {
        ev.push(p(10, "A", "[lan] LAN service surface (dashboard + DNS + TLS routes + STUN)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} -l 'mgmt-{site}-01*' site-services.yml --tags lan-dns,traefik,stun -e target={g} -e site_id={site} -e postgres_superuser_password=\"$(printenv {u}_PG_PASS)\" -e netbird_encryption_key=\"$(printenv {u}_NB_ENCKEY)\"")));
        ev.push(p(10, "A", "[lan] TLS routes for the mgmt stack (file certs)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} -l 'mgmt-{site}-01*' lan-traefik-routes.yml -e target={g} -e site_id={site}")));
        ev.push(p(10, "A", "[lan] dashboard".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} -l 'mgmt-{site}-01*' dashboard-deploy.yml -e site_id={site}")));
        // Phase 11 on a LAN site (goatlite.md §5 row 6): the same first login and
        // bootstrap token the internet branch pauses for, printed only while the
        // token is not yet available under the declared posture. The driver-made
        // identity and token (phase B) remove this step; until then it is the
        // exact dashboard route with the one-time password.
        if !s.nb_token_available {
            ev.push(p(11, "MANUAL", "[lan] first login, admin-promote, the bootstrap NetBird token".into(), format!("handbook §'Phase 11' has the exact block; on a LAN site the dashboard is the dashboard.* name under this site's zone (site_dns_zone in group_vars/{g}/vars.yml), resolved by the host's LAN CoreDNS: point the laptop's DNS at the host or pin the names in /etc/hosts. Read the one-time password from /opt/mgmt-host/.bootstrap-passwords/{oe}.txt on the host, log in; re-run the phase-10 LAN service line ONCE (promotes you to admin — the UI 403s until then); create service user svc-{site}-netbird-infra (Team > Service Users, role admin), create its token, and store it as {u}_NB_API_TOKEN under the declared secrets posture (posture.py hint {u}_NB_API_TOKEN says where). THEN resume: {resume} --from 12")));
        }
    } else {
        ev.push(p(10, "A", "[internet] dashboard".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} -l 'mgmt-{site}-01*' dashboard-deploy.yml -e site_id={site}")));
        let oidc_play = site_play(repo, site, "oidc-clients.yml");
        ev.push(p(10, "A", format!("[internet] register OIDC clients ({oidc_play})"), format!("cd '{r}/ops/ansible' && {pre} {inv} {oidc_play} -e target={g}")));
        ev.push(p(11, "MANUAL", "[internet] first login, admin-promote, the bootstrap NetBird token".into(), format!("handbook §'Phase 11' has the exact block. In short: enroll YOUR laptop (netbird up), pin the three dashboard hosts in /etc/hosts, read the one-time password from /opt/mgmt-host/.bootstrap-passwords/{oe}.txt, log in; re-run the phase-10 dashboard-deploy line ONCE (promotes you to admin — the UI 403s until then); create service user svc-{site}-netbird-infra (Team > Service Users, role admin), create its token, add 'export {u}_NB_API_TOKEN=<token>' to .envrc.local.d/30-site-services.envrc and store it in 1Password as {site}-netbird / api_token. THEN resume: {resume} --from 12")));
    }
    let fqdn = &s.mgmt_fqdn;
    if !s.joiner {
        ev.push(p(12, "A", "declared NetBird groups (netbird-acl-bootstrap)".into(), format!("cd '{r}/ops/ansible' && ansible-playbook -i localhost, --connection local netbird-acl-bootstrap.yml -e target_site={site} -e netbird_mgmt_url=https://{fqdn}")));
        ev.push(p(12, "A", "declared NetBird policies (cutover apply)".into(), format!("cd '{r}/ops/ansible' && ansible-playbook -i localhost, --connection local netbird-acl-cutover.yml -e target_site={site} -e mode=apply -e netbird_mgmt_url=https://{fqdn}")));
        ev.push(p(12, "A", "enforce: no Default All->All (cutover enforce)".into(), format!("cd '{r}/ops/ansible' && ansible-playbook -i localhost, --connection local netbird-acl-cutover.yml -e target_site={site} -e mode=enforce -e confirm_cutover=YES -e netbird_mgmt_url=https://{fqdn}")));
    }
    let acl_upper = s.acl_site.replace('-', "_").to_uppercase();
    ev.push(p(12, "GATE", format!("the live NetBird account holds only the declared ACL for {}, and no All-group rule", s.acl_site), format!("python3 '{r}/ops/scripts/netbird-policy-posture.py' '{}' --url https://{fqdn} --token-env {acl_upper}_NB_API_TOKEN", s.acl_site)));
    ev.push(p(12, "T", "mint the NetBird service identities this goatnet needs (creates only what is missing)".into(), format!("python3 '{r}/ops/scripts/netbird-service-identities.py' {site} --group {g} --mint")));
    ev.push(p(12, "GATE", "every required NetBird service identity is present and verified".into(), format!("python3 '{r}/ops/scripts/netbird-service-identities.py' {site} --group {g} --live")));
    if !s.joiner {
        ev.push(p(12, "A", "netbird account baseline (mesh-DNS nameserver group; no policy)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} goat-netbird-account-init.yml --limit {g}")));
    }
    if internet {
        ev.push(p(12, "A", "[internet] mesh CoreDNS (only if this site has a mesh zone; joiners use the founder's DNS)".into(), format!("cd '{r}' && python3 ops/dns/generate-mesh-hosts.py && {{ if [ -f ops/dns/{site}/mesh-hosts ]; then cd ops/ansible && {pre} {inv} goat-coredns.yml --limit {g}; else echo '    no ops/dns/{site}/mesh-hosts — this site declares no mesh zone (a joiner on the founder DNS plane); skipping mesh CoreDNS'; fi; }}")));
    }
    ev.push(p(12, "A", "portal (registry, certificate minting, grant rows)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} portal-deploy.yml -e target={g} -l 'mgmt-{site}-01*' -e postgres_superuser_password=\"$(printenv {u}_PG_PASS)\"")));
    if s.obs {
        ev.push(Event::LegJoin { leg: "host", at: 12 });
        let obs_ssh = inventory_hosts(repo, &[og], true).into_iter().next().unwrap_or_default();
        ev.push(Event::Phase(Phase { n: 12, kind: "T", label: "mint the pubsub-state mirror bot on the obs host's Forgejo (once; skipped when FORGEJO_PUBSUB_TOKEN is set)".into(), cmd: format!("if [ -n \"${{FORGEJO_PUBSUB_TOKEN:-}}\" ]; then echo '    FORGEJO_PUBSUB_TOKEN already set — bot minted; skipping';    else '{r}/ops/scripts/forgejo-mint-pubsub-bot.sh' '{obs_ssh}'      && eval \"$('{r}/ops/scripts/envrc-var.sh' FORGEJO_PUBSUB_TOKEN)\"      && '{r}/ops/scripts/standup-keep-secret' FORGEJO_PUBSUB_TOKEN      && echo '    token loaded from the operator envrc; the run continues'; fi"), stop_after: true }));
    }
    ev.push(p(12, "GATE", "every input the phases after the manual step will reach is set (the NetBird service token phase 11 issues)".into(), format!("python3 '{r}/ops/scripts/standup-required-inputs.py' {site} --profile {} --from 12", s.profile)));
    ev.push(p(12, "A", "pubsub-state Repl mirror (Block 86)".into(), format!("cd '{r}/ops/ansible' && if ls group_vars/{g}/pubsub-state.yml >/dev/null 2>&1; then {pre} {inv} pubsub-state-mirror-deploy.yml -e target={g}; else echo 'no group_vars/{g}/pubsub-state.yml — Repl not configured for this goatnet; add one by copying any goatnet that has one, mint the bot, then re-run --from 12'; fi")));
    ev.push(p(12, "A", "onboarding intake front desk (public, ADR 1062 stage 1)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} onboard-intake-deploy.yml -e target={g} -e site_id={site}")));
    ev.push(p(12, "A", "onboard-runner (auto-mint on approval, ADR 1062 stage 3)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} onboard-runner-deploy.yml -e target={g} -e site_id={site}")));
    ev.push(p(12, "A", "grant-keyd (encryption key distribution)".into(), format!("cd '{r}/ops/ansible' && {pre} {inv} grant-keyd-deploy.yml -e target={g} -l 'mgmt-{site}-01*'")));
    ev.push(p(12, "A", "notional pub/sub (synthetic vendors — opt-in via synth_vendors_enabled)".into(), format!("cd '{r}/ops/ansible' && {{ [ ! -e group_vars/{g}/synth-vendors.yml ] || [ -r group_vars/{g}/synth-vendors.yml ] || {{ echo 'cannot read group_vars/{g}/synth-vendors.yml' >&2; false; }}; }} && if grep -qE '^[[:space:]]*synth_vendors_enabled:[[:space:]]*.?(true|True|TRUE|yes|Yes|YES|on|On|ON|1).?[[:space:]]*(#.*)?$' group_vars/{g}/synth-vendors.yml 2>/dev/null; then {pre} {inv} zagros-zenoh-synth-vendors-deploy.yml -e target={g}; else echo 'synth_vendors_enabled not true for {g} — notional pub/sub not requested, skipping'; fi")));
    if s.obs {
        ev.push(Event::LegJoin { leg: "stack", at: 13 });
    }
    // The site's declared end state (ADR 1206) goes with the values: a row whose
    // `requires` names only services the site declares absent reads N/A, so a
    // one-box goatnet is graded against what it declares, not against the fleet.
    let sd = &s.state_dir;
    ev.push(p(13, "GATE", "acceptance suite (values derived from config; all applicable rows green)".into(), crate::assay::phase13(Path::new(r), site, sd).phase_line()));

    // Phase 14 — the two rescue classes (ADR 1183): declared-but-not-inventoried is a
    // real state and prints its reason outside the phase gate, as the script does.
    let gv = repo.join("ops/ansible/group_vars").join(g).join("vars.yml");
    let site_us = site.replace('-', "_");
    let rescue = [
        ("has_breakglass_node", "breakglass_absent_reason", "breakglass", format!("inventory/{site}-breakglass-nodes.ini"), "break-glass node (ADR 1114 — the way in when mesh + control plane are both down)".to_string(), "breakglass-nodes.yml".to_string()),
        ("has_network_jump_node", "network_jump_absent_reason", "network_jump", format!("inventory/{site}-network-jump.ini"), "network-jump host (ADR 1131 — reach to switch, router and out-of-band gear)".to_string(), format!("herd-connector-deploy.yml -e herd_connector_target_hosts={site_us}_network_jump_hosts")),
    ];
    for (flag, reason_key, suffix, extra_inv, label, play) in rescue {
        let rgroup = format!("{site_us}_{suffix}_hosts");
        let declared = gv_value(&gv, flag).unwrap_or_default();
        if declared != "True" && declared != "true" {
            let why = gv_value(&gv, reason_key).filter(|w| !w.is_empty()).unwrap_or_else(|| "(none recorded — the phase-1 config gate refuses this; ADR 1183)".to_string());
            ev.push(Event::Skip { label: label.clone(), lines: vec![format!("SKIPPED — {flag} is not true in group_vars/{g}/vars.yml"), format!("declared reason: {why}")] });
            continue;
        }
        if inventory_hosts(repo, &[&rgroup], false).is_empty() {
            ev.push(Event::Skip { label: label.clone(), lines: vec![format!("SKIPPED — {flag}: true but no host in inventory group [{rgroup}] yet."), "Declared and not yet built is a real state; add the host when the box arrives.".to_string()] });
            continue;
        }
        let inv_extra = if repo.join("ops/ansible").join(&extra_inv).is_file() { format!("-i {extra_inv}") } else { String::new() };
        ev.push(p(14, "A", label, format!("cd '{r}/ops/ansible' && {pre} {inv} {inv_extra} {play} -e target={rgroup}")));
    }
    if s.lite {
        // G-20: cross-env-apply.sh redeploys relay-health on EVERY box in
        // cross-env.yaml -- other goatnets' machines. A goatlite must not
        // reach past its one box (ADR 1213), and a fresh checkout's roster
        // names goatnets whose credentials a learner does not hold.
        ev.push(Event::Skip { label: "join the cross-goatnet surfaces (relay-health overview on every box)".into(), lines: vec![
            "SKIPPED — goatlite (ADR 1213): one box on its own LAN; this step reaches every relay-health box in ops/goatnets/cross-env.yaml, none of which belong to this goatnet".into(),
            "declared reason: cross-env.yaml carries this goatnet as relay_health absent; the overviews pick its roster row up on their next deploy".into()] });
    } else {
        ev.push(p(14, "A", "join the cross-goatnet surfaces (relay-health overview on every box)".into(), format!("'{r}/ops/goatnets/cross-env-apply.sh' --require {site}")));
    }
    ev
}

/// One row per executed phase in `<state_dir>/standup-<site>-timings.tsv`, the
/// same file and columns goat-standup.sh writes (ts, phase, label, seconds, rc),
/// so either avenue's runs read back the same way.
fn record_timing(state_dir: &str, site: &str, n: u32, label: &str, secs: u64, rc: i32) {
    use std::io::Write;
    let dir = Path::new(state_dir);
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(format!("standup-{site}-timings.tsv"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}\t{n}\t{label}\t{secs}\t{rc}", crate::push::stamp_pub());
    }
}

/// The run's timing line: total wall-clock and the slowest phases first, so the
/// next run knows where its hour went.
fn timing_summary(t: &[(u32, String, u64)], total: u64) -> String {
    let mut by: Vec<&(u32, String, u64)> = t.iter().collect();
    by.sort_by_key(|x| std::cmp::Reverse(x.2));
    let top: Vec<String> = by.iter().take(5).map(|(n, l, s)| format!("{n} {l} {s}s")).collect();
    format!("total {total}s ({} min) over {} phases; slowest: {}", total / 60, t.len(), top.join("; "))
}

/// One step of phase 12's dependency graph (ops/goatbuilder/standup-dag.yaml),
/// the file goat-standup.sh reads too.
#[derive(Debug, Clone, Deserialize)]
struct DagStep {
    id: String,
    label: String,
    #[serde(default)]
    needs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DagFile {
    #[serde(default)]
    steps: Vec<DagStep>,
}

fn load_dag(repo: &Path) -> Vec<DagStep> {
    std::fs::read_to_string(repo.join("ops/goatbuilder/standup-dag.yaml"))
        .ok()
        .and_then(|t| serde_yaml::from_str::<DagFile>(&t).ok())
        .map(|d| d.steps)
        .unwrap_or_default()
}

/// The graph step a phase-12 label belongs to: matched by the start of the label
/// after any "[internet] ", "[lan] " or "[joiner] " tag, as dag_index does.
fn dag_lookup<'d>(dag: &'d [DagStep], label: &str) -> Option<&'d DagStep> {
    let mut l = label;
    for tag in ["[internet] ", "[lan] ", "[joiner] "] {
        l = l.strip_prefix(tag).unwrap_or(l);
    }
    dag.iter().find(|d| l.starts_with(d.label.as_str()))
}

/// Each queued step's wave: one more than the latest wave among the queued steps
/// it needs; a need the site never queued is met.
fn waves(q: &[(String, Vec<String>)]) -> Vec<usize> {
    let mut w = vec![1usize; q.len()];
    loop {
        let mut changed = false;
        for i in 0..q.len() {
            for need in &q[i].1 {
                for j in 0..q.len() {
                    if &q[j].0 == need && w[i] <= w[j] {
                        w[i] = w[j] + 1;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return w;
        }
    }
}

struct Queued {
    id: String,
    needs: Vec<String>,
    label: String,
    cmd: String,
}

/// Run the queued phase-12 steps wave by wave. Returns the ids that failed in the
/// first failing wave (empty when every wave passed).
fn flush_waves(repo: &Path, s: &Site, q: &mut Vec<Queued>, checks: &mut Vec<Check>, timings: &mut Vec<(u32, String, u64)>) -> Result<Vec<String>> {
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let pairs: Vec<(String, Vec<String>)> = q.iter().map(|x| (x.id.clone(), x.needs.clone())).collect();
    let w = waves(&pairs);
    let max = w.iter().copied().max().unwrap_or(0);
    std::fs::create_dir_all(&s.state_dir).ok();
    for wave in 1..=max {
        let idx: Vec<usize> = (0..q.len()).filter(|i| w[*i] == wave).collect();
        let ids: String = idx.iter().map(|i| format!(" {}", q[*i].id)).collect();
        checks.push(row("phase", format!("=== Phase 12 wave {wave} of {max}:{ids} ===")));
        let mut kids = Vec::new();
        for i in &idx {
            let log = format!("{}/standup-{}-12-{}.log", s.state_dir, s.site, q[*i].id);
            let f = std::fs::File::create(&log)?;
            let child = phase_command(repo, &q[*i].cmd).current_dir(repo).stdout(Stdio::from(f.try_clone()?)).stderr(Stdio::from(f)).spawn().context("bash")?;
            kids.push((*i, child, std::time::Instant::now(), log));
        }
        let mut failed = Vec::new();
        for (i, mut child, t0, log) in kids {
            let status = child.wait()?;
            let secs = t0.elapsed().as_secs();
            let rc = status.code().unwrap_or(1);
            checks.push(row("time", format!("phase 12 {} took {secs}s (exit {rc})", q[i].id)));
            timings.push((12, q[i].label.clone(), secs));
            record_timing(&s.state_dir, &s.site, 12, &q[i].label, secs, rc);
            if !status.success() {
                let tail: Vec<String> = std::fs::read_to_string(&log).unwrap_or_default().lines().rev().take(15).map(String::from).collect::<Vec<_>>().into_iter().rev().collect();
                for l in tail {
                    checks.push(row("log", format!("| {l}")));
                }
                failed.push(q[i].id.clone());
            }
        }
        if !failed.is_empty() {
            return Ok(failed.into_iter().map(|f| format!("{f} (wave {wave} of {max})")).collect());
        }
    }
    q.clear();
    Ok(Vec::new())
}

/// The value of `export NAME="…"` as ops/scripts/envrc-var.sh prints it from the
/// operator envrc, or None.
fn envrc_export(repo: &Path, name: &str) -> Option<String> {
    let out = Command::new(repo.join("ops/scripts/envrc-var.sh")).arg(name).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let v = line.strip_prefix(&format!("export {name}="))?;
    let v = v.trim_matches('"').trim_matches('\'').to_string();
    (!v.is_empty()).then_some(v)
}

fn row(name: &str, detail: String) -> Check {
    Check { name: name.into(), ok: true, detail, fix: None }
}

/// The last standup phase. Phases are 1..=LAST_PHASE, whole numbers, no gaps.
pub const LAST_PHASE: u32 = 14;

/// `--from`/`--to` for standup and teardown: a phase number 1..=14. A lettered
/// id (6b, 7b, 12m, …) is from the numbering previously ; it is refused
/// with the phase it became, the same words goat-standup.sh prints.
pub fn phase_arg(v: &str) -> std::result::Result<u32, String> {
    let moved = match v {
        "0" | "0b" => Some("phases start at 1 now; the gates are phase 1, the host baseline phase 2".to_string()),
        "6b" => Some("phase 6b is now part of phase 5".to_string()),
        "7a" | "7b" | "7c" => Some(format!("phase {v} is now part of phase 6")),
        "9b" | "9c" => Some(format!("phase {v} is now part of phase 7")),
        "12m" => Some("the first-login step is now phase 11".to_string()),
        _ => None,
    };
    if let Some(m) = moved {
        return Err(m);
    }
    match v.parse::<u32>() {
        Ok(n) if (1..=LAST_PHASE).contains(&n) => Ok(n),
        _ => Err(format!("want a phase number 1..{LAST_PHASE} (docs/operations/goatnet-standup.md §\"Phase numbers\")")),
    }
}

fn in_range(n: u32, o: &Options) -> bool {
    n >= o.from && n <= o.to
}

/// One planned standup line, run the way goat-standup.sh runs it: through
/// ops/scripts/standup-phase-run, which refuses an empty `$(printenv NAME)`,
/// naming it, and under a value store reads the store afresh for each phase.
fn phase_command(repo: &Path, cmd: &str) -> Command {
    let mut c = Command::new(repo.join("ops/scripts/standup-phase-run"));
    c.arg(cmd);
    c
}

/// `standup` on real hosts: the plan, printed; executed unless `--dry-run`.
pub fn standup(repo: &Path, o: &Options) -> Result<Report> {
    // A goatlite run (ADR 1213) declares its one environment in <repo>/goatlite.yaml,
    // not in this repository's own config; either file may declare the environment,
    // and a checkout with no config of its own (the open-source export) has only the first.
    let lite_cfg = resolve_lite(repo, o.site)?;
    let own = repo.join("ops/goatbuilder/goatbuilder.yaml");
    let cfg = if lite_cfg.is_some() && !own.is_file() { None } else { Some(config::load(&own)?) };
    let env = cfg.iter().flat_map(|c| c.environments.iter()).find(|e| e.name == o.env).or_else(|| lite_cfg.as_ref().and_then(|c| c.environments.iter().find(|e| e.name == o.env)));
    let Some(env) = env else {
        let mut declared: Vec<&str> = cfg.iter().flat_map(|c| c.environments.iter()).map(|e| e.name.as_str()).collect();
        declared.extend(lite_cfg.iter().flat_map(|c| c.environments.iter().map(|e| e.name.as_str())));
        return Err(anyhow!("unknown environment \"{}\"; declared: {declared:?}", o.env));
    };
    if !env.sites.is_empty() && !env.sites.iter().any(|s| s.name == o.site) {
        return Err(anyhow!("site '{}' is not declared for environment '{}' (declared: {})", o.site, o.env, env.sites.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    let s = read_site(repo, o)?;
    let resume = format!("goatbuilder standup {} --site {}", o.env, s.site);
    let (from, to) = if o.obs_only { (1, 14) } else { (o.from, o.to) };
    let o = &Options { from, to, ..*o };
    let events = plan(repo, &s, o, &resume);
    let mut checks = vec![row("banner", format!(
        "standup: site={}  group={}  profile={}  obs={}  {}",
        s.site, s.group, s.profile,
        if s.obs { format!("{} (parallel leg)", s.obs_group) } else { format!("none — {}", s.obs_why) },
        if o.dry_run { "(DRY RUN — nothing executes)" } else { "" }
    ))];
    if !o.obs_only && !s.group_vars_exists {
        checks.push(Check { name: "banner".into(), ok: true, detail: format!("!! ops/ansible/group_vars/{} does not exist. This goatnet has no config surface yet, so nothing below can run. Author it first — it is a FILE INVENTORY (~30 files), not a single vars.yml: ops/scripts/standup-config-preflight.py {} --scaffold-plan; an inventory group [{}]; a cert manifest ops/enrollment/manifests/{}-certs.yaml (copy TEMPLATE-certs.yaml). Full pre-phase list: docs/operations/goatnet-standup.md §\"Before phase 1\".", s.group, s.group, s.group, s.site), fix: None });
    }
    // Fresh source (C3): once, before phase 1 touches a host. Every play then goes
    // through ansible-preflight.sh judged against the same trunk commit, so a merge
    // landing during a long standup does not stop it half way.
    if !o.dry_run {
        let f = crate::fresh::check(repo)?;
        let refused = f.refused();
        if !f.trunk.is_empty() {
            std::env::set_var(crate::fresh::TRUNK_ENV, &f.trunk);
        }
        checks.push(f.check);
        if refused {
            return Ok(Report::new("standup", checks, crate::EXIT_REFUSED));
        }
    }

    let mut legs: BTreeMap<&'static str, (std::process::Child, String)> = BTreeMap::new();
    let mut pulling: Option<(std::process::Child, String)> = None;
    let mut timings: Vec<(u32, String, u64)> = Vec::new();
    let dag = load_dag(repo);
    let mut queue: Vec<Queued> = Vec::new();
    let run_started = std::time::Instant::now();
    let mut exit = EXIT_OK;
    'run: for ev in &events {
        let leaving_12 = !matches!(ev, Event::Phase(ph) if ph.n == 12);
        if leaving_12 && !queue.is_empty() {
            let failed = flush_waves(repo, &s, &mut queue, &mut checks, &mut timings)?;
            if !failed.is_empty() {
                let prof = if s.profile != "lan" { format!(" --profile {}", s.profile) } else { String::new() };
                checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase 12 FAILED: {}. Steps in that wave that passed stay done; no later wave has run.", failed.join(", ")), fix: Some(format!("every phase-12 step is idempotent: fix each failure named above, then resume: {resume} --from 12 --parallel{prof}")) });
                exit = EXIT_REFUSED;
                break 'run;
            }
        }
        match ev {
            Event::Skip { label, lines } => {
                checks.push(row("phase", format!("=== Phase 14 [A] {label} ===")));
                for l in lines {
                    checks.push(row("note", l.clone()));
                }
            }
            Event::LegStart { leg, label, cmd, log } => {
                checks.push(row("leg", format!("=== Observability leg \"{leg}\" [A ∥] {label} ===")));
                checks.push(row("cmd", cmd.clone()));
                checks.push(row("note", format!("(log: {log})")));
                if o.dry_run {
                    continue;
                }
                if let Some(d) = Path::new(log).parent() {
                    std::fs::create_dir_all(d)?;
                }
                let f = std::fs::File::create(log)?;
                let child = phase_command(repo, cmd).current_dir(repo).stdout(Stdio::from(f.try_clone()?)).stderr(Stdio::from(f)).spawn().context("bash")?;
                legs.insert(leg, (child, log.clone()));
            }
            Event::PullStart { label, cmd, log } => {
                checks.push(row("phase", format!("=== Image pull [A ∥] {label} ===")));
                checks.push(row("cmd", cmd.clone()));
                checks.push(row("note", format!("(log: {log})")));
                if o.dry_run {
                    continue;
                }
                if let Some(d) = Path::new(log).parent() {
                    std::fs::create_dir_all(d)?;
                }
                let f = std::fs::File::create(log)?;
                let child = phase_command(repo, cmd).current_dir(repo).stdout(Stdio::from(f.try_clone()?)).stderr(Stdio::from(f)).spawn().context("bash")?;
                pulling = Some((child, log.clone()));
            }
            Event::PullJoin { at } => {
                if o.dry_run {
                    checks.push(row("phase", format!("=== join the image pull before phase {at} ===")));
                    continue;
                }
                let Some((mut child, log)) = pulling.take() else { continue };
                checks.push(row("phase", format!("=== join the image pull before phase {at} (waiting) ===")));
                let status = child.wait()?;
                let tail: Vec<String> = std::fs::read_to_string(&log).unwrap_or_default().lines().rev().take(25).map(String::from).collect::<Vec<_>>().into_iter().rev().collect();
                for l in tail {
                    checks.push(row("log", format!("| {l}")));
                }
                if !status.success() {
                    let rc = status.code().unwrap_or(1);
                    let prof = if s.profile != "lan" { format!(" --profile {}", s.profile) } else { String::new() };
                    checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Image pull FAILED — exit {rc} (full log: {log}). Phase {at} and every later phase run these images, so the run stops here; the log names each image that could not be pulled."), fix: Some(format!("a private package needs GHCR_READ_TOKEN on the laptop (phase 2 logs the box in with it); a pin naming an image the registry no longer has needs a new pin in ops/pins/. Then resume: {resume} --from {at}{prof}")) });
                    exit = EXIT_REFUSED;
                    break 'run;
                }
            }
            Event::LegJoin { leg, at } => {
                if o.dry_run {
                    checks.push(row("leg", format!("=== join observability leg \"{leg}\" before phase {at} ===")));
                    continue;
                }
                let Some((mut child, log)) = legs.remove(leg) else { continue };
                checks.push(row("leg", format!("=== join observability leg \"{leg}\" before phase {at} (waiting) ===")));
                let status = child.wait()?;
                let tail: Vec<String> = std::fs::read_to_string(&log).unwrap_or_default().lines().rev().take(25).map(String::from).collect::<Vec<_>>().into_iter().rev().collect();
                for l in tail {
                    checks.push(row("log", format!("| {l}")));
                }
                if !status.success() {
                    let rc = status.code().unwrap_or(1);
                    checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Observability leg \"{leg}\" FAILED — exit {rc} (full log: {log}). The core phases up to {at} have run; phase {at} needs this leg, so the run stops here."), fix: Some(format!("fix the cause (the log names it; obs_scrape_targets unset is the usual one for the stack leg), then: {resume} --obs-only && {resume} --from {at}")) });
                    exit = EXIT_REFUSED;
                    break 'run;
                }
            }
            Event::Phase(ph) => {
                if !in_range(ph.n, o) {
                    continue;
                }
                checks.push(row("phase", format!("=== Phase {} [{}] {} ===", ph.n, ph.kind, ph.label)));
                checks.push(row("cmd", ph.cmd.clone()));
                let step = if ph.n == 12 { dag_lookup(&dag, &ph.label) } else { None };
                if let Some(d) = step {
                    checks.push(row("note", format!("needs: {}", if d.needs.is_empty() { "nothing".to_string() } else { d.needs.join(" ") })));
                }
                if o.dry_run {
                    continue;
                }
                if let (true, Some(d)) = (o.parallel, step) {
                    queue.push(Queued { id: d.id.clone(), needs: d.needs.clone(), label: ph.label.clone(), cmd: ph.cmd.clone() });
                    checks.push(row("note", "(queued for the phase-12 waves)".into()));
                    continue;
                }
                if ph.kind == "MANUAL" {
                    checks.push(row("note", format!("^ OPERATOR STEP — run the above, then re-run with --from {}", ph.n + 1)));
                    break 'run;
                }
                let t0 = std::time::Instant::now();
                let status = phase_command(repo, &ph.cmd).current_dir(repo).status().context("bash")?;
                let secs = t0.elapsed().as_secs();
                let rc_now = status.code().unwrap_or(1);
                checks.push(row("time", format!("phase {} took {secs}s (exit {rc_now})", ph.n)));
                timings.push((ph.n, ph.label.clone(), secs));
                record_timing(&s.state_dir, &s.site, ph.n, &ph.label, secs, rc_now);
                if !status.success() {
                    let rc = status.code().unwrap_or(1);
                    let prof = if s.profile != "lan" { format!(" --profile {}", s.profile) } else { String::new() };
                    checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase {} (\"{}\") FAILED — exit {rc}. No later phase has run; the site is left partway through phase {}. Read the error above; docs/operations/goatnet-standup.md \"The run\" table has this phase's \"Worked when\" and the Troubleshooting table lists the common failures by symptom + fix.", ph.n, ph.label, ph.n), fix: Some(format!("fix the cause, then resume — every phase is idempotent: {resume} --from {}{prof}", ph.n)) });
                    exit = EXIT_REFUSED;
                    break 'run;
                }
                // The bot step loaded its token into its own shell; this process runs every
                // later phase in a fresh one, so load the same one variable here and go on.
                if ph.stop_after && std::env::var("FORGEJO_PUBSUB_TOKEN").map(|v| v.is_empty()).unwrap_or(true) {
                    if let Some(v) = envrc_export(repo, "FORGEJO_PUBSUB_TOKEN") {
                        std::env::set_var("FORGEJO_PUBSUB_TOKEN", v);
                    }
                }
                if ph.stop_after && std::env::var("FORGEJO_PUBSUB_TOKEN").map(|v| v.is_empty()).unwrap_or(true) {
                    checks.push(row("note", format!("stopped after this phase on purpose: reload the environment it wrote, then resume: {resume} --from {}", ph.n)));
                    break 'run;
                }
            }
        }
    }
    if exit == EXIT_OK && !queue.is_empty() {
        let failed = flush_waves(repo, &s, &mut queue, &mut checks, &mut timings)?;
        if !failed.is_empty() {
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("!! Phase 12 FAILED: {}.", failed.join(", ")), fix: Some(format!("fix each failure named above, then resume: {resume} --from 12 --parallel")) });
            exit = EXIT_REFUSED;
        }
    }
    if o.dry_run {
        checks.push(row("footer", format!("PLAN complete ({} profile). Remove --dry-run to execute. Manual steps (phase 8 enrolment and phase 11 first login, internet only) pause the run; resume with --from <next>. Read docs/operations/goatnet-standup.md first.", s.profile)));
    } else if o.obs_only && exit == EXIT_OK {
        checks.push(row("footer", format!("obs leg complete for {}.", s.site)));
    }
    if !o.dry_run && !timings.is_empty() {
        checks.push(row("timings", timing_summary(&timings, run_started.elapsed().as_secs())));
    }
    if let Some((mut child, log)) = pulling {
        let _ = child.try_wait();
        checks.push(row("note", format!("the image pull is still running in the background (log: {log})")));
    }
    // A leg still running when the run stopped early is left to finish; its log names it.
    for (leg, (mut child, log)) in legs {
        let _ = child.try_wait();
        checks.push(row("note", format!("obs leg {leg} still running in the background (log: {log})")));
    }
    let mut r = Report::new("standup", checks, EXIT_REFUSED);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    Ok(r)
}

// ---------------------------------------------------------------------------
// The rehearsal on a fixture.
// ---------------------------------------------------------------------------

/// `standup:` in the fixture's converger.yaml: the site, its classes'
/// machines, and the doctrine's phase order as class → components.
#[derive(Debug, Deserialize)]
struct MockScript {
    standup: MockStandup,
}

#[derive(Debug, Deserialize)]
struct MockStandup {
    env: String,
    site: String,
    /// Class → machines. A key `<site>/<class>` names another site's class: a
    /// joiner's standup changes its founder (the founder-side mirror, phase 7),
    /// and the rehearsal records that on the founder's machines.
    classes: BTreeMap<String, Vec<String>>,
    phases: Vec<MockPhase>,
    /// Records an earlier standup left on this environment that `status` must
    /// still expect, as component → machines — a joiner's rehearsal starts from
    /// a goatnet whose founder already converged.
    #[serde(default)]
    carried: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct MockPhase {
    phase: u32,
    class: String,
    components: Vec<String>,
}

/// What a fixture rehearsal honours from the command line, as a real run does.
#[derive(Clone, Copy)]
pub struct MockRun {
    pub dry_run: bool,
    pub from: u32,
    pub to: u32,
}

impl MockRun {
    /// A real rehearsal of every phase.
    #[cfg(test)]
    pub const ALL: MockRun = MockRun { dry_run: false, from: 1, to: u32::MAX };
}

/// Observations per machine before a pending machine is called not converged.
const MOCK_MAX_TICKS: u32 = 8;

/// The class scope a fixture declares, as component → the machines that carry
/// it; `None` when the fixture rehearses no standup (every machine carries
/// every component, as `fixtures/basic/` does). `status` reads this so a
/// component declared for one class is never reported pending on another.
pub fn class_scope(root: &Path) -> Option<BTreeMap<String, Vec<String>>> {
    let text = std::fs::read_to_string(root.join("converger.yaml")).ok()?;
    let script: MockScript = serde_yaml::from_str(&text).ok()?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (c, machines) in &script.standup.carried {
        let e = out.entry(c.clone()).or_default();
        for m in machines {
            if !e.contains(m) {
                e.push(m.clone());
            }
        }
    }
    for ph in &script.standup.phases {
        let machines = script.standup.classes.get(&ph.class).cloned().unwrap_or_default();
        for c in &ph.components {
            let e = out.entry(c.clone()).or_default();
            for m in &machines {
                if !e.contains(m) {
                    e.push(m.clone());
                }
            }
        }
    }
    Some(out)
}

/// `standup --mock <fixture>`: declare every component for every class, in
/// the phase order the fixture states, converge each through the scripted
/// converger, verify every record, and report declared → verified time.
pub fn mock(fixture: &mut Fixture, env: &str, site: &str, actor: &str, run: MockRun) -> Result<Report> {
    let text = std::fs::read_to_string(fixture.root.join("converger.yaml"))?;
    let script: MockScript = serde_yaml::from_str(&text).context("converger.yaml has no standup: section (goatbuilder-conformance.md §\"Rehearsing a standup on the fixture\")")?;
    let m = script.standup;
    if m.env != env {
        return Err(anyhow!("the fixture rehearses environment '{}', not '{env}'", m.env));
    }
    if m.site != site {
        return Err(anyhow!("the fixture rehearses site '{}', not '{site}'", m.site));
    }
    let declared_site = fixture.config.environments.iter().find(|e| e.name == env).and_then(|e| e.sites.iter().find(|s| s.name == site));
    let Some(declared_site) = declared_site else {
        return Err(anyhow!("site '{site}' is not declared for environment '{env}' in goatbuilder.yaml"));
    };
    let env_sites = &fixture.config.environments.iter().find(|e| e.name == env).expect("found above").sites;
    for ph in &m.phases {
        if let Some((other, class)) = ph.class.split_once('/') {
            // Another site's class: only the founder this joiner attaches to.
            if declared_site.joins.as_deref() != Some(other) {
                return Err(anyhow!("phase {}: class '{}' names site '{other}', but '{site}' joins {} — a standup may touch only the founder it attaches to", ph.phase, ph.class, declared_site.joins.as_deref().map(|j| format!("'{j}'")).unwrap_or_else(|| "no site".into())));
            }
            let Some(founder) = env_sites.iter().find(|s| s.name == other) else {
                return Err(anyhow!("phase {}: site '{other}' is not declared for environment '{env}'", ph.phase));
            };
            if !founder.classes.contains_key(class) {
                return Err(anyhow!("phase {}: class '{class}' is not declared at site '{other}' in goatbuilder.yaml", ph.phase));
            }
        } else if !declared_site.classes.contains_key(&ph.class) {
            return Err(anyhow!("phase {}: class '{}' is not declared at site '{site}' in goatbuilder.yaml", ph.phase, ph.class));
        }
        if !m.classes.contains_key(&ph.class) {
            return Err(anyhow!("phase {}: class '{}' has no machines in converger.yaml standup.classes", ph.phase, ph.class));
        }
    }
    // A dry run records nothing, so it needs no actor.
    if !run.dry_run {
        crate::verbs::attributed(actor)?;
    }
    let started = std::time::Instant::now();
    let mut checks = Vec::new();
    let mut all_converged = true;
    let mut machines_touched = std::collections::BTreeSet::new();
    let mut components = 0usize;
    for ph in m.phases.iter().filter(|ph| ph.phase >= run.from && ph.phase <= run.to) {
        let machines = &m.classes[&ph.class];
        checks.push(row("phase", format!("=== Phase {} [{}] {} ===", ph.phase, ph.class, ph.components.join(", "))));
        for component in &ph.components {
            components += 1;
            let declared = fixture.declared_ref(env, component)?;
            if !declared.contains("@sha256:") {
                checks.push(Check { name: format!("{env}/{site}/{}/{component}/declaration", ph.class), ok: false, detail: format!("{declared} is a mutable tag; a standup declares digests (C1)"), fix: Some("pin the declaration to <image>@sha256:<digest> in declared/pins.yaml".into()) });
                all_converged = false;
                continue;
            }
            if run.dry_run {
                machines_touched.extend(machines.iter().cloned());
                checks.push(row(&format!("{env}/{site}/{}/{component}/plan", ph.class), format!("DRY: would publish {} and observe {} machine(s)", crate::verbs::short(&declared), machines.len())));
                continue;
            }
            let serial = fixture.converger.publish(env, component, &declared, actor)?;
            checks.push(row(&format!("{env}/{site}/{}/{component}/publish", ph.class), format!("serial {serial}: {} by {actor}", crate::verbs::short(&declared))));
            let mut ticks = 0;
            loop {
                ticks += 1;
                let observed = fixture.converger.observe_on(env, component, &declared, Some(machines))?;
                let pending = observed.values().any(|o| *o == crate::converger::Outcome::Pending);
                if !pending || ticks >= MOCK_MAX_TICKS {
                    break;
                }
            }
            for machine in machines {
                machines_touched.insert(machine.clone());
                let name = format!("{env}/{site}/{}/{machine}/{component}", ph.class);
                match fixture.converger.read_record(env, machine, component) {
                    Ok(Some(r)) if r.reference == declared => checks.push(row(&name, format!("converged on {} after {ticks} observation{}", crate::verbs::short(&declared), if ticks == 1 { "" } else { "s" }))),
                    Ok(_) => { all_converged = false; checks.push(Check { name, ok: false, detail: format!("pending after {ticks} observations"), fix: None }); }
                    Err(e) => { all_converged = false; checks.push(Check { name, ok: false, detail: format!("unknown: {e}"), fix: None }); }
                }
            }
        }
    }
    let elapsed = started.elapsed();
    if run.dry_run {
        checks.push(row("plan", format!("DRY: {components} component(s) on {} machine(s); nothing was published or read", machines_touched.len())));
    } else {
    checks.push(row("measured", format!(
        "declared → verified: {} component{} on {} machine{} in {} ms{}",
        components, if components == 1 { "" } else { "s" },
        machines_touched.len(), if machines_touched.len() == 1 { "" } else { "s" },
        elapsed.as_millis(),
        if all_converged { " — every record names its declared ref" } else { " — NOT every record names its declared ref (exit 3)" }
    )));
    }
    let exit = if checks.iter().any(|c| c.fix.is_some()) { EXIT_REFUSED } else if all_converged { EXIT_OK } else { EXIT_NOT_CONVERGED };
    let mut r = Report::new("standup", checks, exit);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    Ok(r)
}

// ---------------------------------------------------------------------------
// teardown — standup's mirror: a site from verified to nothing, records kept.
// ---------------------------------------------------------------------------

/// `teardown --mock <fixture>`: every component the site's standup declared,
/// in REVERSE phase order, gets an `action: teardown` row on each machine of
/// its class. Nothing earlier in any record is removed (history is evidence),
/// and `status` afterwards reads the site as torn down, not as pending.
pub fn teardown_mock(fixture: &mut Fixture, env: &str, site: &str, run: MockRun) -> Result<Report> {
    let text = std::fs::read_to_string(fixture.root.join("converger.yaml"))?;
    let script: MockScript = serde_yaml::from_str(&text).context("converger.yaml has no standup: section")?;
    let m = script.standup;
    if m.env != env || m.site != site {
        return Err(anyhow!("the fixture rehearses {}/{}, not {env}/{site}", m.env, m.site));
    }
    let started = std::time::Instant::now();
    let mut checks = Vec::new();
    let mut rows = 0usize;
    for ph in m.phases.iter().rev().filter(|ph| ph.phase >= run.from && ph.phase <= run.to) {
        // Another site's class (a joiner's founder-side mirror) is not this
        // site's to remove: the founder keeps running.
        if ph.class.contains('/') {
            checks.push(row("skip", format!("phase {} [{}]: another site's class — left running", ph.phase, ph.class)));
            continue;
        }
        let machines = m.classes.get(&ph.class).cloned().unwrap_or_default();
        checks.push(row("phase", format!("=== Teardown of phase {} [{}] {} ===", ph.phase, ph.class, ph.components.join(", "))));
        for component in ph.components.iter().rev() {
            for machine in &machines {
                let name = format!("{env}/{site}/{}/{machine}/{component}", ph.class);
                match fixture.converger.read_record(env, machine, component) {
                    Ok(Some(r)) if r.action == "teardown" => checks.push(row(&name, "already torn down — nothing to do (C5)".into())),
                    Ok(Some(_)) if run.dry_run => {
                        rows += 1;
                        checks.push(row(&name, "DRY: would tear down; nothing changed".into()));
                    }
                    Ok(Some(_)) => {
                        fixture.converger.record_teardown(env, machine, component)?;
                        rows += 1;
                        checks.push(row(&name, "torn down; earlier rows kept".into()));
                    }
                    Ok(None) => checks.push(row(&name, "never deployed here — nothing to do".into())),
                    Err(e) => checks.push(Check { name, ok: false, detail: format!("unknown: {e}"), fix: None }),
                }
            }
        }
    }
    if run.dry_run {
        checks.push(row("plan", format!("DRY: {rows} record{} would be appended; nothing changed", if rows == 1 { "" } else { "s" })));
    } else {
        checks.push(row("measured", format!("verified → torn down: {rows} record{} appended in {} ms; no record removed", if rows == 1 { "" } else { "s" }, started.elapsed().as_millis())));
    }
    let exit = if checks.iter().any(|c| !c.ok) { EXIT_NOT_CONVERGED } else { EXIT_OK };
    let mut r = Report::new("teardown", checks, exit);
    r.exit = exit;
    r.ok = exit == EXIT_OK;
    Ok(r)
}

/// How one standup step comes down (ops/goatbuilder/teardown.yaml, the file
/// ops/scripts/goat-teardown.sh reads too).
#[derive(Debug, Clone, Deserialize)]
struct TeardownDecl {
    label: String,
    action: String,
    #[serde(default)]
    component: String,
    #[serde(default)]
    dirs: Vec<TeardownDir>,
    #[serde(default)]
    why: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct TeardownDir {
    var: String,
    default: String,
}

#[derive(Debug, Deserialize)]
struct TeardownFile {
    #[serde(default)]
    steps: Vec<TeardownDecl>,
}

fn load_teardown(repo: &Path) -> Result<Vec<TeardownDecl>> {
    let t = std::fs::read_to_string(repo.join("ops/goatbuilder/teardown.yaml")).context("ops/goatbuilder/teardown.yaml")?;
    Ok(serde_yaml::from_str::<TeardownFile>(&t).context("teardown.yaml")?.steps)
}

fn teardown_decl<'d>(decls: &'d [TeardownDecl], label: &str) -> Option<&'d TeardownDecl> {
    let mut l = label;
    for tag in ["[internet] ", "[lan] ", "[joiner] "] {
        l = l.strip_prefix(tag).unwrap_or(l);
    }
    decls.iter().find(|d| l.starts_with(d.label.as_str()))
}

/// `teardown <env> --site <s>`: the standup's applied steps in reverse, each
/// coming down as ops/goatbuilder/teardown.yaml declares. Without `--services`
/// it refuses (exit 3) when any reached step would destroy data, is not built,
/// or is undeclared — a half-removed site is worse than a standing one. With
/// `--services` it stops every compose-down service, volumes kept, proves each
/// stop on the host, and lists what it left in place.
pub fn teardown(repo: &Path, o: &Options, services: bool) -> Result<Report> {
    let s = read_site(repo, o)?;
    let resume = format!("goatbuilder teardown {} --site {}", o.env, s.site);
    let decls = load_teardown(repo)?;
    let events = plan(repo, &s, &Options { from: 1, to: 14, ..*o }, &resume);
    let mut checks = vec![row("banner", format!("teardown: site={}  group={}  {}", s.site, s.group, if o.dry_run { "(DRY RUN — nothing executes)" } else { "" }))];
    let applied: Vec<&Phase> = events.iter().filter_map(|e| if let Event::Phase(p) = e { Some(p) } else { None }).filter(|p| p.kind == "A" && p.n >= o.from && p.n <= o.to).collect();
    let mut cmds: Vec<(String, String)> = Vec::new();
    let mut blocking: Vec<String> = Vec::new();
    for p in applied.iter().rev() {
        checks.push(row("phase", format!("=== Teardown of phase {} [A] {} ===", p.n, p.label)));
        match teardown_decl(&decls, &p.label) {
            None => {
                checks.push(row("note", "undeclared: no line in ops/goatbuilder/teardown.yaml says how this step comes down".into()));
                blocking.push(format!("{} {} (undeclared)", p.n, p.label));
            }
            Some(d) if d.action == "compose-down" => {
                let dirs = serde_json::to_string(&d.dirs)?;
                let cmd = format!("cd '{}/ops/ansible' && {} {} teardown-compose.yml -e target={} -e teardown_component={} -e 'teardown_dirs={dirs}'", s.repo, s.ansible_pre, s.inv, s.group, d.component);
                checks.push(row("note", format!("compose-down: stop {}, volumes kept", d.component)));
                checks.push(row("cmd", cmd.clone()));
                cmds.push((d.component.clone(), cmd));
            }
            Some(d) => {
                checks.push(row("note", format!("{}: {}", d.action, d.why)));
                if d.action == "refused" || d.action == "designed" {
                    blocking.push(format!("{} {} ({})", p.n, p.label, d.action));
                }
            }
        }
    }
    if !blocking.is_empty() && !services {
        checks.push(row("phase", format!("=== Teardown verdict: refused: {} step(s) cannot come down by this teardown (data, not built, or undeclared): {}. --services stops the services only and leaves those in place. ===", blocking.len(), blocking.join("; "))));
        if !o.dry_run {
            checks.push(Check { name: "refused".into(), ok: false, detail: "a reached step would destroy data, is not built, or is undeclared; nothing was touched".into(), fix: Some(format!("{resume} --services   (stop the services, keep the account and data); read the order with --dry-run")) });
        }
        let mut r = Report::new("teardown", checks, EXIT_REFUSED);
        r.exit = if o.dry_run { EXIT_OK } else { EXIT_REFUSED };
        r.ok = o.dry_run;
        return Ok(r);
    }
    let left = if blocking.is_empty() { String::new() } else { format!("; {} step(s) left in place", blocking.len()) };
    checks.push(row("phase", format!("=== Teardown verdict: {} service(s) come down, volumes kept{left}. Records go to teardown.jsonl; deploy-history.jsonl is untouched. ===", cmds.len())));
    if o.dry_run {
        return Ok(Report::new("teardown", checks, EXIT_OK));
    }
    for (component, cmd) in &cmds {
        let t0 = std::time::Instant::now();
        let status = Command::new("bash").args(["-euo", "pipefail", "-c", cmd]).current_dir(repo).status().context("bash")?;
        let rc = status.code().unwrap_or(1);
        checks.push(row("time", format!("teardown {component} took {}s (exit {rc})", t0.elapsed().as_secs())));
        if !status.success() {
            checks.push(Check { name: "failed".into(), ok: false, detail: format!("teardown of {component} FAILED — exit {rc}; the services before it are down, nothing after it ran"), fix: Some(format!("fix the cause, then re-run — every stop is idempotent: {resume} --services")) });
            let mut r = Report::new("teardown", checks, EXIT_REFUSED);
            r.exit = EXIT_REFUSED;
            return Ok(r);
        }
    }
    Ok(Report::new("teardown", checks, EXIT_OK))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both drivers run a planned line through the one runner that refuses an empty
    /// `$(printenv NAME)` (tests/test_standup_phase_run.py tests its refusal).
    #[test]
    fn a_phase_runs_through_the_shared_runner() {
        let c = phase_command(Path::new("/r"), "true");
        assert_eq!(c.get_program(), "/r/ops/scripts/standup-phase-run");
        assert_eq!(c.get_args().collect::<Vec<_>>(), vec!["true"]);
        let sh = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/goat-standup.sh")).unwrap();
        assert_eq!(sh.matches("ops/scripts/standup-phase-run\"").count(), 5, "the serial step (twice), the waves, the legs and the image pull");
    }

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn teardown_declarations_match_labels_and_name_their_dirs_as_the_script_does() {
        let decls = load_teardown(&repo_root()).unwrap();
        assert_eq!(teardown_decl(&decls, "portal (registry, certificate minting, grant rows)").unwrap().action, "compose-down");
        assert_eq!(teardown_decl(&decls, "[internet] relay tier (coturn + rathole + TLS renewal)").unwrap().action, "refused");
        assert_eq!(teardown_decl(&decls, "break-glass node (ADR 1114)").unwrap().action, "keep");
        let intake = teardown_decl(&decls, "onboarding intake front desk (public)").unwrap();
        assert_eq!(serde_json::to_string(&intake.dirs).unwrap(), r#"[{"var":"onboard_intake_dir","default":"/opt/onboard-intake"},{"var":"intake_mesh_advisory_compose_dir","default":"/opt/intake-mesh-advisory"}]"#);
        assert!(teardown_decl(&decls, "a step nobody declared").is_none());
    }

    #[test]
    fn a_full_teardown_of_a_live_site_is_refused_and_services_only_is_not() {
        let o = Options { env: "zagros_dev", site: "zagros-dev", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let full = teardown(&repo_root(), &o, false).unwrap();
        assert!(full.checks.iter().any(|c| c.detail.starts_with("=== Teardown verdict: refused:")));
        let svc = teardown(&repo_root(), &Options { from: 12, ..o }, true).unwrap();
        assert!(svc.checks.iter().any(|c| c.detail.contains("service(s) come down, volumes kept")));
        assert!(svc.checks.iter().filter(|c| c.detail.contains("teardown-compose.yml")).count() >= 4);
    }

    #[test]
    fn dag_is_read_and_matches_labels_past_their_profile_tag() {
        let dag = load_dag(&repo_root());
        assert_eq!(dag.len(), 14);
        assert_eq!(dag_lookup(&dag, "portal (registry, certificate minting, grant rows)").unwrap().id, "portal");
        assert_eq!(dag_lookup(&dag, "[internet] mesh CoreDNS (only if this site has a mesh zone)").unwrap().id, "mesh-dns");
        assert!(dag_lookup(&dag, "mint the pubsub-state mirror bot on the obs host's Forgejo").is_none());
    }

    #[test]
    fn waves_put_independent_steps_together_and_respect_needs() {
        let q = |id: &str, n: &[&str]| (id.to_string(), n.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        let w = waves(&[
            q("acl-groups", &[]), q("acl-policies", &["acl-groups"]), q("acl-enforce", &["acl-policies"]),
            q("acl-gate", &["acl-enforce"]), q("si-mint", &["acl-groups"]), q("si-gate", &["si-mint"]),
            q("account-baseline", &["acl-groups"]), q("mesh-dns", &["account-baseline"]),
            q("portal", &["acl-gate", "si-gate"]), q("pubsub-mirror", &["portal"]), q("intake", &["portal"]),
            q("runner", &["portal"]), q("grant-keyd", &["portal"]), q("synth", &["portal"]),
        ]);
        assert_eq!(w, vec![1, 2, 3, 4, 2, 3, 2, 3, 5, 6, 6, 6, 6, 6]);
    }

    #[test]
    fn a_need_the_site_never_queued_is_met() {
        // a joiner queues no ACL steps: the portal then needs only the identities
        let q = vec![("si-mint".to_string(), vec!["acl-groups".to_string()]), ("si-gate".to_string(), vec!["si-mint".to_string()]), ("portal".to_string(), vec!["acl-gate".to_string(), "si-gate".to_string()])];
        assert_eq!(waves(&q), vec![1, 2, 3]);
    }

    #[test]
    fn a_wave_runs_its_steps_together_and_stops_on_a_failure() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        // The phase runner reads the checkout's secrets posture; a learner's
        // goatlite.yaml at the root would turn it into a value store (ADR 1213 D4).
        std::env::set_var("GOAT_LITE_CONFIG", "/nonexistent/goatlite.yaml");
        let t = std::env::temp_dir().join(format!("gb-waves-{}", std::process::id()));
        std::fs::create_dir_all(&t).unwrap();
        let site = Site {
            repo: String::new(), site: "zagros".into(), group: "zagros_hosts".into(), upper: "ZAGROS".into(), obs_group: String::new(),
            state_dir: t.display().to_string(), ansible_pre: String::new(), inv: "", profile: "lan".into(), joiner: false,
            join_founder_site: String::new(), join_founder_group: String::new(), acl_site: "zagros".into(), mgmt_fqdn: String::new(),
            intra_pair: false, obs: false, obs_why: String::new(), group_vars_exists: true, lite: false, nb_token_available: false,
            owner_email: "$OWNER_EMAIL".to_string(),
        };
        let mk = |id: &str, needs: &[&str], cmd: &str| Queued { id: id.into(), needs: needs.iter().map(|x| x.to_string()).collect(), label: id.into(), cmd: cmd.into() };
        // The repository root: a step runs through its ops/scripts/standup-phase-run.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut q = vec![mk("a", &[], "sleep 2"), mk("b", &[], "sleep 2"), mk("c", &["a", "b"], "true")];
        let (mut checks, mut timings) = (Vec::new(), Vec::new());
        let t0 = std::time::Instant::now();
        let failed = flush_waves(&root, &site, &mut q, &mut checks, &mut timings).unwrap();
        assert!(failed.is_empty(), "{failed:?}");
        // Serial would take 4 s; each step also starts the shared runner, a few hundred ms on a loaded machine.
        assert!(t0.elapsed().as_millis() < 3600, "a and b ran together: {:?}", t0.elapsed());
        assert_eq!(timings.len(), 3);
        let mut q = vec![mk("a", &[], "exit 3"), mk("b", &[], "true"), mk("c", &["a"], "true")];
        let failed = flush_waves(&root, &site, &mut q, &mut checks, &mut Vec::new()).unwrap();
        assert_eq!(failed, vec!["a (wave 1 of 2)".to_string()]);
        let _ = std::fs::remove_dir_all(&t);
    }

    fn test_site(joiner: bool) -> Site {
        Site {
            repo: "/r".into(), site: "zagros".into(), group: "zagros_hosts".into(), upper: "ZAGROS".into(), obs_group: String::new(),
            state_dir: "/s".into(), ansible_pre: "PRE".into(), inv: "INV", profile: "lan".into(), joiner,
            join_founder_site: if joiner { "alpha".into() } else { String::new() }, join_founder_group: if joiner { "alpha_hosts".into() } else { String::new() },
            acl_site: "zagros".into(), mgmt_fqdn: String::new(), intra_pair: false, obs: false, obs_why: String::new(),
            group_vars_exists: true, lite: false, nb_token_available: false, owner_email: "$OWNER_EMAIL".to_string(),
        }
    }

    fn phase_lines(s: &Site, o: &Options) -> Vec<(u32, String, String)> {
        plan(&repo_root(), s, o, "RESUME").into_iter().filter_map(|e| match e { Event::Phase(p) => Some((p.n, p.label, p.cmd)), _ => None }).collect()
    }

    /// Phase 5 no longer stops on purpose and phase 6 no longer re-runs the whole
    /// mgmt stack: 5 runs site-bootstrap's `stack` plays, 6 deploys Ory and then
    /// the `after-ory` plays, whose seed waits for the schema with Ory up.
    #[test]
    fn phase_5_runs_the_stack_and_phase_6_the_rest_after_ory() {
        let o = Options { env: "e", site: "zagros", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let lines = phase_lines(&test_site(false), &o);
        let sb: Vec<&(u32, String, String)> = lines.iter().filter(|l| l.2.contains("site-bootstrap.yml")).collect();
        assert_eq!(sb.len(), 2, "{sb:?}");
        assert_eq!(sb[0].0, 5);
        assert!(sb[0].2.ends_with("--skip-tags after-ory"), "{}", sb[0].2);
        assert!(!sb[0].2.contains("never had Ory") && !sb[0].2.contains("grep"), "no expected-stop wrapper: {}", sb[0].2);
        assert_eq!(sb[1].0, 6);
        assert!(sb[1].2.ends_with("--skip-tags stack"), "{}", sb[1].2);
        let ory = lines.iter().position(|l| l.2.contains("ory-deploy.yml")).unwrap();
        let rest = lines.iter().position(|l| l.2.ends_with("--skip-tags stack")).unwrap();
        assert!(ory < rest, "Ory deploys before the account seed");
        // A joiner's pass 3 turns Spock on, which the stack plays configure: the whole file again.
        let j = phase_lines(&test_site(true), &o);
        let jsb: Vec<&(u32, String, String)> = j.iter().filter(|l| l.2.contains("site-bootstrap.yml")).collect();
        assert!(jsb[0].2.ends_with("--skip-tags after-ory"));
        assert!(!jsb[1].2.contains("--skip-tags"), "{}", jsb[1].2);
    }

    /// The image pull starts after phase 2 and is joined before phase 5, and only
    /// when phase 5 is in the run's range.
    #[test]
    fn the_image_pull_runs_between_phase_2_and_phase_5() {
        let o = Options { env: "e", site: "zagros", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let at = |ev: &[Event]| -> Vec<String> { ev.iter().map(|e| match e {
            Event::Phase(p) => format!("{}", p.n), Event::PullStart { .. } => "pull".into(), Event::PullJoin { at } => format!("join{at}"), _ => "other".into() }).collect() };
        let ev = plan(&repo_root(), &test_site(false), &o, "RESUME");
        let seq = at(&ev);
        let start = seq.iter().position(|x| x == "pull").unwrap();
        let join = seq.iter().position(|x| x == "join5").unwrap();
        assert_eq!(seq[start - 1], "2", "{seq:?}");
        assert_eq!(seq[join + 1], "5", "{seq:?}");
        assert!(seq[start + 1..join].iter().all(|x| x == "3" || x == "4"), "{seq:?}");
        let Event::PullStart { cmd, log, .. } = &ev.iter().find(|e| matches!(e, Event::PullStart { .. })).unwrap() else { unreachable!() };
        assert!(cmd.ends_with("standup-image-pull.yml -e target=zagros_hosts"), "{cmd}");
        assert_eq!(log, "/s/standup-zagros-images.log");
        for (from, to) in [(6, 14), (1, 4)] {
            let seq = at(&plan(&repo_root(), &test_site(false), &Options { from, to, ..o }, "RESUME"));
            assert!(!seq.iter().any(|x| x == "pull" || x == "join5"), "--from {from} --to {to}: {seq:?}");
        }
    }

    /// The two parts phases 5 and 6 select: every play but the untagged pre-flight
    /// gate is `stack` or `after-ory`, the stack plays come first, and the account
    /// seed (the one play that needs Ory) opens the after-ory part.
    #[test]
    fn site_bootstrap_plays_are_tagged_stack_then_after_ory() {
        let text = std::fs::read_to_string(repo_root().join("ops/ansible/site-bootstrap.yml")).unwrap();
        let plays: Vec<serde_yaml::Value> = serde_yaml::from_str(&text).unwrap();
        let tags: Vec<(String, Vec<String>)> = plays.iter().map(|p| (
            p.get("name").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
            p.get("tags").and_then(|t| t.as_sequence()).map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        )).collect();
        assert!(tags[0].1.is_empty(), "the pre-flight gate runs in both phases");
        let part: Vec<&str> = tags[1..].iter().map(|(n, t)| { assert_eq!(t.len(), 1, "{n}: {t:?}"); t[0].as_str() }).collect();
        let first_after = part.iter().position(|t| *t == "after-ory").unwrap();
        assert!(part[..first_after].iter().all(|t| *t == "stack") && part[first_after..].iter().all(|t| *t == "after-ory"), "{part:?}");
        assert!(tags[1 + first_after].0.contains("seed account"), "{}", tags[1 + first_after].0);
        assert!(text.contains("netbird_schema_wait_seconds"), "the schema wait is bounded by a named variable");
    }

    /// A burn-down on the fixture: reverse phase order, every converged record
    /// gains a teardown row, nothing earlier is removed, and a second run
    /// changes nothing (C5).
    #[test]
    fn mock_teardown_appends_in_reverse_and_keeps_history() {
        let mut f = fixture("teardown");
        let up = mock(&mut f, "dev", "alpha", "A Person", MockRun::ALL).unwrap();
        assert_eq!(up.exit, EXIT_OK);
        let before = std::fs::read_to_string(f.root.join("records/dev/alpha-core-1.jsonl")).unwrap();
        let down = teardown_mock(&mut f, "dev", "alpha", MockRun::ALL).unwrap();
        assert_eq!(down.exit, EXIT_OK, "{:?}", down.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        let phases: Vec<&str> = down.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert!(phases.first().unwrap().contains("phase 14"), "{phases:?}");
        assert!(phases.last().unwrap().contains("phase 5"), "{phases:?}");
        let after = std::fs::read_to_string(f.root.join("records/dev/alpha-core-1.jsonl")).unwrap();
        assert!(after.starts_with(&before), "history must be kept");
        assert!(after.lines().last().unwrap().contains("\"action\":\"teardown\""));
        let again = teardown_mock(&mut f, "dev", "alpha", MockRun::ALL).unwrap();
        assert!(again.checks.iter().any(|c| c.detail.contains("0 records appended")), "a second teardown changes nothing");
    }

    #[test]
    fn timing_summary_names_the_slowest_phases_first() {
        let t = vec![(2, "baseline".to_string(), 600), (6, "ory".to_string(), 120), (12, "portal".to_string(), 900)];
        let s = timing_summary(&t, 1620);
        assert!(s.starts_with("total 1620s (27 min) over 3 phases; slowest: 12 portal 900s; 2 baseline 600s"), "{s}");
    }

    fn repo() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let dst = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy_dir(&e.path(), &dst);
            } else {
                std::fs::copy(e.path(), dst).unwrap();
            }
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let src = repo().join("ops/goatbuilder/fixtures/standup");
        let dst = std::env::temp_dir().join(format!("goatbuilder-standup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        copy_dir(&src, &dst);
        Fixture::open(&dst).unwrap()
    }

    /// The rehearsal: every class in phase order, every record verified, exit 0,
    /// and the declared → verified time reported.
    #[test]
    fn mock_standup_converges_every_class_in_phase_order() {
        let mut f = fixture("order");
        let r = mock(&mut f, "dev", "alpha", "A Person", MockRun::ALL).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        let phases: Vec<&str> = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert_eq!(phases.len(), 5);
        assert!(phases[0].starts_with("=== Phase 5 [core]"), "{}", phases[0]);
        assert!(phases[4].starts_with("=== Phase 14 [breakglass]"), "{}", phases[4]);
        // Publications carry the phase order as their serials.
        let pubs = std::fs::read_to_string(f.root.join("declared/publications.jsonl")).unwrap();
        let comps: Vec<String> = pubs.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["component"].as_str().unwrap().to_string()).collect();
        assert_eq!(comps, ["netbird-mgmt", "zenoh-router", "obs-stack", "portal", "breakglass-node"]);
        // The after:2 core machine converged inside the run, on its second observation.
        let core2 = r.checks.iter().find(|c| c.name == "dev/alpha/core/alpha-core-2/netbird-mgmt").unwrap();
        assert!(core2.ok && core2.detail.contains("after 2 observations"), "{}", core2.detail);
        // Records exist only for the class each component belongs to.
        assert!(!f.root.join("records/dev/alpha-obs-1.jsonl").exists() || !std::fs::read_to_string(f.root.join("records/dev/alpha-obs-1.jsonl")).unwrap().contains("netbird-mgmt"));
        assert!(r.checks.last().unwrap().detail.starts_with("declared → verified: 5 components on 4 machines in "));
        // status afterwards: every machine converged for every component its class carries, exit 0.
        let s = crate::verbs::status(&f, "dev", None).unwrap();
        assert_eq!(s.exit, EXIT_OK, "{:?}", s.checks.iter().filter(|c| !c.ok).map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(s.checks.len(), 2 + 2 + 1 + 2 + 1, "one row per machine per component of its class");
    }

    /// C5: a second rehearsal over a converged site writes no new record.
    #[test]
    fn mock_standup_is_idempotent() {
        let mut f = fixture("idem");
        mock(&mut f, "dev", "alpha", "A Person", MockRun::ALL).unwrap();
        let before = std::fs::read_to_string(f.root.join("records/dev/alpha-core-1.jsonl")).unwrap();
        let r = mock(&mut f, "dev", "alpha", "A Person", MockRun::ALL).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        assert_eq!(before, std::fs::read_to_string(f.root.join("records/dev/alpha-core-1.jsonl")).unwrap());
    }

    /// C4: a standup declares every component; without an actor it is
    /// refused before anything is published.
    #[test]
    fn mock_standup_without_an_actor_is_refused() {
        let mut f = fixture("c4");
        let e = mock(&mut f, "dev", "alpha", "", MockRun::ALL).unwrap_err();
        assert!(e.to_string().contains("--ack"), "{e}");
        assert!(!f.root.join("declared/publications.jsonl").exists());
    }

    #[test]
    fn mock_standup_refuses_a_site_the_fixture_does_not_rehearse() {
        let mut f = fixture("site");
        assert!(mock(&mut f, "dev", "bravo", "A Person", MockRun::ALL).is_err());
        assert!(mock(&mut f, "preprod", "alpha", "A Person", MockRun::ALL).is_err());
    }

    /// The real plan for a declared site: the phase-1 gates first, the acceptance
    /// gate at 14, the cross-goatnet join last; nothing executed.
    #[test]
    fn dry_run_plans_the_scripts_phases_in_order() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let o = Options { env: "zagros_prod", site: "zagros-prod", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let r = standup(&repo(), &o).unwrap();
        assert_eq!(r.exit, EXIT_OK);
        let phases: Vec<&str> = r.checks.iter().filter(|c| c.name == "phase").map(|c| c.detail.as_str()).collect();
        assert!(phases[0].starts_with("=== Phase 1 [GATE] config preflight"), "{}", phases[0]);
        assert!(phases.iter().any(|p| p.starts_with("=== Phase 13 [GATE] acceptance suite")));
        assert!(phases.last().unwrap().starts_with("=== Phase 14 [A] join the cross-goatnet surfaces"));
        assert!(phases.iter().any(|p| p.contains("[internet] mesh-enroll")), "zagros-prod is an internet site");
        let ns: Vec<u32> = phases.iter().filter_map(|p| p.split_whitespace().nth(2)).filter_map(|n| n.parse().ok()).collect();
        assert!(ns.windows(2).all(|w| w[0] <= w[1]), "phases are in order: {ns:?}");
        // Contiguous 1..=14: every number appears, none is skipped, none is lettered.
        let mut seen: Vec<u32> = ns.clone();
        seen.dedup();
        assert_eq!(seen, (1..=LAST_PHASE).collect::<Vec<_>>(), "an internet site walks every phase 1..14: {seen:?}");
    }

    /// An obs leg whose play is absent is named before anything runs.
    #[test]
    fn a_missing_obs_play_is_named() {
        let dir = std::env::temp_dir().join(format!("goatbuilder-obs-play-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ops/ansible")).unwrap();
        let why = obs_play_missing(&dir).expect("an absent play is reported");
        assert!(why.contains("utility-host-bootstrap.yml"), "{why}");
        std::fs::write(dir.join("ops/ansible/utility-host-bootstrap.yml"), "---\n").unwrap();
        assert_eq!(obs_play_missing(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The synth guard reads YAML, so it has to agree with what YAML means:
    /// every truthy spelling enables the step, and a comment does not.
    #[test]
    fn the_synth_guard_agrees_with_yaml() {
        let o = Options { env: "e", site: "zagros", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let lines = phase_lines(&test_site(false), &o);
        let line = lines.iter().find(|l| l.2.contains("synth-vendors-deploy.yml")).expect("the synth step is planned");
        assert!(line.2.contains("'^[[:space:]]*synth_vendors_enabled:"), "anchored past comments: {}", line.2);
        assert!(line.2.contains("(true|True|TRUE|yes|Yes|YES|on|On|ON|1)"), "{}", line.2);
        assert!(line.2.contains("cannot read group_vars/"), "an unreadable file fails: {}", line.2);
    }

    /// A play that fails must fail its step, so these two steps may not end in
    /// the `|| echo` that used to report a failed play as "not configured".
    #[test]
    fn optional_plays_do_not_mask_their_own_failure() {
        let o = Options { env: "e", site: "zagros", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        let lines = phase_lines(&test_site(false), &o);
        for play in ["pubsub-state-mirror-deploy.yml", "zagros-zenoh-synth-vendors-deploy.yml"] {
            let line = lines.iter().find(|l| l.2.contains(play)).unwrap_or_else(|| panic!("{play} is in the plan"));
            assert!(line.2.contains("; then ") && line.2.ends_with("; fi"), "{}", line.2);
            assert!(!line.2.contains("|| echo"), "{}", line.2);
        }
    }

    #[test]
    fn undeclared_environment_is_refused() {
        let o = Options { env: "nowhere", site: "zagros-prod", profile: None, from: 1, to: 14, skip_obs: false, obs_only: false, dry_run: true, parallel: false };
        assert!(standup(&repo(), &o).is_err());
    }

    fn goatnet_from_scratch(step: &str, tag: &str) -> Fixture {
        let src = repo().join("ops/goatbuilder/fixtures/goatnet-from-scratch").join(step);
        let dst = std::env::temp_dir().join(format!("goatbuilder-hello-galaxy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dst);
        copy_dir(&src, &dst);
        Fixture::open(&dst).unwrap()
    }

    /// A site as read_site would build it for a LAN run, lite or not, with no
    /// group_vars on disk — the plan is pure in the Site.
    fn lan_site(lite: bool, nb_token_available: bool) -> Site {
        Site {
            repo: "/r".into(), site: "pasture".into(), group: "pasture_hosts".into(), upper: "PASTURE".into(), obs_group: "pasture_obs_hosts".into(),
            state_dir: "/h/.goat/state".into(), ansible_pre: "PRE".into(), inv: INV, profile: "lan".into(), joiner: false,
            join_founder_site: String::new(), join_founder_group: String::new(), acl_site: "pasture".into(), mgmt_fqdn: "mgmt.pasture.lan".into(),
            intra_pair: false, obs: false, obs_why: String::new(), group_vars_exists: false, lite, nb_token_available,
            owner_email: if lite { "a.learner@example.org".to_string() } else { "$OWNER_EMAIL".to_string() },
        }
    }

    fn phases_of(ev: &[Event]) -> Vec<&Phase> {
        ev.iter().filter_map(|e| if let Event::Phase(p) = e { Some(p) } else { None }).collect()
    }

    /// goatlite (ADR 1213): the plan renders the config surface as the FIRST
    /// phase-1 gate, mints the CA when missing in phase 4, and pauses a LAN site
    /// at phase 11 for the bootstrap token — with these exact words, which the
    /// parity gate diffs against goat-standup.sh.
    #[test]
    fn a_goatlite_plan_renders_first_mints_the_ca_and_pauses_for_the_lan_token() {
        let o = Options { env: "pasture", site: "pasture", profile: Some("lan"), from: 1, to: 14, skip_obs: true, obs_only: false, dry_run: true, parallel: false };
        let ev = plan(Path::new("/r"), &lan_site(true, false), &o, "goatbuilder standup pasture --site pasture");
        let ph = phases_of(&ev);
        assert_eq!((ph[0].n, ph[0].kind, ph[0].label.as_str()), (1, "GATE", "goatlite: render the config surface from goatlite.yaml (idempotent; an edited file is reported, never overwritten)"));
        assert_eq!(ph[0].cmd, "python3 '/r/ops/scripts/goatlite-render.py' --site pasture");
        // LP-007: the env-file store is created by the run itself, after the render (it
        // needs the rendered groups) and before the first gate that loads it.
        assert_eq!(ph[1].cmd, "python3 '/r/ops/secrets/posture.py' init --if-env-file --env pasture");
        assert_eq!(ph[2].label, "config preflight (fail-closed)", "the render and the store come before every other gate");
        let mint = ph.iter().find(|p| p.label == "mint leaf certs").unwrap();
        assert_eq!((mint.n, mint.cmd.as_str()), (4, "'/r/ops/scripts/mint-site-certs.sh' pasture --mint-ca-if-missing"));
        let login = ph.iter().find(|p| p.n == 11).expect("a LAN site without the token pauses at phase 11");
        assert_eq!((login.kind, login.label.as_str()), ("MANUAL", "[lan] first login, admin-promote, the bootstrap NetBird token"));
        assert_eq!(login.cmd, "handbook §'Phase 11' has the exact block; on a LAN site the dashboard is the dashboard.* name under this site's zone (site_dns_zone in group_vars/pasture_hosts/vars.yml), resolved by the host's LAN CoreDNS: point the laptop's DNS at the host or pin the names in /etc/hosts. Read the one-time password from /opt/mgmt-host/.bootstrap-passwords/a.learner@example.org.txt on the host, log in; re-run the phase-10 LAN service line ONCE (promotes you to admin — the UI 403s until then); create service user svc-pasture-netbird-infra (Team > Service Users, role admin), create its token, and store it as PASTURE_NB_API_TOKEN under the declared secrets posture (posture.py hint PASTURE_NB_API_TOKEN says where). THEN resume: goatbuilder standup pasture --site pasture --from 12");
        // It sits where the internet branch pauses: after the phase-10 LAN surface, before phase 12.
        let i_surface = ph.iter().position(|p| p.label.starts_with("[lan] LAN service surface")).unwrap();
        let i_routes = ph.iter().position(|p| p.label.starts_with("[lan] TLS routes")).unwrap();
        let i10 = ph.iter().position(|p| p.label == "[lan] dashboard").unwrap();
        assert!(i_surface < i_routes && i_routes < i10, "the dashboard comes after DNS and its routes");
        let i11 = ph.iter().position(|p| p.n == 11).unwrap();
        assert!(i11 == i10 + 1 && ph[i11 + 1].n == 12, "{:?}", ph.iter().map(|p| (p.n, p.label.as_str())).collect::<Vec<_>>());
        let ns: Vec<u32> = ph.iter().map(|p| p.n).collect();
        assert!(ns.windows(2).all(|w| w[0] <= w[1]), "phases stay in order: {ns:?}");
        assert!(!ph.iter().any(|p| p.label.starts_with("[internet]")), "a lite site is a LAN site");
    }

    /// Not lite: no render gate, phase 4 as before; a LAN site pauses at 11 only
    /// while the token is missing, and never when it is available.
    #[test]
    fn a_lan_site_that_is_not_lite_keeps_its_plan_and_pauses_only_without_the_token() {
        let o = Options { env: "pasture", site: "pasture", profile: Some("lan"), from: 1, to: 14, skip_obs: true, obs_only: false, dry_run: true, parallel: false };
        let ev = plan(Path::new("/r"), &lan_site(false, false), &o, "RESUME");
        let ph = phases_of(&ev);
        assert_eq!(ph[0].label, "config preflight (fail-closed)");
        assert!(!ph.iter().any(|p| p.label.starts_with("goatlite:")));
        assert_eq!(ph.iter().find(|p| p.label == "mint leaf certs").unwrap().cmd, "'/r/ops/scripts/mint-site-certs.sh' pasture");
        assert!(ph.iter().any(|p| p.n == 11 && p.kind == "MANUAL" && p.label.starts_with("[lan] first login")));
        let ev = plan(Path::new("/r"), &lan_site(false, true), &o, "RESUME");
        assert!(!phases_of(&ev).iter().any(|p| p.n == 11), "the token is available: no pause");
        let ev = plan(Path::new("/r"), &lan_site(true, true), &o, "RESUME");
        assert!(!phases_of(&ev).iter().any(|p| p.n == 11), "lite with the token available: no pause either");
    }

    /// Tutorial 1's rehearsal: a new goatnet's founder site converges and
    /// `status` agrees afterwards.
    #[test]
    fn goatnet_from_scratch_tutorial_1_founder_converges() {
        let mut f = goatnet_from_scratch("01-founder", "t1");
        let r = mock(&mut f, "zagros", "alpha", "A Platform Admin", MockRun::ALL).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        let s = crate::verbs::status(&f, "zagros", None).unwrap();
        assert_eq!(s.exit, EXIT_OK, "{:?}", s.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
    }

    /// goatlite's rehearsal (ADR 1213, goatlite.md §10 phase A): the four-line
    /// config derives environment `pasture`, site `pasture`, class `core`; every
    /// tier-1 component converges on the one machine and `status` agrees.
    #[test]
    fn goatnet_from_scratch_00_goatlite_mock_converges() {
        let mut f = goatnet_from_scratch("00-goatlite", "t0");
        assert!(f.config.lite.is_some() && f.config.environments.len() == 1, "the environment is derived, not declared");
        let r = mock(&mut f, "pasture", "pasture", "A Learner", MockRun::ALL).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        for c in ["netbird-mgmt", "ory", "zenoh-router", "spire", "portal", "grant-keyd"] {
            let row = r.checks.iter().find(|x| x.name == format!("pasture/pasture/core/pasture-core-1/{c}")).unwrap_or_else(|| panic!("no row for {c}"));
            assert!(row.ok && row.detail.starts_with("converged on"), "{c}: {}", row.detail);
        }
        assert!(r.checks.last().unwrap().detail.starts_with("declared → verified: 6 components on 1 machine in "), "{}", r.checks.last().unwrap().detail);
        let s = crate::verbs::status(&f, "pasture", None).unwrap();
        assert_eq!(s.exit, EXIT_OK, "{:?}", s.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        assert_eq!(s.checks.len(), 6, "one row per component on the one machine");
    }

    /// Tutorial 2's rehearsal starts from tutorial 1's end state: the joiner
    /// converges, the founder-side mirror is recorded on the FOUNDER's machines,
    /// publication serials continue tutorial 1's, and `status` expects the
    /// carried founder records as well as the joiner's.
    #[test]
    fn goatnet_from_scratch_tutorial_2_joiner_converges_and_records_the_mirror_on_the_founder() {
        let mut f = goatnet_from_scratch("02-first-federated-node", "t2");
        let r = mock(&mut f, "zagros", "beta", "A Platform Admin", MockRun::ALL).unwrap();
        assert_eq!(r.exit, EXIT_OK, "{:?}", r.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        assert!(r.checks.iter().any(|c| c.name == "zagros/beta/alpha/core/alpha-core-1/spock-mesh-wire" && c.ok));
        assert!(r.checks.iter().any(|c| c.detail.starts_with("serial 6:")), "serials must continue tutorial 1's");
        let s = crate::verbs::status(&f, "zagros", None).unwrap();
        assert_eq!(s.exit, EXIT_OK, "{:?}", s.checks.iter().filter(|c| !c.ok).map(|c| &c.detail).collect::<Vec<_>>());
        assert!(s.checks.iter().any(|c| c.name == "zagros/alpha-obs-1/obs-stack"), "the carried founder records stay in scope");
    }

    /// A standup touches only the founder it attaches to: naming any other
    /// site's class is refused before anything is published.
    #[test]
    fn a_phase_naming_a_site_this_one_does_not_join_is_refused() {
        let mut f = goatnet_from_scratch("02-first-federated-node", "refuse");
        let p = f.root.join("converger.yaml");
        let text = std::fs::read_to_string(&p).unwrap().replace("alpha/core", "gamma/core");
        std::fs::write(&p, text).unwrap();
        let e = mock(&mut f, "zagros", "beta", "A Platform Admin", MockRun::ALL).unwrap_err().to_string();
        assert!(e.contains("names site 'gamma'") && e.contains("joins 'alpha'"), "{e}");
        assert!(!f.root.join("declared/publications.jsonl").exists() || std::fs::read_to_string(f.root.join("declared/publications.jsonl")).unwrap().lines().count() == 5, "nothing published");
    }
}
