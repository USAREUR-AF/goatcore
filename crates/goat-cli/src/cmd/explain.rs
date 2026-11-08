// `goat explain` — a code, or any error text, interpreted against the
// catalog goat and goatbuilder share (crates/goat-errors, ops/errors/
// catalog.yaml, ADR 1199). The catalog is compiled in, so the lookup works
// offline and on a machine with no checkout. Three layers, in order: the
// exact code the text carries, the catalog's patterns over the whole text
// (a tool's stderr carried inside a goat error is found too), and a lexical
// nearest neighbour for text no pattern knows.

use clap::Args;

#[derive(Args, Debug)]
pub struct ExplainArgs {
    /// A code (`E_PROFILE_MISSING`, `profile-missing`, `GB-007`, `X-SSH-001`)
    /// or the words of an error. `-` reads the text from stdin.
    #[arg(value_name = "CODE|TEXT", num_args = 0.., trailing_var_arg = true)]
    pub input: Vec<String>,

    /// Also write the text and its interpretation to ~/.goat/error-reports/
    /// — the file to share when the catalog does not know the error.
    #[arg(long)]
    pub report: bool,

    /// List every code the catalog knows.
    #[arg(long)]
    pub list: bool,

    /// Machine output: the entry, or the interpretation, as JSON.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(_ctx: &super::Ctx, args: ExplainArgs) -> anyhow::Result<()> {
    let cat = goat_errors::catalog();
    if args.list {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&cat.entries)?);
        } else {
            for e in &cat.entries {
                println!("{:<22} {:<16} {}", e.code, e.tool, e.title);
            }
        }
        return Ok(());
    }
    let text = if args.input.is_empty() || args.input == ["-"] {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    } else {
        args.input.join(" ")
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        anyhow::bail!("give a code or an error text, or `-` to read it from stdin");
    }
    if !text.contains(char::is_whitespace) {
        if let Some(e) = goat_errors::explain(&text) {
            if args.json {
                println!("{}", serde_json::to_string_pretty(e)?);
            } else {
                print!("{}", goat_errors::render_entry(e, ""));
            }
            return Ok(());
        }
    }
    let interp = goat_errors::interpret(&text);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&interp)?);
    } else {
        println!("{}", interp.summary);
        print!("{}", goat_errors::render_human(&interp, "goat"));
    }
    if args.report {
        let p = goat_errors::write_report(&text, &interp, "goat")?;
        eprintln!("report written: {}", p.display());
    }
    if interp.unrecognised {
        // No code list here: the top-level path interprets this message too,
        // and a list of every code would be read as every code matching.
        anyhow::bail!(
            "no catalog entry matches `{}` — `goat explain --list` shows the {} codes it knows",
            interp.summary,
            cat.entries.len()
        );
    }
    Ok(())
}

/// Append any inner-tool interpretation to a rendered goat error: the E_*
/// entry itself is already rendered by `DisplayHuman`, so only what else
/// the text matches (a tool's words carried in the hint) is added.
pub fn passthrough_hits(text: &str, own_code: Option<&str>) -> String {
    let mut i = goat_errors::interpret(text);
    i.hits.retain(|h| Some(h.code.as_str()) != own_code && h.layer != goat_errors::Layer::Lexical);
    if i.hits.is_empty() {
        return String::new();
    }
    i.unrecognised = false;
    goat_errors::render_human(&i, "goat")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_resolve_in_kebab_and_bare_forms() {
        assert_eq!(goat_errors::explain("profile-missing").unwrap().code, "E_PROFILE_MISSING");
        assert_eq!(goat_errors::explain("e_profile_missing").unwrap().code, "E_PROFILE_MISSING");
        assert_eq!(goat_errors::explain("E_PROFILE_MISSING").unwrap().exit, Some(3));
        assert!(goat_errors::explain("E_DOES_NOT_EXIST").is_none());
    }

    #[test]
    fn every_goat_error_variant_code_is_in_the_catalog() {
        let codes = goat_errors::catalog().codes();
        for c in [
            "E_PROFILE_MISSING", "E_PROFILE_DENIED", "E_OFF_MESH", "E_CLOCK_SKEW", "E_ROUTER_UNREACHABLE",
            "E_CERT_EXPIRED", "E_DOCTOR_INCONCLUSIVE", "E_PORTAL_UNREACHABLE", "E_PORTAL_BAD_RESPONSE",
            "E_TOKEN_INVALID", "E_PORTAL_FORBIDDEN", "E_NOT_ADMIN", "E_TOPIC_CONFLICT", "E_BUNDLE_INVALID",
            "E_PORTAL_NOT_FOUND", "E_PORTAL_RATE_LIMITED", "E_PORTAL_SERVER_ERROR", "E_PORTAL_REFUSED",
            "E_ACL_DECLARATION_MISSING",
        ] {
            assert!(codes.contains(&c), "{c} missing from ops/errors/catalog.yaml");
        }
    }

    #[test]
    fn the_portal_answer_codes_explain_with_the_binary_exit_codes() {
        // The catalog's `exit:` and the binary's `exit_code()` are one fact (#2087).
        for (status, code) in [(404, "E_PORTAL_NOT_FOUND"), (429, "E_PORTAL_RATE_LIMITED"), (503, "E_PORTAL_SERVER_ERROR"), (422, "E_PORTAL_REFUSED")] {
            let err = crate::errors::portal_answer(status, "/api/catalog", None, "");
            let entry = goat_errors::explain(code).unwrap_or_else(|| panic!("{code} not in the catalog"));
            assert_eq!(err.code(), code);
            assert_eq!(entry.exit, Some(err.exit_code()), "{code}");
        }
        // Text carried inside another error is recognized too.
        let i = goat_errors::interpret("error: the portal answered HTTP 429 for `/api/catalog`: too many requests, retry later");
        assert!(i.hits.iter().any(|h| h.code == "E_PORTAL_RATE_LIMITED"), "{:?}", i.hits.iter().map(|h| &h.code).collect::<Vec<_>>());
    }

    #[test]
    fn passthrough_adds_only_the_inner_tool() {
        let s = passthrough_hits("error: portal API at `https://portal` is unreachable\n  help: curl: (6) Could not resolve host: portal.zagros\n  code: E_PORTAL_UNREACHABLE", Some("E_PORTAL_UNREACHABLE"));
        assert!(s.contains("X-DNS-001"), "{s}");
        assert!(!s.contains("E_PORTAL_UNREACHABLE —"), "{s}");
        assert!(passthrough_hits("error: plain words only here", None).is_empty());
    }
}
