//! Which secret values a planned shell line would put on a command line.
//!
//! A standup line passes a secret through the environment: the play reads it
//! with `{{ lookup("env", "NAME") }}` inside single quotes, so the shell
//! expands nothing and the value is on no process's argument list. A line that
//! expands `$(printenv NAME)`, `$NAME` or `${NAME}` outside single quotes hands
//! the value to whatever command the word belongs to. This module reads a line
//! as the shell would split it and names every secret-class variable expanded
//! into a word, except a word that only assigns a shell variable.
//!
//! Self-contained so the integration tests can include it by path
//! (tests/no_secret_on_argv.rs). Test-only.

/// The suffixes of every name ops/scripts/newsite-secrets.sh generates (its
/// SPEC, `${S}_NAME|generator|comment`), read from the script's text.
pub fn spec_suffixes(newsite_secrets_sh: &str) -> Vec<String> {
    let mut out: Vec<String> = newsite_secrets_sh
        .lines()
        .filter_map(|l| l.strip_prefix("${S}_"))
        .filter_map(|r| r.split('|').next())
        .filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .map(String::from)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A secret-class name: one of newsite-secrets' SPEC names for any site, or a
/// name that says it holds a token, a secret or a password.
pub fn is_secret(name: &str, spec: &[String]) -> bool {
    spec.iter().any(|s| name.ends_with(&format!("_{s}")))
        || ["TOKEN", "SECRET", "PASSWORD", "PASSWD"].iter().any(|w| name.split('_').any(|p| p == *w))
}

/// Every secret-class name whose value `line` expands into a command's
/// arguments, in order of appearance.
pub fn secret_values_on_argv(line: &str, spec: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for w in words(line) {
        if w.assignment {
            continue;
        }
        for n in w.expands {
            if is_secret(&n, spec) {
                out.push(n);
            }
        }
    }
    out
}

/// One shell word: whether it only assigns a variable (`NAME=value` in command
/// position), and the variables it expands.
#[derive(Debug, Default)]
struct Word {
    raw: String,
    expands: Vec<String>,
    assignment: bool,
}

fn ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The index just past the `)` that closes the `$(` whose body starts at `i`.
fn close_paren(c: &[char], mut i: usize) -> usize {
    let (mut depth, mut sq, mut dq) = (1usize, false, false);
    while i < c.len() {
        let ch = c[i];
        if sq {
            sq = ch != '\'';
        } else if ch == '\\' {
            i += 1;
        } else if dq {
            dq = ch != '"';
        } else {
            match ch {
                '\'' => sq = true,
                '"' => dq = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    c.len()
}

/// The words of `line` across every simple command in it, with the words of
/// each `$(...)` body that is not a bare `printenv NAME` added as their own.
fn words(line: &str) -> Vec<Word> {
    let c: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut cur = Word::default();
    let mut started = false;
    let mut cmd_pos = true;
    let (mut sq, mut dq) = (false, false);
    let mut i = 0;
    let finish = |cur: &mut Word, started: &mut bool, cmd_pos: &mut bool, out: &mut Vec<Word>| {
        if !*started {
            return;
        }
        let w = std::mem::take(cur);
        let assigns = w.raw.split_once('=').is_some_and(|(k, _)| !k.is_empty() && !k.starts_with(|ch: char| ch.is_ascii_digit()) && k.chars().all(ident));
        let mut w = w;
        w.assignment = *cmd_pos && assigns;
        // After an assignment, or a keyword that opens a command, the next word
        // is still in command position.
        *cmd_pos = w.assignment || matches!(w.raw.as_str(), "if" | "then" | "else" | "elif" | "do" | "while" | "until" | "!" | "{" | "}" | "time");
        out.push(w);
        *started = false;
    };
    while i < c.len() {
        let ch = c[i];
        if sq {
            if ch == '\'' {
                sq = false;
            } else {
                cur.raw.push(ch);
            }
            i += 1;
            continue;
        }
        if ch == '\\' && i + 1 < c.len() {
            cur.raw.push(c[i + 1]);
            started = true;
            i += 2;
            continue;
        }
        if ch == '$' && i + 1 < c.len() {
            started = true;
            if c[i + 1] == '(' {
                let end = close_paren(&c, i + 2);
                let body: String = c[i + 2..end.saturating_sub(1).max(i + 2)].iter().collect();
                let t = body.trim();
                match t.strip_prefix("printenv ").map(str::trim).filter(|n| !n.is_empty() && n.chars().all(ident)) {
                    Some(n) => cur.expands.push(n.to_string()),
                    None => out.extend(words(&body)),
                }
                cur.raw.push_str("$(...)");
                i = end;
                continue;
            }
            if c[i + 1] == '{' {
                let n: String = c[i + 2..].iter().take_while(|ch| ident(**ch)).collect();
                let mut j = i + 2;
                while j < c.len() && c[j] != '}' {
                    j += 1;
                }
                if !n.is_empty() {
                    cur.expands.push(n);
                }
                cur.raw.push_str("${...}");
                i = (j + 1).min(c.len());
                continue;
            }
            let n: String = c[i + 1..].iter().take_while(|ch| ident(**ch)).collect();
            if !n.is_empty() && !n.starts_with(|ch: char| ch.is_ascii_digit()) {
                cur.raw.push_str("$...");
                i += 1 + n.len();
                cur.expands.push(n);
                continue;
            }
            cur.raw.push(ch);
            i += 1;
            continue;
        }
        if dq {
            if ch == '"' {
                dq = false;
            } else {
                cur.raw.push(ch);
            }
            i += 1;
            continue;
        }
        match ch {
            '\'' => {
                sq = true;
                started = true;
            }
            '"' => {
                dq = true;
                started = true;
            }
            ' ' | '\t' | '\n' => finish(&mut cur, &mut started, &mut cmd_pos, &mut out),
            ';' | '&' | '|' | '(' | ')' => {
                finish(&mut cur, &mut started, &mut cmd_pos, &mut out);
                cmd_pos = true;
            }
            '>' | '<' => {
                finish(&mut cur, &mut started, &mut cmd_pos, &mut out);
                // The target of a redirection is a file name, not an argument;
                // it is still a word the shell expands, so it is kept.
            }
            _ => {
                cur.raw.push(ch);
                started = true;
            }
        }
        i += 1;
    }
    finish(&mut cur, &mut started, &mut cmd_pos, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Vec<String> {
        ["PG_PASS", "NB_ENCKEY", "BKP_KEY", "RELAY_SECRET"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_spec_is_read_from_newsite_secrets() {
        let sh = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/scripts/newsite-secrets.sh")).unwrap();
        let s = spec_suffixes(&sh);
        for n in ["PG_PASS", "NB_ENCKEY", "BKP_KEY", "RELAY_SECRET", "KRATOS_CIPHER"] {
            assert!(s.iter().any(|x| x == n), "{n} in {s:?}");
        }
        assert!(s.len() >= 40, "{s:?}");
    }

    #[test]
    fn a_printenv_in_double_quotes_puts_the_value_on_the_command_line() {
        let l = "cd '/r/ops/ansible' && x -i inventory/a.ini ory-deploy.yml -e postgres_superuser_password=\"$(printenv ALPHA_PG_PASS)\"";
        assert_eq!(secret_values_on_argv(l, &spec()), vec!["ALPHA_PG_PASS"]);
    }

    #[test]
    fn plain_and_braced_expansions_count_too() {
        assert_eq!(secret_values_on_argv("x -e k=$ALPHA_NB_ENCKEY -e j=\"${ALPHA_BKP_KEY}\"", &spec()), vec!["ALPHA_NB_ENCKEY", "ALPHA_BKP_KEY"]);
        assert_eq!(secret_values_on_argv("x -e t=\"$(printenv CLOUDFLARE_API_TOKEN_ALPHA)\"", &spec()), vec!["CLOUDFLARE_API_TOKEN_ALPHA"]);
    }

    #[test]
    fn an_env_lookup_in_single_quotes_expands_nothing() {
        let l = "x -e 'postgres_superuser_password={{ lookup(\"env\", \"ALPHA_PG_PASS\") }}' -e 'k={{ lookup(\"env\", \"ALPHA_NB_ENCKEY\") }}'";
        assert!(secret_values_on_argv(l, &spec()).is_empty());
        assert!(secret_values_on_argv("echo '$(printenv ALPHA_PG_PASS)'", &spec()).is_empty());
        assert!(secret_values_on_argv("echo \\$ALPHA_PG_PASS", &spec()).is_empty());
    }

    #[test]
    fn a_builtin_printf_into_a_file_still_counts() {
        let l = "umask 077 && VF=$(mktemp -t v.XXXXXX) && printf 'a: \"%s\"\\n' \"$(printenv ALPHA_RELAY_SECRET)\" > \"$VF\" && x -e @\"$VF\"";
        assert_eq!(secret_values_on_argv(l, &spec()), vec!["ALPHA_RELAY_SECRET"]);
    }

    #[test]
    fn an_assignment_in_command_position_is_not_an_argument() {
        assert!(secret_values_on_argv("PGPASSWORD=\"$(printenv ALPHA_PG_PASS)\" psql -h db", &spec()).is_empty());
        assert!(secret_values_on_argv("a && T=$ALPHA_PG_PASS; x", &spec()).is_empty());
        // ... but the same text after a command is an argument.
        assert_eq!(secret_values_on_argv("env PGPASSWORD=\"$(printenv ALPHA_PG_PASS)\" psql", &spec()), vec!["ALPHA_PG_PASS"]);
    }

    #[test]
    fn a_nested_command_is_read_as_its_own_command() {
        assert_eq!(secret_values_on_argv("x \"$(curl -H \"Authorization: $(printenv ALPHA_PG_PASS)\" u)\"", &spec()), vec!["ALPHA_PG_PASS"]);
    }

    #[test]
    fn names_that_are_not_secrets_are_left_alone() {
        assert!(secret_values_on_argv("x -e bootstrap_owner_email=\"$(printenv GOAT_BOOTSTRAP_OWNER_EMAIL)\" -e site=$SITE \"$VF\"", &spec()).is_empty());
        assert!(is_secret("ALPHA_DEV_PG_PASS", &spec()));
        assert!(is_secret("FORGEJO_PUBSUB_TOKEN", &spec()));
        assert!(!is_secret("GOAT_AWS_PROFILE", &spec()));
    }
}
