//! Whether a machine installs security updates by itself, read on the machine:
//! `doctor --target <type> --site <site> --live` for a type whose file lists
//! `checks: [automatic-updates]` (docs/design/goatbuilder-build-targets.md
//! section 3.5). One shell command over Ansible reads, and changes nothing; the
//! judging is here. A fact that could not be read is unknown, which is not a
//! pass.

use std::path::Path;

use crate::push;
use crate::Check;

/// The name a type file lists in `checks:`.
pub(crate) const KEY: &str = "automatic-updates";

/// What is read: one `GBAU <what> <value...>` line per fact.
const READ: &str = concat!(
    "echo \"GBAU pkg $(dpkg-query -W -f='${Status}' unattended-upgrades 2>/dev/null)\"; ",
    "echo \"GBAU periodic $(apt-config dump APT::Periodic 2>/dev/null | tr '\\n' ' ')\"; ",
    "for u in unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer; do ",
    "echo \"GBAU unit $u $(systemctl is-enabled $u 2>/dev/null || true) $(systemctl is-active $u 2>/dev/null || true)\"; done"
);

/// The facts one machine printed, or why there are none.
fn read(repo: &Path, inv: &[String], node: &str) -> Result<String, String> {
    let mut args = inv.to_vec();
    args.extend([node.to_string(), "-m".into(), "shell".into(), "-a".into(), READ.into()]);
    let out = push::ansible_cmd(repo, "ansible", &args).output().map_err(|e| format!("could not start ansible: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("ansible answered nothing").trim().to_string();
    if !first.contains("| CHANGED") && !first.contains("| SUCCESS") {
        return Err(first.chars().take(160).collect());
    }
    Ok(text)
}

/// The value APT::Periodic gives `key`, from `apt-config dump`'s one line.
fn periodic(dump: &str, key: &str) -> Option<String> {
    let needle = format!("APT::Periodic::{key} \"");
    let rest = &dump[dump.find(&needle)? + needle.len()..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Judge a machine's facts: what is off, or None when every one holds.
/// `Err` when a fact was not read.
pub(crate) fn judge(facts: &str) -> Result<Vec<String>, String> {
    let line = |what: &str| facts.lines().map(str::trim).find_map(|l| l.strip_prefix(&format!("GBAU {what}")).map(str::trim).map(String::from));
    let mut off = Vec::new();
    match line("pkg") {
        None => return Err("the unattended-upgrades package's state was not read".into()),
        Some(p) if p == "install ok installed" => {}
        Some(_) => off.push("the unattended-upgrades package is not installed".to_string()),
    }
    let dump = line("periodic").ok_or("APT::Periodic was not read")?;
    for key in ["Update-Package-Lists", "Unattended-Upgrade"] {
        match periodic(&dump, key) {
            Some(v) if !v.is_empty() && v != "0" => {}
            Some(v) => off.push(format!("APT::Periodic::{key} is \"{v}\"")),
            None => off.push(format!("APT::Periodic::{key} is not set")),
        }
    }
    for (unit, want_active) in [("unattended-upgrades.service", false), ("apt-daily.timer", true), ("apt-daily-upgrade.timer", true)] {
        let got = line(&format!("unit {unit}")).ok_or_else(|| format!("{unit} was not read"))?;
        let mut w = got.split_whitespace();
        let enabled = w.next().unwrap_or("");
        let active = w.next().unwrap_or("");
        if enabled != "enabled" {
            off.push(format!("{unit} is {}", if enabled.is_empty() { "not found" } else { enabled }));
        } else if want_active && active != "active" {
            off.push(format!("{unit} is enabled but {}", if active.is_empty() { "not running" } else { active }));
        }
    }
    Ok(off)
}

/// The `--live` row of one machine.
pub(crate) fn row(repo: &Path, inv: &[String], node: &str) -> Check {
    let name = format!("automatic security updates {node}");
    let fix = Some("turn automatic security updates back on on the machine (unattended-upgrades and the apt-daily timers), with whatever installed it, then run this again".to_string());
    match read(repo, inv, node).and_then(|f| judge(&f)) {
        Ok(off) if off.is_empty() => Check { name, ok: true, detail: "enabled: unattended-upgrades is installed and enabled, the apt-daily and apt-daily-upgrade timers are on, and APT::Periodic asks for both".into(), fix: None },
        Ok(off) => Check { name, ok: false, detail: format!("not enabled: {}", off.join("; ")), fix },
        Err(why) => Check { name, ok: false, detail: format!("UNKNOWN — not read: {why}"), fix: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "vm-node-1 | CHANGED | rc=0 >>\nGBAU pkg install ok installed\nGBAU periodic APT::Periodic \"\"; APT::Periodic::Update-Package-Lists \"1\"; APT::Periodic::Unattended-Upgrade \"1\"; \nGBAU unit unattended-upgrades.service enabled active\nGBAU unit apt-daily.timer enabled active\nGBAU unit apt-daily-upgrade.timer enabled active\n";

    #[test]
    fn a_machine_that_updates_itself_passes() {
        assert_eq!(judge(GOOD).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn each_part_turned_off_on_its_own_is_named() {
        for (from, to, says) in [
            ("install ok installed", "deinstall ok config-files", "package is not installed"),
            ("Unattended-Upgrade \"1\"", "Unattended-Upgrade \"0\"", "APT::Periodic::Unattended-Upgrade is \"0\""),
            ("APT::Periodic::Update-Package-Lists \"1\"; ", "", "APT::Periodic::Update-Package-Lists is not set"),
            ("unattended-upgrades.service enabled active", "unattended-upgrades.service masked inactive", "unattended-upgrades.service is masked"),
            ("apt-daily.timer enabled active", "apt-daily.timer disabled inactive", "apt-daily.timer is disabled"),
            ("apt-daily-upgrade.timer enabled active", "apt-daily-upgrade.timer enabled inactive", "apt-daily-upgrade.timer is enabled but inactive"),
            ("apt-daily.timer enabled active", "apt-daily.timer  ", "apt-daily.timer is not found"),
        ] {
            let off = judge(&GOOD.replace(from, to)).unwrap();
            assert!(off.len() == 1 && off[0].contains(says), "{from} -> {to}: {off:?}");
        }
    }

    #[test]
    fn a_fact_that_was_not_read_is_unknown_and_never_a_pass() {
        for gone in ["GBAU pkg", "GBAU periodic", "GBAU unit apt-daily.timer"] {
            let facts: String = GOOD.lines().filter(|l| !l.starts_with(gone)).map(|l| format!("{l}\n")).collect();
            assert!(judge(&facts).is_err(), "{gone}");
        }
    }
}
