// `goat federate` — the verbs an operator runs to federate with a partner.
//
// The verbs themselves (arguments, checks, reports, exit codes) live in the
// `goat-federation` library (`goat_federation::verbs`), so every front end runs
// the same code with the same flags; `goatbuilder federate <verb>` is the
// other, and the one federation is moving to. This file supplies what only `goat`
// knows: where the active context keeps the declaration, the certificate and
// the CA roots; and it carries the verb's exit code out through `anyhow`.

pub use goat_federation::verbs::FederateCmd;
use goat_federation::verbs::{self, Site, EXIT_OK};

/// The one line `goat federate <verb>` prints on stderr while federation moves
/// to goatbuilder. stderr, so a `--json` report on stdout stays one document.
pub fn moving_notice(verb: &str) -> String {
    format!("goat federate is moving to goatbuilder and will be removed from goat: run `goatbuilder federate {verb} ...`")
}

pub async fn run(ctx: &super::Ctx, cmd: FederateCmd) -> anyhow::Result<()> {
    eprintln!("{}", moving_notice(cmd.name()));
    let site = || -> anyhow::Result<Site> {
        Ok(Site {
            context_dir: ctx.profile.context_dir(),
            cert_path: ctx.profile.cert_path(),
            ca_roots_path: ctx.profile.ca_roots_path(),
        })
    };
    let mut out = std::io::stdout().lock();
    match verbs::run(cmd, &site, &mut out) {
        Ok(EXIT_OK) => Ok(()),
        Ok(code) => bail_with(code),
        Err(e) => match refusal_exit(&e) {
            Some(code) => bail_with(code),
            None => Err(e),
        },
    }
}

/// One of the library's own refusals (X-FED-nnn), or another catalog-coded
/// error it passed up: print the message and its code, as goatbuilder does,
/// and return the exit. An X-FED code has no catalog exit (the library is
/// not a binary) and exits 1, as in goatbuilder, so the binaries exit alike.
/// Anything uncoded is left for main.rs, which prints it and exits 1.
fn refusal_exit(e: &anyhow::Error) -> Option<i32> {
    let c = e.downcast_ref::<goat_errors::Coded>()?;
    eprintln!("error: {e:#}");
    eprintln!("  code: {}", c.code);
    Some(
        goat_errors::explain(&c.code)
            .and_then(|x| x.exit)
            .unwrap_or(1),
    )
}

/// An exit code carried out through `anyhow` without a second error message —
/// every verb has already printed its own structured report by the time it
/// bails, and a bare `error:` line on top of it would say less than nothing.
#[derive(Debug)]
struct Exit(i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never rendered: main.rs reads the code off this type instead.
        write!(f, "federate exited {}", self.0)
    }
}
impl std::error::Error for Exit {}

/// `main.rs` asks this before falling back to EXIT_GENERIC.
pub fn exit_code_of(err: &anyhow::Error) -> Option<i32> {
    err.downcast_ref::<Exit>().map(|e| e.0)
}

fn bail_with(code: i32) -> anyhow::Result<()> {
    Err(anyhow::Error::new(Exit(code)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// goat's own exit-code table and the library's agree, so a wrapper that
    /// reads either reads the same numbers.
    #[test]
    fn exit_codes_match_the_library() {
        use crate::exit_codes as g;
        assert_eq!(g::EXIT_FED_REFUSED, verbs::EXIT_FED_REFUSED);
        assert_eq!(g::EXIT_FED_MALFORMED, verbs::EXIT_FED_MALFORMED);
        assert_eq!(g::EXIT_FED_MISMATCH, verbs::EXIT_FED_MISMATCH);
        assert_eq!(g::EXIT_FED_UNDERIVED, verbs::EXIT_FED_UNDERIVED);
        assert_eq!(g::EXIT_FED_NOT_CONFORMANT, verbs::EXIT_FED_NOT_CONFORMANT);
    }

    /// A library refusal prints its code and exits 1; an uncoded fault is
    /// passed on to main.rs unchanged.
    #[test]
    fn a_library_refusal_exits_1_and_an_uncoded_fault_passes_on() {
        let coded = anyhow::Error::from(goat_errors::coded("X-FED-001", "x: unreadable"));
        assert_eq!(refusal_exit(&coded), Some(1));
        assert_eq!(refusal_exit(&anyhow::anyhow!("Broken pipe")), None);
    }

    #[test]
    fn the_moving_notice_names_the_goatbuilder_command() {
        assert_eq!(
            moving_notice("lint"),
            "goat federate is moving to goatbuilder and will be removed from goat: run `goatbuilder federate lint ...`"
        );
    }
}
