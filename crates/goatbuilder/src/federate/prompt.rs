//! Asking a person, at a terminal, or answering from a script in a test.
//!
//! Questions and the text around them go to stderr, so `-o json` on stdout
//! stays one document. Every confirmation needs a typed answer: an empty line,
//! end of input or anything unexpected means "stop here", never "yes".

#[cfg(test)]
use std::collections::VecDeque;
use std::io::{BufRead, IsTerminal, Write};

pub trait Ask {
    /// Whether a person can answer. Without one, a confirmation needs its
    /// value by flag, and nothing is ever taken as approved.
    fn interactive(&self) -> bool;
    /// Show text to the person.
    fn say(&mut self, text: &str);
    /// Ask one question; `None` at the end of input.
    fn ask(&mut self, question: &str) -> Option<String>;
}

/// The person at this terminal: questions on stderr, answers from stdin.
pub struct Terminal;

impl Terminal {
    /// Both ends have to be a terminal: a question printed into a pipe is a
    /// question nobody reads.
    pub fn present() -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }
}

impl Ask for Terminal {
    fn interactive(&self) -> bool {
        Terminal::present()
    }
    fn say(&mut self, text: &str) {
        eprint!("{text}");
        if !text.ends_with('\n') {
            eprintln!();
        }
    }
    fn ask(&mut self, question: &str) -> Option<String> {
        eprint!("{question} ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_string()),
        }
    }
}

/// No person: every question goes unanswered.
pub struct Nobody;

impl Ask for Nobody {
    fn interactive(&self) -> bool {
        false
    }
    fn say(&mut self, _text: &str) {}
    fn ask(&mut self, _question: &str) -> Option<String> {
        None
    }
}

/// Answers from a script, and a transcript of what was shown; for tests.
#[cfg(test)]
pub struct Scripted {
    pub answers: VecDeque<String>,
    pub transcript: String,
}

#[cfg(test)]
impl Scripted {
    pub fn new(answers: &[&str]) -> Self {
        Scripted {
            answers: answers.iter().map(|s| s.to_string()).collect(),
            transcript: String::new(),
        }
    }
}

#[cfg(test)]
impl Ask for Scripted {
    fn interactive(&self) -> bool {
        true
    }
    fn say(&mut self, text: &str) {
        self.transcript.push_str(text);
        if !text.ends_with('\n') {
            self.transcript.push('\n');
        }
    }
    fn ask(&mut self, question: &str) -> Option<String> {
        self.transcript.push_str(question);
        self.transcript.push('\n');
        self.answers.pop_front().map(|a| a.trim().to_string())
    }
}

/// A key or digest as it is read aloud: groups of four, the group count first.
pub fn groups(s: &str) -> String {
    let body = s.strip_prefix("sha256:").unwrap_or(s);
    let chars: Vec<char> = body.chars().collect();
    let g: Vec<String> = chars.chunks(4).map(|c| c.iter().collect()).collect();
    format!("{} groups: {}", g.len(), g.join(" "))
}

/// The same value with its spacing and a `sha256:` prefix ignored, so a
/// person may type it the way it was read aloud.
pub fn same_value(a: &str, b: &str) -> bool {
    let norm = |s: &str| -> String {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        s.strip_prefix("sha256:").unwrap_or(&s).to_ascii_lowercase()
    };
    !a.trim().is_empty() && norm(a) == norm(b)
}

/// A yes that has to be typed as the word.
pub fn is_yes(answer: &Option<String>) -> bool {
    answer
        .as_deref()
        .map(|a| a.eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_compare_the_way_they_are_read_aloud() {
        assert_eq!(groups("sha256:abcdef0123"), "3 groups: abcd ef01 23");
        assert!(same_value("ABCD ef01 23", "sha256:abcdef0123"));
        assert!(!same_value("", "sha256:"));
    }

    #[test]
    fn a_script_answers_in_order_and_keeps_what_was_shown() {
        let mut sc = Scripted::new(&["yes"]);
        sc.say("shown");
        assert_eq!(sc.ask("q?").as_deref(), Some("yes"));
        assert_eq!(sc.ask("again?"), None, "the end of input is no answer");
        assert!(sc.transcript.contains("shown") && sc.transcript.contains("q?"));
    }

    #[test]
    fn only_the_typed_word_is_a_yes() {
        assert!(is_yes(&Some("yes".into())) && is_yes(&Some("YES".into())));
        for no in [
            None,
            Some(String::new()),
            Some("y".into()),
            Some("sure".into()),
        ] {
            assert!(!is_yes(&no), "{no:?}");
        }
    }
}
