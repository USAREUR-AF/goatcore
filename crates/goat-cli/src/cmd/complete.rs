// Hidden `goat _complete` subcommand — emits one completion
// candidate per line to stdout. Used by operator-installed shell
// hooks to provide live dynamic completion for things `clap_complete`
// can't generate statically (e.g. topic keys that exist right now
// at the portal). Pattern lifted from `gh __complete` and
// `kubectl __complete`.
//
// Wiring example (~/.zshrc, after `goat completion zsh > ...`):
//
//   _goat_topic_complete() {
//     local partial="${words[CURRENT]}"
//     local lines=("${(@f)$(goat _complete topic-key "$partial" 2>/dev/null)}")
//     compadd -- "${lines[@]}"
//   }
//
// The subcommand is named with a leading underscore + hidden in
// --help so operators don't trip on it accidentally; the only
// supported caller is shell completion code.

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CompleteCmd {
    /// Emit topic keys matching a partial prefix. Used by shell
    /// completion hooks for `goat topic show|delete|sub|pub <TAB>`.
    /// Output is one key per line on stdout. Errors go to stderr
    /// and exit code is 0 (so an absent portal doesn't break
    /// the operator's interactive shell).
    TopicKey {
        /// Partial prefix the operator has already typed. Filters
        /// the result. Empty string returns all keys.
        partial: String,
    },
    /// Emit vendor prefixes matching a partial prefix.
    VendorPrefix { partial: String },
    /// Emit context names matching a partial prefix. Pure-local
    /// (no portal call); always works.
    ContextName { partial: String },
}

pub async fn run(ctx: &super::Ctx, op: CompleteCmd) -> anyhow::Result<()> {
    // Suppress errors — completion hooks must never fail loudly
    // (would corrupt the interactive prompt). Empty result on any
    // failure is the right interactive behavior.
    match op {
        CompleteCmd::TopicKey { partial } => {
            if let Ok(portal) = ctx.profile.portal_client() {
                if let Ok(vendors) = portal.list_vendors().await {
                    for v in vendors {
                        if let Ok(topics) = portal.list_topics(&v.prefix).await {
                            for t in topics {
                                if t.key_pattern.starts_with(&partial) {
                                    println!("{}", t.key_pattern);
                                }
                            }
                        }
                    }
                }
            }
        }
        CompleteCmd::VendorPrefix { partial } => {
            if let Ok(portal) = ctx.profile.portal_client() {
                if let Ok(vendors) = portal.list_vendors().await {
                    for v in vendors {
                        if v.prefix.starts_with(&partial) {
                            println!("{}", v.prefix);
                        }
                    }
                }
            }
        }
        CompleteCmd::ContextName { partial } => {
            if let Ok(contexts) = super::context::list_contexts(&ctx.profile) {
                for name in contexts
                    .into_iter()
                    .filter(|name| name.starts_with(&partial))
                {
                    println!("{name}");
                }
            }
        }
    }
    Ok(())
}
