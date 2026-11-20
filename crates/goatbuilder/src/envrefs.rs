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
    let _ = (cmd, get);
    Vec::new()
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
        let cmd = r#"'/r/ops/scripts/with-secrets' bash -c 'echo "$LATER"' && echo \$NOT_A_REF ${WITH_DEFAULT:-x} ${ALSO-y} $(printenv VIA_PRINTENV) $((1+2)) $1 $? $$"#;
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
        let cmd = r#"cd '/r/ops/ansible' && ansible-playbook -i inventory/mgmt-hosts-mesh.ini -l node-x site-bootstrap.yml -e site_id=alpha -e bootstrap_owner_email="$OWNER_EMAIL" -e 'postgres_password={{ lookup("env", "ALPHA_PG_PASS") }}'"#;
        assert_eq!(unset(cmd, &none), vec!["OWNER_EMAIL"]);
    }
}
