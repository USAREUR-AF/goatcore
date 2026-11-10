// `diff` — the verb an operator reaches for at the moment two digests
// disagree, which is this design's one loud failure (§9).
//
// A raw structural dump is not useful at that moment. Two things are:
//
//   1. The NAME of the differing field. "Loud, in the first hour, instead of
//      silent for three weeks" is only true if the offending field names
//      itself.
//   2. An answer for the case §9 calls out explicitly and the one that reads
//      as impossible: the digests differ AND the two printed agreements are
//      identical. The diff above then has nothing to show, and the instinct is
//      to re-run everything. The measurement to take instead is ONE number —
//      the two byte lengths. A difference of one is a trailing newline; three
//      is a byte-order mark; equal lengths means the same characters were
//      encoded differently.
//
// This module prints the conclusion, not the lengths.

use super::compose::digest_of;
use super::json::{canonical, parse_document, utf16_cmp, Finding, JVal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug)]
pub enum DiffOutcome {
    /// Equal digests: both sides hold the identical agreement, and
    /// transcription error is now impossible.
    Identical { digest: String },
    /// Digests differ, agreements do not — the disagreement is in the bytes
    /// hashed, not in the agreement.
    Bytes(ByteFinding),
    /// The agreements genuinely differ. Each entry names one field.
    Fields(Vec<FieldDiff>),
    /// One of the two inputs is not a readable agreement.
    Unreadable { side: Side, finding: Finding },
}

#[derive(Debug)]
pub struct ByteFinding {
    pub cause: ByteCause,
    /// The sentence to print: what the difference IS, not what was measured.
    pub conclusion: String,
    pub fix: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ByteCause {
    TrailingNewline {
        longer: Side,
    },
    ByteOrderMark {
        longer: Side,
    },
    EncodingDifference {
        side: Option<Side>,
    },
    /// One of the two files is not its own canonical form, so it holds the
    /// right agreement and the wrong bytes. §9's length rule assumes both
    /// sides emitted the canonical form; where that premise does not hold this
    /// is the more precise answer, and "equal lengths" is not conclusive.
    NotCanonical {
        side: Side,
    },
}

#[derive(Debug, Clone)]
pub struct FieldDiff {
    pub path: String,
    pub left: String,
    pub right: String,
}

const ABSENT: &str = "(absent)";

/// Compare two composed agreements, as the bytes each side actually hashed.
///
/// The inputs are byte slices rather than parsed values on purpose: half the
/// answers this verb gives are about the bytes, and a caller that had already
/// parsed them would have thrown that away.
pub fn diff(left: &[u8], right: &[u8], left_label: &str, right_label: &str) -> DiffOutcome {
    let dl = digest_of(left);
    let dr = digest_of(right);
    if dl == dr {
        return DiffOutcome::Identical { digest: dl };
    }

    // §9's encoding case, and the only form of it that is actually reachable:
    // one side's bytes are not valid UTF-8. Two files that are each the
    // canonical form of the same agreement are byte-identical by definition,
    // so a same-length disagreement between two well-formed sides cannot
    // happen — what does happen is that one side wrote the same characters in
    // some other encoding, and that is not UTF-8 at all. Caught here rather
    // than reported as an unreadable file, because "unreadable" sends someone
    // to look at the agreement, and the agreement is fine.
    let l_utf8 = std::str::from_utf8(strip_framing(left)).is_ok();
    let r_utf8 = std::str::from_utf8(strip_framing(right)).is_ok();
    if l_utf8 != r_utf8 {
        let side = if l_utf8 { Side::Right } else { Side::Left };
        let which = if l_utf8 { right_label } else { left_label };
        return DiffOutcome::Bytes(ByteFinding {
            cause: ByteCause::EncodingDifference { side: Some(side) },
            conclusion: format!(
                "{which} is not valid UTF-8, so the two sides encoded the same characters \
                 differently. §4 says UTF-8."
            ),
            fix: "Emit the canonical form as UTF-8. Do not decode the other side's file with a \
                  replacing decoder in order to compare them: substituting U+FFFD is silent, \
                  changes the bytes and therefore changes the digest, with nothing reported at \
                  either end."
                .to_string(),
        });
    }

    // Parse from a framing-stripped copy, and keep the ORIGINAL bytes for the
    // length measurement. This is the whole reason the third case is
    // diagnosable: a byte-order mark and a trailing newline are refusals in a
    // package, and refusing them here would answer "unreadable" to precisely
    // the question this verb exists to answer.
    let lv = parse_document(strip_framing(left));
    let rv = parse_document(strip_framing(right));

    // §9's third case in the form it actually takes. "Equal lengths with
    // differing digests means the two sides encoded the same characters
    // differently" cannot arise between two well-formed sides — two canonical
    // forms of one value are byte-identical — so what reaches an operator is
    // one file that will not parse. Answering "unreadable" there sends them to
    // look at the agreement, and the agreement is fine. So before giving up on
    // a side, re-decode it from the encodings people actually hit and ask
    // whether it is the SAME agreement in other bytes.
    match (&lv, &rv) {
        (Ok(good), Err(f)) => {
            if let Some(enc) = same_agreement_in_another_encoding(right, good) {
                return DiffOutcome::Bytes(encoding_finding(Side::Right, right_label, enc));
            }
            return DiffOutcome::Unreadable {
                side: Side::Right,
                finding: f.clone(),
            };
        }
        (Err(f), Ok(good)) => {
            if let Some(enc) = same_agreement_in_another_encoding(left, good) {
                return DiffOutcome::Bytes(encoding_finding(Side::Left, left_label, enc));
            }
            return DiffOutcome::Unreadable {
                side: Side::Left,
                finding: f.clone(),
            };
        }
        (Err(f), Err(_)) => {
            return DiffOutcome::Unreadable {
                side: Side::Left,
                finding: f.clone(),
            }
        }
        (Ok(_), Ok(_)) => {}
    }
    let (lv, rv) = (lv.expect("both Ok"), rv.expect("both Ok"));

    let lc = canonical(&lv);
    let rc = canonical(&rv);
    if lc == rc {
        return DiffOutcome::Bytes(byte_finding(left, right, &lc, left_label, right_label));
    }

    let mut out = Vec::new();
    walk(&lv, &rv, "", &mut out);
    DiffOutcome::Fields(out)
}

/// Is this the same agreement, written in some encoding other than UTF-8?
/// UTF-16 in both byte orders is what turns up, and an ASCII document in
/// UTF-16LE is even valid UTF-8 byte-for-byte — its padding bytes are U+0000 —
/// so a validity check alone never sees it.
fn same_agreement_in_another_encoding(bytes: &[u8], good: &JVal) -> Option<&'static str> {
    let want = canonical(good);
    for (name, decoded) in [
        ("UTF-16LE", decode_utf16(bytes, u16::from_le_bytes)),
        ("UTF-16BE", decode_utf16(bytes, u16::from_be_bytes)),
    ] {
        let Some(text) = decoded else { continue };
        if let Ok(v) = parse_document(strip_framing(text.as_bytes())) {
            if canonical(&v) == want {
                return Some(name);
            }
        }
    }
    None
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    if !remainder.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().copied().map(unit).collect();
    String::from_utf16(&units).ok()
}

fn encoding_finding(side: Side, which: &str, encoding: &'static str) -> ByteFinding {
    ByteFinding {
        cause: ByteCause::EncodingDifference { side: Some(side) },
        conclusion: format!(
            "The two agreements are the same agreement. {which} is written in {encoding}, so the \
             two sides encoded the same characters differently and hashed different bytes. §4 \
             says UTF-8."
        ),
        fix: "Emit the canonical form as UTF-8 (§4, §9). Nothing about the agreement is wrong — \
              only the bytes one side hashed."
            .to_string(),
    }
}

/// A leading byte-order mark and trailing whitespace, removed for the PARSE
/// only. Both are refusals everywhere else in this implementation; here they
/// are the answer rather than the error.
fn strip_framing(b: &[u8]) -> &[u8] {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    let mut end = b.len();
    while end > 0 && b[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &b[..end]
}

/// §9's third case. The conclusion is stated; the raw lengths are not, because
/// the number is a means to the conclusion and printing it invites the reader
/// to re-derive what has already been derived.
fn byte_finding(
    left: &[u8],
    right: &[u8],
    canonical_form: &str,
    left_label: &str,
    right_label: &str,
) -> ByteFinding {
    let (longer_side, longer, shorter) = if left.len() >= right.len() {
        (Side::Left, left, right)
    } else {
        (Side::Right, right, left)
    };
    let longer_label = match longer_side {
        Side::Left => left_label,
        Side::Right => right_label,
    };
    let delta = longer.len() - shorter.len();

    // Take the measurement §9 names, then CONFIRM it against the bytes rather
    // than reporting the inference alone. The length rule is what points at
    // the cause; the bytes are what settle it.
    if delta == 1 && longer.ends_with(b"\n") && &longer[..longer.len() - 1] == shorter {
        return ByteFinding {
            cause: ByteCause::TrailingNewline {
                longer: longer_side,
            },
            conclusion: format!(
                "The two agreements are the same agreement. {longer_label} carries a trailing \
                 newline and the other side does not, so the two of you hashed different bytes."
            ),
            fix: "Hash the canonical form itself: the UTF-8 bytes, with no trailing newline (§9). \
                  A canonical form is a byte string, not a text file — writing it out is where \
                  editors, `echo`, here-documents and several standard libraries add a newline on \
                  your behalf."
                .to_string(),
        };
    }
    if delta == 3 && longer.starts_with(&[0xEF, 0xBB, 0xBF]) && &longer[3..] == shorter {
        return ByteFinding {
            cause: ByteCause::ByteOrderMark {
                longer: longer_side,
            },
            conclusion: format!(
                "The two agreements are the same agreement. {longer_label} begins with a UTF-8 \
                 byte-order mark and the other side does not, so the two of you hashed different \
                 bytes."
            ),
            fix: "Hash the canonical form with no byte-order mark (§9). A BOM is usually added by \
                  the writer, not the composer — check how the file was saved."
                .to_string(),
        };
    }
    // §9's length rule assumes both sides emitted the canonical form. Where one
    // did not, that is the answer, and it is a better one than the rule gives —
    // including at equal lengths, where two files of the same size whose
    // members are in different orders would otherwise be misreported as an
    // encoding fault.
    let c = canonical_form.as_bytes();
    let not_canonical = match (left == c, right == c) {
        (true, false) => Some((Side::Right, right_label)),
        (false, true) => Some((Side::Left, left_label)),
        _ => None,
    };
    if let Some((side, which)) = not_canonical {
        return ByteFinding {
            cause: ByteCause::NotCanonical { side },
            conclusion: format!(
                "The two agreements are the same agreement. {which} is not the canonical form of \
                 it — most likely an indented rendering, or members in the order they were \
                 written rather than sorted — so that side hashed the right agreement and the \
                 wrong bytes."
            ),
            fix: "Hash the canonical form (§4): members sorted by UTF-16 code unit, no \
                  insignificant whitespace, UTF-8, no trailing newline. An indented file is a \
                  reading aid and is never an input to a digest."
                .to_string(),
        };
    }
    if delta == 0 {
        return ByteFinding {
            cause: ByteCause::EncodingDifference { side: None },
            conclusion: "The two agreements are the same agreement and the two files are the \
                         same length, so the two sides encoded the same characters differently. \
                         §4 says UTF-8."
                .to_string(),
            fix: "Emit the canonical form as UTF-8, and check for a decoder that substituted \
                  U+FFFD for input it could not read — that substitution is silent, changes the \
                  bytes and therefore changes the digest, with nothing reported on either side."
                .to_string(),
        };
    }
    // Both sides ARE the canonical form and the lengths still differ, which
    // cannot happen: two canonical forms of one value are byte-identical.
    // Reported rather than asserted, because a confident wrong answer here is
    // worse than an admission.
    let _ = longer_side;
    ByteFinding {
        cause: ByteCause::EncodingDifference { side: None },
        conclusion: format!(
            "The two agreements are the same agreement, both files are the canonical form of it, \
             and {longer_label} is still {delta} bytes longer. That combination should not be \
             reachable; treat it as a defect in this tool rather than in the agreement."
        ),
        fix: "Report it, with both files.".to_string(),
    }
}

/// Name the differing field. The path is the one an operator can look up in
/// §8.1 and quote to the counterparty.
fn walk(l: &JVal, r: &JVal, path: &str, out: &mut Vec<FieldDiff>) {
    match (l, r) {
        (JVal::Obj(lm), JVal::Obj(rm)) => {
            let mut names: Vec<&str> = lm.iter().map(|(k, _)| k.as_str()).collect();
            for (k, _) in rm {
                if !names.contains(&k.as_str()) {
                    names.push(k);
                }
            }
            names.sort_by(|a, b| utf16_cmp(a, b));
            for name in names {
                let child = if path.is_empty() {
                    name.to_string()
                } else {
                    format!("{path}.{name}")
                };
                match (l.get(name), r.get(name)) {
                    (Some(a), Some(b)) => walk(a, b, &child, out),
                    (Some(a), None) => out.push(FieldDiff {
                        path: child,
                        left: render(a),
                        right: ABSENT.into(),
                    }),
                    (None, Some(b)) => out.push(FieldDiff {
                        path: child,
                        left: ABSENT.into(),
                        right: render(b),
                    }),
                    (None, None) => unreachable!("name came from one of the two objects"),
                }
            }
        }
        (JVal::Arr(la), JVal::Arr(ra)) => {
            // `parties` and `flows` have their order fixed by §8.1, so a
            // positional comparison names the right entry rather than guessing
            // at an alignment.
            for n in 0..la.len().max(ra.len()) {
                let child = format!("{path}[{n}]");
                match (la.get(n), ra.get(n)) {
                    (Some(a), Some(b)) => walk(a, b, &child, out),
                    (Some(a), None) => out.push(FieldDiff {
                        path: child,
                        left: render(a),
                        right: ABSENT.into(),
                    }),
                    (None, Some(b)) => out.push(FieldDiff {
                        path: child,
                        left: ABSENT.into(),
                        right: render(b),
                    }),
                    (None, None) => unreachable!("n is below at least one length"),
                }
            }
        }
        (a, b) if a == b => {}
        (a, b) => out.push(FieldDiff {
            path: path.to_string(),
            left: render(a),
            right: render(b),
        }),
    }
}

fn render(v: &JVal) -> String {
    canonical(v)
}
