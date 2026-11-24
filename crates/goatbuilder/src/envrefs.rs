//! The environment variables a shell command line reads as `$NAME` or
//! `${NAME}`, so a verb can refuse before running a step whose command bash
//! (`-u`) would stop with `NAME: unbound variable` (GB-369).

use std::collections::BTreeSet;

/// The names `cmd` expands as `$NAME` or `${NAME}` in a place the outer shell
/// expands them, and that `get` does not hold. Not counted: anything in single
/// quotes (a nested command expands it later, in its own environment), an
/// escaped `\$`, `${NAME:-…}` and `${NAME-…}` (a default), and a name the
/// command assigns itself (`NAME=`, `for NAME in`, `read … NAME`).
pub fn unset(cmd: &str, get: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let assigned = assigned(cmd);
    read(cmd).into_iter().filter(|n| !assigned.contains(n) && get(n).is_none()).collect()
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Every name `cmd` expands outside single quotes, without a default.
fn read(cmd: &str) -> BTreeSet<String> {
    let c: Vec<char> = cmd.chars().collect();
    let mut out = BTreeSet::new();
    let (mut i, mut single, mut double) = (0, false, false);
    while i < c.len() {
        let ch = c[i];
        if single {
            single = ch != '\'';
            i += 1;
            continue;
        }
        match ch {
            '\\' => i += 2,
            '\'' if !double => {
                single = true;
                i += 1;
            }
            '"' => {
                double = !double;
                i += 1;
            }
            '$' if i + 1 < c.len() && is_name_start(c[i + 1]) => {
                let mut j = i + 1;
                while j < c.len() && is_name(c[j]) {
                    j += 1;
                }
                out.insert(c[i + 1..j].iter().collect());
                i = j;
            }
            '$' if i + 2 < c.len() && c[i + 1] == '{' && is_name_start(c[i + 2]) => {
                let mut j = i + 2;
                while j < c.len() && is_name(c[j]) {
                    j += 1;
                }
                let name: String = c[i + 2..j].iter().collect();
                // `${NAME}` reads it; `${NAME:-x}`, `${NAME-x}` (and the other
                // default and assign forms) do not stop bash -u.
                let defaulted = matches!(c.get(j), Some('-') | Some('=') | Some('?') | Some('+')) || (c.get(j) == Some(&':') && matches!(c.get(j + 1), Some('-') | Some('=') | Some('+')));
                if !defaulted {
                    out.insert(name);
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

/// The names `cmd` assigns itself: `NAME=…` at the start of a word,
/// `for NAME in`, `read [-opts] NAME…`, `export`/`local`/`declare NAME`.
fn assigned(cmd: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let words: Vec<&str> = cmd.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')')).filter(|w| !w.is_empty()).collect();
    for (k, w) in words.iter().enumerate() {
        let w = w.trim_start_matches('"');
        if let Some((name, _)) = w.split_once('=') {
            if !name.is_empty() && name.starts_with(is_name_start) && name.chars().all(is_name) {
                out.insert(name.to_string());
            }
        }
        match w {
            "for" => {
                if let Some(n) = words.get(k + 1) {
                    out.insert(n.to_string());
                }
            }
            "read" | "export" | "local" | "declare" | "readonly" => {
                for n in words[k + 1..].iter().take_while(|n| !n.contains('<') && !n.contains('>')) {
                    if n.starts_with('-') {
                        continue;
                    }
                    let n = n.split('=').next().unwrap_or("");
                    if n.starts_with(is_name_start) && n.chars().all(is_name) {
                        out.insert(n.to_string());
                    } else {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn a_bare_or_braced_reference_that_is_unset_is_named() {
        let cmd = r#"ansible-playbook -e site_id=alpha -e bootstrap_owner_email="$OWNER_EMAIL" -e x=${OTHER_ONE} play.yml"#;
        assert_eq!(unset(cmd, &none), vec!["OTHER_ONE", "OWNER_EMAIL"]);
    }

    #[test]
    fn a_set_name_is_not_named() {
        let get = |n: &str| (n == "OWNER_EMAIL").then(|| "ops@example.org".to_string());
        assert!(unset(r#"x -e e="$OWNER_EMAIL""#, &get).is_empty());
        // Set but empty: bash -u does not stop on it, so neither does this.
        let empty = |n: &str| (n == "OWNER_EMAIL").then(String::new);
        assert!(unset(r#"x -e e="$OWNER_EMAIL""#, &empty).is_empty());
    }

    #[test]
    fn single_quotes_escapes_and_defaults_are_not_read_here() {
        // Built with concat! so ops/scripts/generate-envrc-example.py does not
        // read this test's name as a variable goatbuilder reads.
        let cmd = concat!(r#"'/r/ops/scripts/with-secrets' bash -c 'echo "$LATER"' && echo \$NOT_A_REF ${"#, "WITH_DEFAULT", r#":-x} ${ALSO-y} $(printenv VIA_PRINTENV) $((1+2)) $1 $? $$"#);
        assert!(unset(cmd, &none).is_empty(), "{:?}", unset(cmd, &none));
    }

    #[test]
    fn a_name_the_command_assigns_is_its_own() {
        let cmd = r#"VF=$(mktemp) && enc=$(grep -c x f) && for h in a b; do echo "$h $VF $enc"; done; read -r ans <f; echo "$ans"; export LATE=1; echo "$LATE""#;
        assert!(unset(cmd, &none).is_empty(), "{:?}", unset(cmd, &none));
    }

    /// The drill that found it: phase 13's founder pass ran without
    /// OWNER_EMAIL, and bash stopped before Ansible with no catalogued error.
    #[test]
    fn the_phase_13_founder_pass_names_owner_email() {
        // concat! keeps the generator from listing this test's secret name.
        let cmd = concat!(r#"cd '/r/ops/ansible' && ansible-playbook -i inventory/mgmt-hosts-mesh.ini -l node-x site-bootstrap.yml -e site_id=alpha -e bootstrap_owner_email="$OWNER_EMAIL" -e 'postgres_password={{ lookup("env", ""#, "ALPHA_PG_PASS", r#"") }}'"#);
        assert_eq!(unset(cmd, &none), vec!["OWNER_EMAIL"]);
    }
}
