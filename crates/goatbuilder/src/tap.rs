//! A network tap's declaration (docs/design/goatbuilder-build-targets.md
//! section 3.8). A machine of a type that `declares: [tap]` states, in its
//! host_vars, how it sits on the link it records:
//!
//! ```yaml
//! tap:
//!   mode: listen            # listen | inline; required, no default
//!   source: copper-tap      # listen only: copper-tap | bypass-relay | mirror-port
//!   ports: [tap0, tap1]     # the link's interfaces; never the management one
//!   management: mgmt0
//! ```
//!
//! Listen fails open: the link does not depend on the machine. Inline fails
//! closed: the machine bridges the link, so the link is down whenever the
//! machine is. Only the checkout is read; nothing here looks at the machine.

use std::path::Path;

use anyhow::Result;
use serde_yaml::Value;

/// The host_vars key a tap is declared under.
pub(crate) const KEY: &str = "tap";

const CODE: &str = "GB-380";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Listen,
    Inline,
}

/// A tap's declaration, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tap {
    pub mode: Mode,
    /// What hands a listening machine its copy of the link; None when inline.
    pub source: Option<String>,
    pub ports: Vec<String>,
    pub management: String,
}

const SOURCES: [&str; 3] = ["copper-tap", "bypass-relay", "mirror-port"];
const KEYS: [&str; 4] = ["mode", "source", "ports", "management"];

impl Tap {
    /// The line join-node and doctor --target print for the machine.
    pub(crate) fn line(&self) -> String {
        let wires = format!(
            "ports {}; management {}",
            self.ports.join(", "),
            self.management
        );
        match self.mode {
            Mode::Inline => format!("inline, fail-closed: the machine bridges the link, so the link is down whenever the machine is off, restarting or broken ({wires})"),
            Mode::Listen => format!("listen from a {}, fail-open: the link does not pass through the machine, which only receives ({wires})", self.source.as_deref().unwrap_or_default()),
        }
    }
}

/// An interface name Linux accepts (at most 15 bytes, no slash, space or colon).
fn interface(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 15
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
}

/// The machine's tap declaration and the file it is in, read from its
/// host_vars file and the files of its host_vars directory. Exactly one file
/// may set `tap`: which of two Ansible applies is its own precedence, so a
/// second declaration is refused (GB-380) rather than guessed at. None when
/// nothing declares one.
pub(crate) fn declared(repo: &Path, node: &str) -> Result<Option<(Tap, String)>> {
    let dir = repo.join("ops/ansible/host_vars");
    let mut paths: Vec<std::path::PathBuf> = ["yml", "yaml"]
        .iter()
        .map(|x| dir.join(format!("{node}.{x}")))
        .collect();
    let mut inside: Vec<std::path::PathBuf> = std::fs::read_dir(dir.join(node))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
                .collect()
        })
        .unwrap_or_default();
    inside.sort();
    paths.extend(inside);
    let mut hits: Vec<(Value, String)> = Vec::new();
    for p in paths.iter().filter(|p| p.is_file()) {
        let at = format!(
            "ops/ansible/host_vars/{}",
            p.strip_prefix(&dir).unwrap_or(p).display()
        );
        let text = std::fs::read_to_string(p)
            .map_err(|e| refused(node, &at, format!("it cannot be read: {e}")))?;
        if !text.contains(KEY) {
            continue;
        }
        let doc: Value = serde_yaml::from_str(&text)
            .map_err(|e| refused(node, &at, format!("it is not readable YAML: {e}")))?;
        if let Some(v) = doc.get(KEY) {
            hits.push((v.clone(), at));
        }
    }
    if hits.len() > 1 {
        let files: Vec<&str> = hits.iter().map(|(_, at)| at.as_str()).collect();
        return Err(refused(
            node,
            files[0],
            format!(
                "{KEY} is declared in {} files ({}); declare it in one",
                files.len(),
                files.join(", ")
            ),
        ));
    }
    match hits.pop() {
        None => Ok(None),
        Some((v, at)) => check(node, &at, &v).map(|t| Some((t, at))),
    }
}

/// The declaration a machine of a tap type must have: GB-380 when it has none.
pub(crate) fn required(repo: &Path, type_name: &str, node: &str) -> Result<(Tap, String)> {
    declared(repo, node)?.ok_or_else(|| {
        goat_errors::coded(
            CODE,
            format!("{node} is a {type_name}, and its host_vars declare no {KEY}: a network tap states its mode, listen (fail-open) or inline (fail-closed), and there is no default. Write {KEY}: with mode, ports and management (and source, when listening) in ops/ansible/host_vars/{node}.yml"),
        )
        .into()
    })
}

fn refused(node: &str, at: &str, why: String) -> anyhow::Error {
    goat_errors::coded(
        CODE,
        format!("{node}'s {KEY} declaration ({at}) is not valid: {why}"),
    )
    .into()
}

fn check(node: &str, at: &str, v: &Value) -> Result<Tap> {
    let bad = |why: String| refused(node, at, why);
    let Value::Mapping(m) = v else {
        return Err(bad(format!("{KEY} is a mapping with {}", KEYS.join(", "))));
    };
    let mut keys: Vec<String> = Vec::new();
    for k in m.keys() {
        match k.as_str() {
            Some(k) if KEYS.contains(&k) => keys.push(k.to_string()),
            Some(k) => {
                return Err(bad(format!(
                    "it has an unknown key '{k}' (known: {})",
                    KEYS.join(", ")
                )))
            }
            None => return Err(bad("its keys are words".into())),
        }
    }
    let text = |k: &str| -> Result<Option<String>> {
        match m.get(k) {
            None => Ok(None),
            Some(Value::String(s)) if s.contains("{{") => Err(bad(format!(
                "{k} is templated; the checkout is read as it is, so write the value out"
            ))),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(bad(format!("{k} is one word"))),
        }
    };
    let mode = match text("mode")?.as_deref() {
        Some("listen") => Mode::Listen,
        Some("inline") => Mode::Inline,
        Some(other) => return Err(bad(format!("mode is '{other}'; it is listen (fail-open) or inline (fail-closed)"))),
        None => return Err(bad("it has no mode; it is listen (fail-open) or inline (fail-closed), and there is no default".into())),
    };
    let source = text("source")?;
    match (mode, source.as_deref()) {
        (Mode::Inline, Some(_)) => {
            return Err(bad(
                "an inline tap carries the link itself, so it has no source; source is for listen"
                    .into(),
            ))
        }
        (Mode::Listen, None) => {
            return Err(bad(format!(
                "a listening tap names its source: {}",
                SOURCES.join(", ")
            )))
        }
        (Mode::Listen, Some(s)) if !SOURCES.contains(&s) => {
            return Err(bad(format!(
                "source is '{s}'; known: {}",
                SOURCES.join(", ")
            )))
        }
        _ => {}
    }
    let ports: Vec<String> = match m.get("ports") {
        Some(Value::Sequence(items)) => {
            let mut out = Vec::new();
            for i in items {
                match i.as_str() {
                    Some(s) if interface(s) => out.push(s.to_string()),
                    _ => return Err(bad("each of ports is an interface name".into())),
                }
            }
            out
        }
        None => return Err(bad("it has no ports, the link's interfaces".into())),
        Some(_) => return Err(bad("ports is a list of interface names".into())),
    };
    let mut seen = ports.clone();
    seen.sort();
    seen.dedup();
    if seen.len() != ports.len() {
        return Err(bad("ports names one interface twice".into()));
    }
    let (want, why) = match (mode, source.as_deref()) {
        (Mode::Inline, _) => ("2", "an inline tap bridges exactly two ports"),
        (Mode::Listen, Some("copper-tap")) => (
            "2",
            "a passive copper tap gives one output for each direction, so two ports",
        ),
        (Mode::Listen, Some("mirror-port")) => ("1", "a mirror port is one port"),
        _ => (
            "1 or 2",
            "a bypass relay hands the machine one or two ports",
        ),
    };
    if !want.split(" or ").any(|n| n == ports.len().to_string()) {
        return Err(bad(format!("it lists {} port(s): {why}", ports.len())));
    }
    let Some(management) = text("management")? else {
        return Err(bad("it has no management interface, which carries management, the mesh and the default route".into()));
    };
    if !interface(&management) {
        return Err(bad("management is an interface name".into()));
    }
    if ports.contains(&management) {
        return Err(bad(format!("{management} is both a port and the management interface; a tap port never carries management")));
    }
    Ok(Tap {
        mode,
        source,
        ports,
        management,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(yaml: &str) -> Result<Option<(Tap, String)>> {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("ops/ansible/host_vars/zagros-tap-1.yml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, yaml).unwrap();
        declared(t.path(), "zagros-tap-1")
    }

    fn code(e: &anyhow::Error) -> Option<String> {
        e.downcast_ref::<goat_errors::Coded>()
            .map(|c| c.code.clone())
    }

    #[test]
    fn inline_is_shown_fail_closed() {
        let (t, at) = read("tap:\n  mode: inline\n  ports: [tap0, tap1]\n  management: mgmt0\n")
            .unwrap()
            .unwrap();
        assert_eq!(at, "ops/ansible/host_vars/zagros-tap-1.yml");
        assert_eq!(t.mode, Mode::Inline);
        assert!(
            t.line().starts_with("inline, fail-closed: "),
            "{}",
            t.line()
        );
    }

    #[test]
    fn listen_names_its_source_and_is_shown_fail_open() {
        for (source, ports) in [
            ("copper-tap", "[tap0, tap1]"),
            ("mirror-port", "[tap0]"),
            ("bypass-relay", "[tap0]"),
            ("bypass-relay", "[tap0, tap1]"),
        ] {
            let (t, _) = read(&format!(
                "tap:\n  mode: listen\n  source: {source}\n  ports: {ports}\n  management: mgmt0\n"
            ))
            .unwrap()
            .unwrap();
            assert_eq!(t.mode, Mode::Listen);
            assert!(
                t.line()
                    .starts_with(&format!("listen from a {source}, fail-open: ")),
                "{}",
                t.line()
            );
        }
    }

    #[test]
    fn no_declaration_is_none_and_a_tap_type_refuses_it() {
        assert!(read("platform_family: small-computer\n").unwrap().is_none());
        let t = tempfile::tempdir().unwrap();
        let e = required(t.path(), "network-tap", "zagros-tap-1").unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-380"));
        assert!(format!("{e:#}").contains("there is no default"), "{e:#}");
    }

    #[test]
    fn a_bad_declaration_is_refused_with_its_reason() {
        for (yaml, says) in [
            ("tap:\n  ports: [tap0, tap1]\n  management: mgmt0\n", "it has no mode"),
            ("tap:\n  mode: passive\n  ports: [tap0, tap1]\n  management: mgmt0\n", "mode is 'passive'"),
            ("tap:\n  mode: inline\n  source: copper-tap\n  ports: [tap0, tap1]\n  management: mgmt0\n", "has no source"),
            ("tap:\n  mode: listen\n  ports: [tap0, tap1]\n  management: mgmt0\n", "names its source"),
            ("tap:\n  mode: listen\n  source: hub\n  ports: [tap0]\n  management: mgmt0\n", "source is 'hub'"),
            ("tap:\n  mode: inline\n  ports: [tap0]\n  management: mgmt0\n", "bridges exactly two ports"),
            ("tap:\n  mode: listen\n  source: copper-tap\n  ports: [tap0]\n  management: mgmt0\n", "one output for each direction"),
            ("tap:\n  mode: listen\n  source: mirror-port\n  ports: [tap0, tap1]\n  management: mgmt0\n", "a mirror port is one port"),
            ("tap:\n  mode: inline\n  ports: [tap0, mgmt0]\n  management: mgmt0\n", "both a port and the management interface"),
            ("tap:\n  mode: inline\n  ports: [tap0, tap0]\n  management: mgmt0\n", "one interface twice"),
            ("tap:\n  mode: inline\n  ports: [tap0, tap1]\n", "no management interface"),
            ("tap:\n  mode: inline\n  ports: [tap0, tap1]\n  management: mgmt0\n  speed: 1000\n", "unknown key 'speed'"),
            ("tap:\n  mode: \"{{ tap_mode }}\"\n  ports: [tap0, tap1]\n  management: mgmt0\n", "templated"),
            ("tap:\n  mode: inline\n  ports: [tap0, \"a/b\"]\n  management: mgmt0\n", "interface name"),
            ("tap: inline\n", "is a mapping"),
        ] {
            let e = read(yaml).unwrap_err();
            assert_eq!(code(&e).as_deref(), Some("GB-380"), "{yaml}");
            assert!(format!("{e:#}").contains(says), "{yaml}: {e:#}");
        }
    }

    #[test]
    fn a_tap_declared_in_two_host_vars_files_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("ops/ansible/host_vars/zagros-tap-1");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("a.yml"),
            "tap:\n  mode: listen\n  source: mirror-port\n  ports: [tap0]\n  management: mgmt0\n",
        )
        .unwrap();
        std::fs::write(
            d.join("b.yml"),
            "tap:\n  mode: inline\n  ports: [tap0, tap1]\n  management: mgmt0\n",
        )
        .unwrap();
        let e = declared(t.path(), "zagros-tap-1").unwrap_err();
        assert_eq!(code(&e).as_deref(), Some(CODE));
        assert!(
            format!("{e:#}").contains("tap is declared in 2 files (ops/ansible/host_vars/zagros-tap-1/a.yml, ops/ansible/host_vars/zagros-tap-1/b.yml); declare it in one"),
            "{e:#}"
        );
        // One declaration, beside a file that sets other values, is read.
        std::fs::write(d.join("a.yml"), "platform_family: small-computer\n").unwrap();
        let (tap, at) = declared(t.path(), "zagros-tap-1").unwrap().unwrap();
        assert_eq!(
            (tap.mode, at.as_str()),
            (Mode::Inline, "ops/ansible/host_vars/zagros-tap-1/b.yml")
        );
    }
}
