//! Phase 10's admission on a core deployed by the goat-zenoh role
//! (docs/design/goatbuilder.md, "Admission on a goat-zenoh role core").
//!
//! A core the goat-zenoh role deploys is already declared: an inventory group
//! of the site inventory whose group_vars configure the role, the play that
//! runs the role (the fleet pin of the component whose image variable those
//! group_vars set, `deploy_playbook:`), and the role's own `site_id`. This
//! finds that declaration, so admission restarts the core by running the
//! role's play on that group with `-e target=<group> -e site_id=<site_id>`
//! (`--check --diff` for the dry run), with no restart command typed for it.
//!
//! Used only when the deployment names no route of its own (FED_CORE_PLAY or
//! FED_CORE_RESTART stay overrides). Nothing found: admission asks for those
//! settings as before. Found but not resolvable (several groups, no
//! `site_id`, no play): refused with GB-236, naming what was looked for and
//! where.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The core router as the goat-zenoh role's declarations give it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleCore {
    /// The inventory group the play targets.
    pub group: String,
    /// The inventory file that defines it, relative to `ops/ansible/`.
    pub inventory: String,
    /// The role's `site_id`.
    pub site_id: String,
    /// The play that runs the role, relative to `ops/ansible/`.
    pub play: String,
    /// How many machines the group holds.
    pub machines: usize,
}

/// Every top-level key the group's group_vars set: group_vars/<group>.yml,
/// .yaml, and each yml/yaml file in group_vars/<group>/.
fn group_keys(repo: &Path, group: &str) -> BTreeSet<String> {
    let gv = repo.join("ops/ansible/group_vars");
    let mut files = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        files.extend(
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml")),
        );
    }
    let mut keys = BTreeSet::new();
    for f in files {
        let Some(doc) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        else {
            continue;
        };
        if let Some(m) = doc.as_mapping() {
            keys.extend(m.keys().filter_map(|k| k.as_str().map(String::from)));
        }
    }
    keys
}

/// The variables the goat-zenoh role defines (its defaults).
fn role_vars(repo: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(repo.join("ops/ansible/roles/goat-zenoh/defaults/main.yml"))
        .ok()
        .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
        .and_then(|d| {
            d.as_mapping().map(|m| {
                m.keys()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The deploy play of the fleet pin whose `var` the group sets, as
/// `goatbuilder deploy` finds it (push::find_play).
fn play_for(repo: &Path, keys: &BTreeSet<String>) -> Option<String> {
    let text = std::fs::read_to_string(repo.join("ops/pins/fleet-pins.yaml")).ok()?;
    let pins: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let ansible = repo.join("ops/ansible");
    let s =
        |v: Option<&serde_yaml::Value>| v.and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let mut found: Vec<PathBuf> = Vec::new();
    for (name, comp) in pins.get("components")?.as_mapping()? {
        let var = s(comp.get("var"));
        if var.is_empty() || !keys.contains(&var) {
            continue;
        }
        let name = name.as_str().unwrap_or_default();
        let ledger = Some(s(comp.get("ledger_component")))
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| name.to_string());
        if let crate::push::Play::Found(p) =
            crate::push::find_play(&ansible, &s(comp.get("deploy_playbook")), &ledger, name)
        {
            if !found.contains(&p) {
                found.push(p);
            }
        }
    }
    match found.as_slice() {
        [one] => one
            .strip_prefix(&ansible)
            .ok()
            .map(|p| p.display().to_string()),
        _ => None,
    }
}

/// The addresses of the hosts of `group` in the peering inventories
/// (`ops/ansible/inventory/peering-*.ini`): the peering's own core.
fn peering_core_addresses(repo: &Path, group: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(rd) = std::fs::read_dir(repo.join("ops/ansible/inventory")) else {
        return out;
    };
    for f in rd.filter_map(|e| e.ok().map(|e| e.path())) {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !(name.starts_with("peering-") && name.ends_with(".ini")) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap_or_default();
        for (g, ms) in super::site_hosts::groups(&text) {
            if g == group {
                out.extend(ms.into_iter().map(|m| m.addr));
            }
        }
    }
    out
}

fn refuse(why: String) -> String {
    format!("our core router's goat-zenoh role declarations do not resolve (GB-236): {why}")
}

/// The goat-zenoh role core of this peering, from the site inventories and
/// their group_vars. `Ok(None)`: no group configures the role, so the
/// deployment's own settings are asked for. `Err`: found but not resolvable,
/// with what was looked for and where (GB-236). `core_group` is the
/// peering's core group, whose address picks one role group among several.
pub fn find(repo: &Path, core_group: &str) -> Result<Option<RoleCore>, String> {
    let role = role_vars(repo);
    if role.is_empty() {
        return Ok(None);
    }
    let looked = "the groups of ops/ansible/inventory/*.ini (not peering-*.ini) whose group_vars set a variable of roles/goat-zenoh/defaults/main.yml";
    // (group, inventory file, its machines' addresses, its group_vars keys)
    let mut cands: Vec<(String, String, BTreeSet<String>, BTreeSet<String>)> = Vec::new();
    for (file, text) in super::site_hosts::site_inventories(repo) {
        for (g, ms) in super::site_hosts::groups(&text) {
            if ms.is_empty() || cands.iter().any(|(c, ..)| *c == g) {
                continue;
            }
            let keys = group_keys(repo, &g);
            if keys.iter().any(|k| role.contains(k)) {
                let rel = file.trim_start_matches("ops/ansible/").to_string();
                cands.push((g, rel, ms.into_iter().map(|m| m.addr).collect(), keys));
            }
        }
    }
    if cands.len() > 1 {
        let all = cands
            .iter()
            .map(|(g, f, ..)| format!("[{g}] in {f}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ours = peering_core_addresses(repo, core_group);
        cands.retain(|(_, _, addrs, _)| addrs.iter().any(|a| ours.contains(a)));
        if cands.len() != 1 {
            return Err(refuse(format!(
                "{all} configure the goat-zenoh role, and the address of the peering's core [{core_group}] in ops/ansible/inventory/peering-*.ini picks {} of them. Looked in {looked}",
                if cands.is_empty() { "none".to_string() } else { cands.len().to_string() }
            )));
        }
    }
    let Some((group, inventory, addrs, keys)) = cands.pop() else {
        return Ok(None);
    };
    let site_id = super::preflight::group_var(repo, &group, "site_id")
        .or_else(|| group.strip_suffix("_hosts").map(String::from))
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        });
    let Some(site_id) = site_id else {
        return Err(refuse(format!(
            "[{group}] in {inventory} configures the role, but nothing gives its site_id: declare site_id in group_vars/{group}/ (the role's mandatory variable), or name the group <site>_hosts"
        )));
    };
    let Some(play) = play_for(repo, &keys) else {
        return Err(refuse(format!(
            "[{group}] in {inventory} configures the role, but no single fleet pin in ops/pins/fleet-pins.yaml whose var group_vars/{group}/ sets names a deploy play that exists (deploy_playbook:, else <ledger>-deploy.yml or <component>-deploy.yml)"
        )));
    };
    Ok(Some(RoleCore {
        group,
        inventory,
        site_id,
        play,
        machines: addrs.len(),
    }))
}

impl RoleCore {
    /// The role's play on the core group, from `ops/ansible`, as admission's
    /// dry run (`check`) or restart.
    pub fn play_cmd(&self, repo: &Path, check: bool) -> String {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        format!(
            "cd {} && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {} -- ../scripts/ansible-preflight.sh -i {} -e target={} -e site_id={} {}{}",
            q(&repo.join("ops/ansible").display().to_string()),
            q(&crate::settings::aws_profile(repo)),
            q(&self.inventory),
            q(&self.group),
            q(&self.site_id),
            q(&self.play),
            if check { " --check --diff" } else { "" }
        )
    }

    /// A variable as the role resolves it for this group: the group's own
    /// plain value, else the role's default, with `{{ site_id }}` and
    /// `{{ zenoh_stack }}` put in. Anything else templated is not guessed.
    pub fn role_value(&self, repo: &Path, key: &str) -> Result<String, String> {
        let default = |k: &str| -> Option<String> {
            let t = std::fs::read_to_string(
                repo.join("ops/ansible/roles/goat-zenoh/defaults/main.yml"),
            )
            .ok()?;
            let d: serde_yaml::Value = serde_yaml::from_str(&t).ok()?;
            d.get(k)?.as_str().map(String::from)
        };
        // A value the group sets is its own, plain or not: a templated one is
        // not replaced by the role's default.
        let keys = group_keys(repo, &self.group);
        let own = |k: &str| match keys.contains(k) {
            true => Some(
                super::preflight::group_var(repo, &self.group, k)
                    .unwrap_or_else(|| "{{ templated }}".into()),
            ),
            false => default(k),
        };
        let stack = own("zenoh_stack")
            .unwrap_or_default()
            .replace("{{ site_id }}", &self.site_id);
        let v = own(key)
            .unwrap_or_default()
            .replace("{{ zenoh_stack }}", &stack)
            .replace("{{ site_id }}", &self.site_id);
        if v.is_empty() || v.contains("{{") || v.contains('\'') {
            return Err(format!(
                "our core router's goat-zenoh role declarations do not resolve (GB-236): {key} for [{}] is not a plain value (group_vars/{}/, else roles/goat-zenoh/defaults/main.yml, with site_id {} and zenoh_stack put in): '{v}'",
                self.group, self.group, self.site_id
            ));
        }
        Ok(v)
    }

    /// One Ansible module call on the group's machine, as the play reaches it.
    fn module_cmd(&self, repo: &Path, module: &str, args: &str) -> String {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        format!(
            "cd {} && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws {} -- ansible {} -i {} -b -m {} -a {}",
            q(&repo.join("ops/ansible").display().to_string()),
            q(&crate::settings::aws_profile(repo)),
            q(&self.group),
            q(&self.inventory),
            module,
            q(args)
        )
    }

    /// Copy `remote` on the core router to `local` on this machine.
    pub fn fetch_cmd(&self, repo: &Path, remote: &str, local: &Path) -> String {
        self.module_cmd(
            repo,
            "ansible.builtin.fetch",
            &format!("src={remote} dest={} flat=yes", local.display()),
        )
    }

    /// Copy `local` from this machine to `remote` on the core router.
    pub fn place_cmd(&self, repo: &Path, local: &Path, remote: &str) -> String {
        self.module_cmd(
            repo,
            "ansible.builtin.copy",
            &format!("src={} dest={remote}", local.display()),
        )
    }

    pub fn route(&self) -> String {
        format!(
            "the goat-zenoh role's play {} on [{}] in {} (site_id {})",
            self.play, self.group, self.inventory, self.site_id
        )
    }
}

/// The CN the partner's edge presents, from the subjects of the agreement's
/// access-control fragment (`cert_common_names`). The fragment also names our
/// own outbound republisher (`fed.<us>.<them>.republisher.01`,
/// `compose::republisher_cn`), which is not the partner; it is left out. When
/// the agreement names the partner's edge leaf (`parties[].peering_cn` of the
/// partner, `agreed`), that is the CN, and the fragment must name it as its
/// one partner subject. Without it, the fragment's one partner subject is.
/// Refused when the fragment names no partner subject, or several.
pub fn fragment_cn(fragment: &str, agreed: Option<&str>) -> Result<String, String> {
    let re = regex::Regex::new(r#"cert_common_names\s*:\s*\[([^\]]*)\]"#).expect("regex");
    let q = regex::Regex::new(r#""([^"]+)""#).expect("regex");
    let mut cns: Vec<String> = Vec::new();
    for c in re.captures_iter(fragment) {
        for n in q.captures_iter(&c[1]) {
            if !cns.contains(&n[1].to_string()) {
                cns.push(n[1].to_string());
            }
        }
    }
    let partner: Vec<&String> = cns
        .iter()
        .filter(|c| !c.contains(".republisher."))
        .collect();
    match (partner.as_slice(), agreed) {
        (_, Some(a)) if partner.len() == 1 && partner[0] == a => Ok(a.to_string()),
        (_, Some(a)) if partner.iter().any(|c| c.as_str() == a) => Err(format!(
            "the fragment names several partner subject CNs: {}",
            partner.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ")
        )),
        (_, Some(a)) if !cns.is_empty() => Err(format!(
            "the fragment names no subject with the CN the agreement gives the partner's edge ({a}); it names {}",
            cns.join(", ")
        )),
        ([one], None) => Ok((*one).clone()),
        ([], _) => Err("the fragment names no partner subject CN (cert_common_names)".into()),
        (many, _) => Err(format!(
            "the fragment names several partner subject CNs: {}",
            many.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The file admission writes the core's admission of the partner into, in
/// the goat-zenoh role's own declarations: `group_vars/<group>/peerings.yml`.
pub fn peerings_file(repo: &Path, group: &str) -> PathBuf {
    repo.join("ops/ansible/group_vars")
        .join(group)
        .join("peerings.yml")
}

/// Whether a value holds a Jinja marker (`{{`, `{%` or `{#`): Ansible would
/// render it, so its literal text is not what the role reads.
fn is_templated(v: &str) -> bool {
    ["{{", "{%", "{#"].iter().any(|m| v.contains(m))
}

/// `zenoh_sovereign_peerings` with this partner's entry (`sovereign`, `cn`)
/// merged into what `group_vars/<group>/peerings.yml` holds: another
/// partner's entry is kept as it is, and this partner's other keys are kept.
/// Refused (GB-237) when another of the group's files sets the variable,
/// because it would then have two owners, and when an entry's `sovereign` is
/// a template, which no literal id matches.
pub fn merge_peerings(repo: &Path, group: &str, them: &str, cn: &str) -> Result<String, String> {
    let file = peerings_file(repo, group);
    let gv = repo.join("ops/ansible/group_vars");
    let mut others: Vec<PathBuf> = vec![
        gv.join(format!("{group}.yml")),
        gv.join(format!("{group}.yaml")),
    ];
    if let Ok(rd) = std::fs::read_dir(gv.join(group)) {
        others.extend(rd.filter_map(|e| e.ok().map(|e| e.path())));
    }
    for f in others.iter().filter(|f| **f != file) {
        let set = std::fs::read_to_string(f)
            .ok()
            .and_then(|t| serde_yaml::from_str::<serde_yaml::Value>(&t).ok())
            .is_some_and(|d| d.get("zenoh_sovereign_peerings").is_some());
        if set {
            return Err(format!(
                "zenoh_sovereign_peerings is set in {}, and admission writes it in {} (GB-237): move the entries into {} and remove the variable from the other file",
                f.strip_prefix(repo).unwrap_or(f).display(),
                file.strip_prefix(repo).unwrap_or(&file).display(),
                file.strip_prefix(repo).unwrap_or(&file).display()
            ));
        }
    }
    // The comment block at the top of the file, kept verbatim on the rewrite.
    let mut leading: Vec<String> = Vec::new();
    let doc: serde_yaml::Value = match std::fs::read_to_string(&file) {
        Ok(t) => {
            // A rewrite through serde_yaml cannot keep a comment. The leading block is
            // carried over as text; one anywhere else is refused by line, not dropped.
            leading = leading_comments(&t);
            if let Some(n) = foreign_comment(&t) {
                return Err(format!(
                    "{}: line {n} holds a comment, and admission rewrites this file from its parsed values, which cannot keep comments (GB-237): move the comment out of the file (or into the role's documentation), then make a new plan and deploy. Nothing was written",
                    file.strip_prefix(repo).unwrap_or(&file).display()
                ));
            }
            serde_yaml::from_str(&t)
                .map_err(|e| format!("{} is not YAML ({e}) (GB-237)", file.display()))?
        }
        Err(_) => serde_yaml::Value::Null,
    };
    // Every key but the list is written back as it was read.
    let mut top = match &doc {
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        serde_yaml::Value::Mapping(m) => m.clone(),
        _ => {
            return Err(format!(
                "{} is not a mapping of variables (GB-237): it is the core router group's group_vars file, so write it as `variable: value` lines. Nothing was written",
                file.strip_prefix(repo).unwrap_or(&file).display()
            ))
        }
    };
    let shown = file
        .strip_prefix(repo)
        .unwrap_or(&file)
        .display()
        .to_string();
    // What the file holds is kept, so it must be read whole: a value that is
    // not a list (a template that renders one, a mapping, a scalar) would read
    // as empty, and the file written back would hold this partner alone.
    let mut list: Vec<serde_yaml::Value> = match doc.get("zenoh_sovereign_peerings") {
        None | Some(serde_yaml::Value::Null) => Vec::new(),
        Some(serde_yaml::Value::Sequence(l)) => l.clone(),
        Some(other) => {
            let what = match other {
                serde_yaml::Value::String(t) if is_templated(t) => format!("the template {t:?}"),
                serde_yaml::Value::Mapping(_) => "a mapping".to_string(),
                _ => "not a list".to_string(),
            };
            return Err(format!(
                "{shown} sets zenoh_sovereign_peerings to {what} (GB-237): admission keeps every other partner's entry and must read them, so write the variable as a literal list of entries (sovereign, cn). Nothing was written"
            ));
        }
    };
    // An entry is found by its literal `sovereign`. One that is not a mapping
    // with a literal sovereign id would never match, so this partner would be
    // appended a second time.
    for (i, e) in list.iter().enumerate() {
        let sovereign = e.get("sovereign").and_then(|v| v.as_str());
        let why = match (e.as_mapping(), sovereign) {
            (None, _) => Some("is not a mapping of sovereign and cn".to_string()),
            (Some(_), None) => Some("has no literal sovereign".to_string()),
            (Some(_), Some(t)) if is_templated(t) => {
                Some(format!("has the sovereign template {t:?}"))
            }
            _ => None,
        };
        if let Some(why) = why {
            return Err(format!(
                "{shown}: entry {} of zenoh_sovereign_peerings {why} (GB-237): admission finds a partner's entry by its literal sovereign id, so write that entry's sovereign as the partner's id. Nothing was written",
                i + 1
            ));
        }
    }
    let key = |k: &str| serde_yaml::Value::String(k.into());
    match list
        .iter_mut()
        .find(|e| e.get("sovereign").and_then(|v| v.as_str()) == Some(them))
    {
        Some(e) => {
            if let Some(m) = e.as_mapping_mut() {
                m.insert(key("cn"), key(cn));
            }
        }
        None => {
            let mut m = serde_yaml::Mapping::new();
            m.insert(key("sovereign"), key(them));
            m.insert(key("cn"), key(cn));
            list.push(serde_yaml::Value::Mapping(m));
        }
    }
    // In place: the list keeps its position among the file's other keys.
    top.insert(
        key("zenoh_sovereign_peerings"),
        serde_yaml::Value::Sequence(list),
    );
    let body =
        serde_yaml::to_string(&serde_yaml::Value::Mapping(top)).map_err(|e| e.to_string())?;
    let kept: String = leading.iter().map(|l| format!("{l}\n")).collect();
    Ok(format!("{kept}{}{body}", PEERINGS_HEADER.join("")))
}

/// The comment admission writes at the top of peerings.yml, one line per item.
const PEERINGS_HEADER: [&str; 2] = [
    "# Each partner this core admits (the goat-zenoh role's zenoh_sovereign_peerings),\n",
    "# written by phase 10's admission from the agreement's access-control fragment.\n",
];

/// The lines before the first YAML line, comments and blank lines only, without
/// admission's own header lines: the block a rewrite keeps verbatim.
fn leading_comments(text: &str) -> Vec<String> {
    let header: Vec<&str> = PEERINGS_HEADER.iter().map(|l| l.trim_end()).collect();
    text.lines()
        .take_while(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
        .filter(|l| !header.contains(l) && !l.trim().is_empty())
        .map(String::from)
        .collect()
}

/// The 1-based line of the first comment in `text` after its leading block: a
/// `#` at the start of a line or after whitespace, outside quotes.
fn foreign_comment(text: &str) -> Option<usize> {
    let skip = text
        .lines()
        .take_while(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
        .count();
    for (i, line) in text.lines().enumerate().skip(skip) {
        let (mut single, mut double, mut prev_space) = (false, false, true);
        for c in line.chars() {
            match c {
                '\'' if !double => single = !single,
                '"' if !single => double = !double,
                '#' if !single && !double && prev_space => return Some(i + 1),
                _ => {}
            }
            prev_space = c.is_whitespace();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let base: &[(&str, &str)] = &[
            ("ops/ansible/roles/goat-zenoh/defaults/main.yml", "zenoh_router_image: \"x\"\nzenoh_sovereign_peerings: []\n"),
            ("ops/ansible/zenoh-deploy.yml", "- hosts: \"{{ target | mandatory }}\"\n"),
            ("ops/pins/fleet-pins.yaml", "components:\n  zenoh-router:\n    var: zenoh_router_image\n    ledger_component: zagros-zenoh\n    deploy_playbook: zenoh-deploy.yml\n  other:\n    var: other_image\n"),
        ];
        for (rel, text) in base.iter().chain(files.iter()) {
            let f = d.path().join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        d
    }

    const INV: (&str, &str) = (
        "ops/ansible/inventory/core-router.ini",
        "[core_router]\nzagros-core ansible_host=192.0.2.10 ansible_user=goat\n",
    );
    const GV: (&str, &str) = (
        "ops/ansible/group_vars/core_router/zenoh.yml",
        "zenoh_router_image: \"ghcr.io/example-org/zenoh@sha256:00\"\nsite_id: zagros\n",
    );

    #[test]
    fn the_role_core_comes_from_the_group_whose_vars_configure_the_role() {
        let r = checkout(&[
            INV,
            GV,
            (
                "ops/ansible/inventory/zagros-site.ini",
                "[edge]\nzagros-edge ansible_host=192.0.2.20\n",
            ),
        ]);
        let c = find(r.path(), "xab_core").unwrap().unwrap();
        assert_eq!(
            c,
            RoleCore {
                group: "core_router".into(),
                inventory: "inventory/core-router.ini".into(),
                site_id: "zagros".into(),
                play: "zenoh-deploy.yml".into(),
                machines: 1,
            }
        );
        let cmd = c.play_cmd(r.path(), true);
        assert!(cmd.contains("-i 'inventory/core-router.ini' -e target='core_router' -e site_id='zagros' 'zenoh-deploy.yml' --check --diff"), "{cmd}");
        assert!(!c.play_cmd(r.path(), false).contains("--check"));
    }

    #[test]
    fn the_org_fragment_path_is_the_roles_with_site_id_and_stack_put_in() {
        let role = ("ops/ansible/roles/goat-zenoh/defaults/main.yml", "zenoh_router_image: x\nzenoh_stack: \"{{ site_id }}-zenoh\"   # a comment\ngoat_zenoh_acl_org_path: \"/etc/{{ zenoh_stack }}/acl-org-fragment.json5\"\n");
        let r = checkout(&[INV, GV, role]);
        let c = find(r.path(), "xab_core").unwrap().unwrap();
        assert_eq!(c.machines, 1);
        assert_eq!(
            c.role_value(r.path(), "goat_zenoh_acl_org_path").unwrap(),
            "/etc/zagros-zenoh/acl-org-fragment.json5"
        );
        let fetch = c.fetch_cmd(
            r.path(),
            "/etc/zagros-zenoh/acl-org-fragment.json5",
            Path::new("/w/fetched/org.json5"),
        );
        assert!(fetch.contains("ansible 'core_router' -i 'inventory/core-router.ini' -b -m ansible.builtin.fetch -a 'src=/etc/zagros-zenoh/acl-org-fragment.json5 dest=/w/fetched/org.json5 flat=yes'"), "{fetch}");
        let place = c.place_cmd(
            r.path(),
            Path::new("/w/staged/org.json5"),
            "/etc/zagros-zenoh/acl-org-fragment.json5",
        );
        assert!(place.contains("-m ansible.builtin.copy -a 'src=/w/staged/org.json5 dest=/etc/zagros-zenoh/acl-org-fragment.json5'"), "{place}");
        // The group's own value wins; a template the role does not resolve here is refused.
        let gv = ("ops/ansible/group_vars/core_router/zenoh.yml", "zenoh_router_image: x\nsite_id: zagros\ngoat_zenoh_acl_org_path: /srv/zenoh/org.json5\nzenoh_stack: \"{{ other }}\"\n");
        let r = checkout(&[INV, gv, role]);
        let c = find(r.path(), "xab_core").unwrap().unwrap();
        assert_eq!(
            c.role_value(r.path(), "goat_zenoh_acl_org_path").unwrap(),
            "/srv/zenoh/org.json5"
        );
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_router_image: x\nsite_id: zagros\nzenoh_stack: \"{{ other }}\"\n",
            ),
            role,
        ]);
        let c = find(r.path(), "xab_core").unwrap().unwrap();
        assert!(c
            .role_value(r.path(), "goat_zenoh_acl_org_path")
            .unwrap_err()
            .contains("(GB-236)"));
    }

    const FRAG: &str = "##ACL-ORG-REGION subjects\n      {\n        id: \"fed-xab\",\n        cert_common_names: [\"fed.xab.zagros.01\"],\n      },\n##ACL-ORG-REGION policies\n";

    /// The fragment apply renders names the partner's edge and our own
    /// outbound republisher. The CN admitted is the one the agreement gives
    /// the partner's edge, whatever the order of the subjects.
    #[test]
    fn the_partner_cn_is_the_agreed_one_beside_our_republisher() {
        let both = |first: &str, second: &str| {
            format!("##ACL-ORG-REGION subjects\n  {{ id: \"a\", cert_common_names: [\"{first}\"] }},\n  {{ id: \"b\", cert_common_names: [\"{second}\"] }},\n")
        };
        let (edge, ours) = ("fed.xab.xaa.01", "fed.xaa.xab.republisher.01");
        for f in [both(edge, ours), both(ours, edge)] {
            assert_eq!(fragment_cn(&f, Some(edge)).unwrap(), edge, "{f}");
            assert_eq!(fragment_cn(&f, None).unwrap(), edge, "{f}");
        }
        // Two partner candidates are still refused.
        let two = both(edge, "fed.xab.xaa.02");
        assert!(fragment_cn(&two, Some(edge))
            .unwrap_err()
            .contains("several partner"));
        assert!(fragment_cn(&two, None)
            .unwrap_err()
            .contains("several partner"));
        // A fragment without the agreed CN is refused, naming what it has.
        let other = both("fed.xac.xaa.01", ours);
        assert!(fragment_cn(&other, Some(edge))
            .unwrap_err()
            .contains("fed.xac.xaa.01"));
    }

    #[test]
    fn the_partner_entry_is_merged_from_the_fragments_subject() {
        assert_eq!(fragment_cn(FRAG, None).unwrap(), "fed.xab.zagros.01");
        assert_eq!(
            fragment_cn(FRAG, Some("fed.xab.zagros.01")).unwrap(),
            "fed.xab.zagros.01"
        );
        assert!(fragment_cn("no subjects", None).is_err());
        assert!(fragment_cn("cert_common_names: [\"a\", \"b\"]", None)
            .unwrap_err()
            .contains("several"));
        // A new file: this partner's entry alone.
        let r = checkout(&[INV, GV]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        assert_eq!(v["zenoh_sovereign_peerings"][0]["sovereign"], "xab");
        assert_eq!(v["zenoh_sovereign_peerings"][0]["cn"], "fed.xab.zagros.01");
        // Another partner kept, this partner's other keys kept, its CN replaced.
        let r = checkout(&[INV, GV, ("ops/ansible/group_vars/core_router/peerings.yml",
            "zenoh_sovereign_peerings:\n- sovereign: xac\n  cn: fed.xac.zagros.01\n- sovereign: xab\n  cn: old\n  release_key_exprs: [from/zagros/REL-ALL/**]\n")]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        let l = v["zenoh_sovereign_peerings"].as_sequence().unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0]["sovereign"], "xac");
        assert_eq!(
            (l[1]["cn"].as_str(), l[1]["release_key_exprs"][0].as_str()),
            (Some("fed.xab.zagros.01"), Some("from/zagros/REL-ALL/**"))
        );
        assert_eq!(
            merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01")
                .unwrap()
                .matches("sovereign: xab")
                .count(),
            1
        );
        // Set in another of the group's files: two owners, refused.
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_router_image: x\nsite_id: zagros\nzenoh_sovereign_peerings: []\n",
            ),
        ]);
        assert!(merge_peerings(r.path(), "core_router", "xab", "c")
            .unwrap_err()
            .contains("(GB-237)"));
    }

    #[test]
    fn an_entry_whose_sovereign_is_templated_is_refused_not_appended_beside() {
        // The entry is found by its literal `sovereign`; a templated one would
        // never match, and a second entry for the same partner would be added.
        for templated in [
            "\"{{ partner_id }}\"",
            "\"{% if x %}xab{% endif %}\"",
            "\"xab{# the partner #}\"",
        ] {
            let r = checkout(&[
                INV,
                GV,
                (
                    "ops/ansible/group_vars/core_router/peerings.yml",
                    &format!("zenoh_sovereign_peerings:\n- sovereign: {templated}\n  cn: fed.xab.zagros.01\n"),
                ),
            ]);
            let e =
                merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap_err();
            assert!(
                e.contains("(GB-237)") && e.contains("literal sovereign id"),
                "{templated}: {e}"
            );
        }
        // The whole list as a template, a mapping or a scalar: refused, not
        // read as empty (which would write this partner alone, dropping the rest).
        for (value, said) in [
            ("\"{{ partners | to_entries }}\"", "the template"),
            ("{xac: {cn: fed.xac.zagros.01}}", "a mapping"),
            ("7", "not a list"),
        ] {
            let r = checkout(&[
                INV,
                GV,
                (
                    "ops/ansible/group_vars/core_router/peerings.yml",
                    &format!("zenoh_sovereign_peerings: {value}\n"),
                ),
            ]);
            let before = std::fs::read_to_string(
                r.path()
                    .join("ops/ansible/group_vars/core_router/peerings.yml"),
            )
            .unwrap();
            let e =
                merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap_err();
            assert!(
                e.contains("(GB-237)") && e.contains(said) && e.contains("Nothing was written"),
                "{value}: {e}"
            );
            assert_eq!(
                std::fs::read_to_string(
                    r.path()
                        .join("ops/ansible/group_vars/core_router/peerings.yml")
                )
                .unwrap(),
                before
            );
        }
        // Another partner's templated entry is named by its position, not as this partner.
        let r = checkout(&[
            INV,
            GV,
            (
                "ops/ansible/group_vars/core_router/peerings.yml",
                "zenoh_sovereign_peerings:\n- sovereign: xac\n  cn: c\n- sovereign: \"{{ other }}\"\n  cn: d\n",
            ),
        ]);
        let e = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap_err();
        assert!(
            e.contains("entry 2") && !e.contains("sovereign: xab"),
            "{e}"
        );
        // A literal entry is matched, as before.
        let r = checkout(&[
            INV,
            GV,
            (
                "ops/ansible/group_vars/core_router/peerings.yml",
                "zenoh_sovereign_peerings:\n- sovereign: xab\n  cn: old\n",
            ),
        ]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        assert_eq!(text.matches("sovereign: xab").count(), 1, "{text}");
    }

    #[test]
    fn no_group_configuring_the_role_finds_nothing() {
        let r = checkout(&[INV]);
        assert_eq!(find(r.path(), "xab_core").unwrap(), None);
    }

    #[test]
    fn site_id_from_a_site_hosts_group_name_else_refused() {
        let r = checkout(&[
            (
                "ops/ansible/inventory/mgmt.ini",
                "[zagros_hosts]\nh1 ansible_host=192.0.2.10\n",
            ),
            (
                "ops/ansible/group_vars/zagros_hosts/zenoh.yml",
                "zenoh_router_image: x\n",
            ),
        ]);
        assert_eq!(
            find(r.path(), "xab_core").unwrap().unwrap().site_id,
            "zagros"
        );
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_router_image: x\n",
            ),
        ]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(
            e.contains("(GB-236)") && e.contains("declare site_id in group_vars/core_router/"),
            "{e}"
        );
    }

    #[test]
    fn several_role_groups_are_narrowed_by_the_peerings_core_address_or_refused() {
        let two = ("ops/ansible/inventory/core-router.ini", "[core_router]\nzagros-core ansible_host=192.0.2.10\n[spare_router]\nspare ansible_host=192.0.2.11\n");
        let gv2 = (
            "ops/ansible/group_vars/spare_router.yml",
            "zenoh_router_image: y\nsite_id: spare\n",
        );
        let r = checkout(&[two, GV, gv2]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(
            e.contains("(GB-236)")
                && e.contains("[core_router] in inventory/core-router.ini")
                && e.contains("[spare_router]"),
            "{e}"
        );
        let r = checkout(&[
            two,
            GV,
            gv2,
            (
                "ops/ansible/inventory/peering-xab_edge.ini",
                "[xab_core]\nzagros-core-xab ansible_host=192.0.2.10\n",
            ),
        ]);
        assert_eq!(
            find(r.path(), "xab_core").unwrap().unwrap().group,
            "core_router"
        );
    }

    #[test]
    fn no_deploy_play_for_the_groups_image_variable_is_refused() {
        let r = checkout(&[
            INV,
            (
                "ops/ansible/group_vars/core_router/zenoh.yml",
                "zenoh_sovereign_peerings: []\nsite_id: zagros\n",
            ),
        ]);
        let e = find(r.path(), "xab_core").unwrap_err();
        assert!(e.contains("(GB-236)") && e.contains("fleet pin"), "{e}");
    }

    /// peerings.yml may hold more than the list. Admission changes only
    /// `zenoh_sovereign_peerings`; every other key and value stays as it was.
    #[test]
    fn admission_keeps_every_other_key_in_peerings_yml() {
        let body = "zenoh_peering_note: kept\nzenoh_sovereign_peerings:\n- sovereign: xac\n  cn: fed.xac.zagros.01\nzenoh_peering_limits:\n  xac: {rate: 100, size: 4096}\n  nested:\n    deep: [1, 2]\n";
        let r = checkout(&[
            INV,
            GV,
            ("ops/ansible/group_vars/core_router/peerings.yml", body),
        ]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        let before: serde_yaml::Value = serde_yaml::from_str(body).unwrap();
        let after: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
        let (b, a) = (before.as_mapping().unwrap(), after.as_mapping().unwrap());
        assert_eq!(a.len(), b.len(), "no key added or dropped: {text}");
        for (k, v) in b {
            if k.as_str() == Some("zenoh_sovereign_peerings") {
                continue;
            }
            assert_eq!(a.get(k), Some(v), "{k:?} unchanged: {text}");
        }
        let l = after["zenoh_sovereign_peerings"].as_sequence().unwrap();
        assert_eq!(l.len(), 2, "{text}");
        assert_eq!(
            (l[0]["sovereign"].as_str(), l[1]["sovereign"].as_str()),
            (Some("xac"), Some("xab"))
        );
        // A second admission over admission's own output (its header comment included) works.
        let r2 = checkout(&[
            INV,
            GV,
            ("ops/ansible/group_vars/core_router/peerings.yml", &text),
        ]);
        let again = merge_peerings(r2.path(), "core_router", "xad", "fed.xad.zagros.01").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&again).unwrap();
        assert_eq!(
            v["zenoh_peering_limits"], before["zenoh_peering_limits"],
            "{again}"
        );
        assert_eq!(
            v["zenoh_sovereign_peerings"].as_sequence().unwrap().len(),
            3,
            "{again}"
        );
    }

    /// serde_yaml cannot carry a comment through a rewrite. The comment block at
    /// the top of the file is kept verbatim (admission's own header is not doubled);
    /// a comment anywhere else is refused, by line, and nothing is written: no
    /// comment is ever silently dropped.
    #[test]
    fn a_comment_admission_did_not_write_is_refused_not_dropped() {
        // The leading block: kept, before admission's header, and stable on a rerun.
        let body = "# who to call: the platform team\n#\nzenoh_sovereign_peerings:\n- sovereign: xac\n  cn: fed.xac.zagros.01\n";
        let r = checkout(&[
            INV,
            GV,
            ("ops/ansible/group_vars/core_router/peerings.yml", body),
        ]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        assert!(
            text.starts_with("# who to call: the platform team\n#\n# Each partner"),
            "{text}"
        );
        let r2 = checkout(&[
            INV,
            GV,
            ("ops/ansible/group_vars/core_router/peerings.yml", &text),
        ]);
        let again = merge_peerings(r2.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        assert_eq!(again, text, "a rerun changes nothing");
        assert_eq!(again.matches("# Each partner").count(), 1, "{again}");
        // Anywhere else: refused.
        for body in [
            "zenoh_peering_note: kept\n# who to call: the platform team\nzenoh_sovereign_peerings:\n- sovereign: xac\n  cn: fed.xac.zagros.01\n",
            "zenoh_sovereign_peerings:\n- sovereign: xac  # agreed 2026-11-01\n  cn: fed.xac.zagros.01\n",
        ] {
            let r = checkout(&[INV, GV, ("ops/ansible/group_vars/core_router/peerings.yml", body)]);
            let file = r.path().join("ops/ansible/group_vars/core_router/peerings.yml");
            let e = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap_err();
            assert!(e.contains("(GB-237)") && e.contains("comment") && e.contains("line "), "{e}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), body, "unchanged");
        }
        // A `#` inside a quoted value is not a comment.
        let r = checkout(&[
            INV,
            GV,
            (
                "ops/ansible/group_vars/core_router/peerings.yml",
                "zenoh_peering_note: \"room #4\"\nzenoh_sovereign_peerings: []\n",
            ),
        ]);
        let text = merge_peerings(r.path(), "core_router", "xab", "fed.xab.zagros.01").unwrap();
        assert!(text.contains("room #4"), "{text}");
    }
}
