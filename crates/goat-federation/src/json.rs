// Strict JSON reader + RFC 8785 (JCS) writer for federation packages.
//
// WHY NOT serde_json. The specification's number rules are stated against the
// TOKEN AS WRITTEN, not the parsed value (spec §4): `1380.0`, `01`, `+1` and
// `-0` are each refused although three of them denote 1380/1/0 exactly. A
// parser that hands on a value has already lost the answer — serde_json turns
// `-0` into the integer 0 and keeps the LAST of two members with the same name,
// which is precisely the divergence §4 spends a page arguing against. Two other
// rules need the same access: a duplicate member name must be REFUSED rather
// than resolved, and an unpaired surrogate must be refused rather than replaced
// with U+FFFD.
//
// So the reader below is a byte-level recursive descent over the document text.
// It is small because the grammar is small, and it is the only place in this
// crate that decides whether a package is well-formed.
//
// The value model carries no Null and no Float variant on purpose: both are
// refusals at any depth (§4), so representing them would mean carrying a value
// that can never be legal downstream.

use std::cmp::Ordering;

/// The largest integer every mainstream runtime represents exactly, 2^53 − 1.
/// Spec §4 bounds every integer in a package and in an agreement to ±this.
pub const INT_MAX: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq)]
pub enum JVal {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<JVal>),
    /// Member order as written. The canonical writer sorts; the reader does
    /// not, so `lint` can report a package that is not already in canonical
    /// form.
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JVal::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            JVal::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JVal::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[JVal]> {
        match self {
            JVal::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&[(String, JVal)]> {
        match self {
            JVal::Obj(o) => Some(o),
            _ => None,
        }
    }
    pub fn get(&self, name: &str) -> Option<&JVal> {
        self.as_obj()?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            JVal::Str(_) => "string",
            JVal::Int(_) => "integer",
            JVal::Bool(_) => "boolean",
            JVal::Arr(_) => "array",
            JVal::Obj(_) => "object",
        }
    }
    /// A copy with one top-level member removed. Used to build the signing
    /// input, which is the canonical form with `signature` taken out (§5).
    pub fn without_member(&self, name: &str) -> JVal {
        match self {
            JVal::Obj(members) => {
                JVal::Obj(members.iter().filter(|(k, _)| k != name).cloned().collect())
            }
            other => other.clone(),
        }
    }
}

/// One reason an input was refused, naming the rule that fired.
///
/// §10 requires an implementation to report WHICH rule fired: "a fixture
/// refused for the wrong reason is a passing test hiding a broken
/// implementation". `rule` is the stable code the test suite asserts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    pub spec: &'static str,
    pub path: String,
    pub detail: String,
}

impl Finding {
    pub fn new(
        rule: &'static str,
        spec: &'static str,
        path: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Finding {
            rule,
            spec,
            path: path.into(),
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{} ({}) — {}", self.rule, self.spec, self.detail)
        } else {
            write!(
                f,
                "{} ({}) at `{}` — {}",
                self.rule, self.spec, self.path, self.detail
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

/// The one comparison this format uses, everywhere: by UTF-16 code unit,
/// one at a time, shorter string first where one is a prefix of the other
/// (spec §4, "One ordering, everywhere").
///
/// Rust's `str` ordering is byte-wise UTF-8, which equals code-point order and
/// therefore DIFFERS from this rule on any character above the basic
/// multilingual plane — a surrogate pair begins D800–DBFF, so UTF-16 places it
/// before U+E000–U+FFFF where a code-point comparison places it after. The
/// `compose-collation-utf16` vector is the fixture that separates them.
pub fn utf16_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// De-duplicate and sort a set-array by §4's ordering.
pub fn normalise_set(items: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for s in items {
        if !out.iter().any(|e| e == s) {
            out.push(s.clone());
        }
    }
    out.sort_by(|a, b| utf16_cmp(a, b));
    out
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

/// Parse a complete document. The top level must be an object.
///
/// The byte-order-mark and UTF-8 checks come first because both change the
/// bytes that get hashed, and a decoder that repairs either has composed a
/// document that was never signed (§4).
pub fn parse_document(bytes: &[u8]) -> Result<JVal, Finding> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err(Finding::new(
            "M_BOM",
            "§4/§9",
            "",
            "input begins with a UTF-8 byte-order mark; the canonical form is bytes with no BOM",
        ));
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err(Finding::new(
            "M_NOT_UTF8",
            "§4",
            "",
            "input is not valid UTF-8. It is refused, never repaired: substituting U+FFFD is \
             silent, changes the bytes and therefore changes the digest",
        ));
    }
    let mut r = Reader { s: bytes, i: 0 };
    r.ws();
    let v = r.value("")?;
    r.ws();
    if r.i != r.s.len() {
        return Err(Finding::new(
            "M_JSON_SYNTAX",
            "§4",
            "",
            format!("trailing content after the top-level value at byte {}", r.i),
        ));
    }
    if !matches!(v, JVal::Obj(_)) {
        return Err(Finding::new(
            "M_NOT_OBJECT",
            "§3",
            "",
            format!(
                "the top-level value is a {}; a package is one JSON object",
                v.type_name()
            ),
        ));
    }
    Ok(v)
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn syntax(&self, path: &str, detail: impl Into<String>) -> Finding {
        Finding::new("M_JSON_SYNTAX", "§4", path, detail)
    }

    fn value(&mut self, path: &str) -> Result<JVal, Finding> {
        match self.peek() {
            None => Err(self.syntax(path, "input ended where a value was expected")),
            Some(b'{') => self.object(path),
            Some(b'[') => self.array(path),
            Some(b'"') => Ok(JVal::Str(self.string(path)?)),
            Some(b't') => {
                self.literal(b"true", path)?;
                Ok(JVal::Bool(true))
            }
            Some(b'f') => {
                self.literal(b"false", path)?;
                Ok(JVal::Bool(false))
            }
            // A null is an error at any depth, on input as much as on output
            // (§4). It is refused rather than normalised away, because the
            // signature covers the null: a receiver that dropped it would
            // verify one document and compose a different one.
            Some(b'n') => {
                self.literal(b"null", path)?;
                Err(Finding::new(
                    "M_NULL",
                    "§4",
                    path,
                    "a null is an error at any depth — not an absent member, and not a member \
                     to be normalised away. The signature covers it",
                ))
            }
            Some(b'+') => Err(Finding::new(
                "M_LEADING_PLUS",
                "§4",
                path,
                "a number token with a leading `+` is not JSON; a lenient parser that accepts \
                 it hands on a value whose canonical form is a different token",
            )),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(path),
            Some(c) => Err(self.syntax(
                path,
                format!("unexpected byte {:?} where a value was expected", c as char),
            )),
        }
    }

    fn literal(&mut self, word: &[u8], path: &str) -> Result<(), Finding> {
        if self.s.len() >= self.i + word.len() && &self.s[self.i..self.i + word.len()] == word {
            self.i += word.len();
            Ok(())
        } else {
            Err(self.syntax(path, "malformed literal"))
        }
    }

    /// Number tokens, classified on the TEXT as written (§4). The parsed value
    /// cannot answer these questions: `1380.0` and `1380` are the same number,
    /// `-0` and `0` are the same number, and `01` and `1` are the same number.
    fn number(&mut self, path: &str) -> Result<JVal, Finding> {
        let start = self.i;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.i += 1;
        }
        let digits_start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.i == digits_start {
            return Err(self.syntax(path, "a number token with no digits"));
        }
        let int_part = &self.s[digits_start..self.i];

        // Consume any fractional part / exponent so the whole token can be
        // reported, then refuse it — never coerce. Coercion makes the set of
        // accepted documents depend on how each language's parser widens
        // numbers, which is the disagreement §4 exists to remove.
        let mut fractional = false;
        if self.peek() == Some(b'.') {
            fractional = true;
            self.i += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            fractional = true;
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        let token = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();

        if fractional {
            return Err(Finding::new(
                "M_NON_INTEGER",
                "§4",
                path,
                format!(
                    "`{token}` carries a fractional part or an exponent. Integers only, and the \
                     test is on the token as written — exactly integral though it is, it is \
                     refused, not coerced"
                ),
            ));
        }
        if int_part.len() > 1 && int_part[0] == b'0' {
            return Err(Finding::new(
                "M_LEADING_ZERO",
                "§4",
                path,
                format!("`{token}` has a leading zero, which is not JSON at all"),
            ));
        }
        if negative && int_part == b"0" {
            return Err(Finding::new(
                "M_NEGATIVE_ZERO",
                "§4",
                path,
                "`-0` is valid JSON and JCS emits it as `0`, so a package carrying it is not in \
                 canonical form and an implementation that passes the token through produces \
                 different bytes for the same value",
            ));
        }
        let parsed: i64 = token.parse().map_err(|_| {
            Finding::new(
                "M_INT_RANGE",
                "§4",
                path,
                format!("`{token}` is outside the closed range ±(2^53 − 1)"),
            )
        })?;
        if !(-INT_MAX..=INT_MAX).contains(&parsed) {
            return Err(Finding::new(
                "M_INT_RANGE",
                "§4",
                path,
                format!(
                    "`{token}` is outside the closed range ±{INT_MAX}. Above it, JavaScript holds \
                     the value as an IEEE-754 double, loses precision silently and re-serialises \
                     to different bytes — a digest mismatch with no wrong value anywhere to find"
                ),
            ));
        }
        Ok(JVal::Int(parsed))
    }

    fn string(&mut self, path: &str) -> Result<String, Finding> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self
                .peek()
                .ok_or_else(|| self.syntax(path, "input ended inside a string"))?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = self
                        .peek()
                        .ok_or_else(|| self.syntax(path, "input ended inside an escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape(path)?),
                        other => {
                            return Err(
                                self.syntax(path, format!("unknown escape `\\{}`", other as char))
                            )
                        }
                    }
                }
                // Raw control characters are not JSON.
                c if c < 0x20 => {
                    return Err(self.syntax(
                        path,
                        format!("a raw control character U+{c:04X} inside a string"),
                    ))
                }
                _ => {
                    // Multi-byte UTF-8 is already validated for the whole
                    // document, so a char boundary walk is safe here.
                    let rest = std::str::from_utf8(&self.s[self.i..])
                        .map_err(|_| self.syntax(path, "invalid UTF-8 inside a string"))?;
                    let ch = rest.chars().next().expect("non-empty by the match above");
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    /// `\uXXXX`. An escape naming an unpaired surrogate is valid JSON SYNTAX
    /// and denotes no Unicode scalar value, so it has no UTF-8 encoding at all
    /// — the package is refused, never repaired to U+FFFD (§4).
    fn unicode_escape(&mut self, path: &str) -> Result<char, Finding> {
        let hi = self.hex4(path)?;
        if (0xD800..0xDC00).contains(&hi) {
            // High surrogate: a low surrogate must follow immediately.
            let paired =
                self.s.get(self.i) == Some(&b'\\') && self.s.get(self.i + 1) == Some(&b'u');
            if !paired {
                return Err(unpaired(path, hi));
            }
            self.i += 2;
            let lo = self.hex4(path)?;
            if !(0xDC00..0xE000).contains(&lo) {
                return Err(unpaired(path, hi));
            }
            let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
            return char::from_u32(cp).ok_or_else(|| unpaired(path, hi));
        }
        if (0xDC00..0xE000).contains(&hi) {
            return Err(unpaired(path, hi));
        }
        char::from_u32(hi).ok_or_else(|| unpaired(path, hi))
    }

    fn hex4(&mut self, path: &str) -> Result<u32, Finding> {
        if self.i + 4 > self.s.len() {
            return Err(self.syntax(path, "a truncated \\u escape"));
        }
        let mut v: u32 = 0;
        for k in 0..4 {
            let c = self.s[self.i + k];
            let d = match c {
                b'0'..=b'9' => u32::from(c - b'0'),
                b'a'..=b'f' => u32::from(c - b'a') + 10,
                b'A'..=b'F' => u32::from(c - b'A') + 10,
                _ => return Err(self.syntax(path, "a non-hex digit in a \\u escape")),
            };
            v = (v << 4) | d;
        }
        self.i += 4;
        Ok(v)
    }

    fn array(&mut self, path: &str) -> Result<JVal, Finding> {
        self.i += 1; // '['
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(JVal::Arr(items));
        }
        loop {
            self.ws();
            let child = format!("{path}[{}]", items.len());
            self.empty_value_guard(&child)?;
            items.push(self.value(&child)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(JVal::Arr(items));
                }
                _ => return Err(self.syntax(path, "expected `,` or `]` in an array")),
            }
        }
    }

    fn object(&mut self, path: &str) -> Result<JVal, Finding> {
        self.i += 1; // '{'
        let mut members: Vec<(String, JVal)> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(JVal::Obj(members));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.syntax(path, "expected a member name"));
            }
            let name = self.string(path)?;
            let child = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            // A member name occurs at most once. A parser handed the same name
            // twice MUST refuse, never resolve: Go's decoder keeps the last and
            // several others keep the first, so one file composes into two
            // different agreements and each party believes its own (§4).
            if members.iter().any(|(k, _)| k == &name) {
                return Err(Finding::new(
                    "M_DUPLICATE_MEMBER",
                    "§4",
                    child,
                    format!(
                        "member `{name}` occurs more than once. It is refused, never resolved — \
                         resolving it is how two conforming implementations reach two digests"
                    ),
                ));
            }
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.syntax(&child, "expected `:` after a member name"));
            }
            self.i += 1;
            self.ws();
            self.empty_value_guard(&child)?;
            let v = self.value(&child)?;
            members.push((name, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(JVal::Obj(members));
                }
                _ => return Err(self.syntax(path, "expected `,` or `}` in an object")),
            }
        }
    }

    /// A member with no value at all — `"measured_mtu":,` — is a syntax error
    /// in any parser, but §4 calls it out by name and calls the corresponding
    /// output "the worst failure this format can have, because both sides can
    /// agree on it". Naming it separately keeps the finding attributable to
    /// that rule rather than to a generic parse failure.
    fn empty_value_guard(&mut self, path: &str) -> Result<(), Finding> {
        if matches!(self.peek(), Some(b',') | Some(b'}') | Some(b']')) {
            return Err(Finding::new(
                "M_EMPTY_VALUE",
                "§4",
                path,
                "a member with no value. There is no empty token: a member is either present \
                 with a value or absent, and a canonicaliser handed one must return an error \
                 rather than produce output",
            ));
        }
        Ok(())
    }
}

fn unpaired(path: &str, unit: u32) -> Finding {
    Finding::new(
        "M_UNPAIRED_SURROGATE",
        "§4",
        path,
        format!(
            "the escape \\u{unit:04x} names an unpaired surrogate, which denotes no Unicode \
             scalar value and has no UTF-8 encoding. Refused, never replaced with U+FFFD — \
             substitution is silent, changes the bytes and therefore changes the digest"
        ),
    )
}

// ---------------------------------------------------------------------------
// Writer — RFC 8785 (JCS)
// ---------------------------------------------------------------------------

/// The canonical form: members sorted by UTF-16 code unit, no insignificant
/// whitespace, UTF-8 output, no trailing newline and no byte-order mark (§4, §9).
pub fn canonical(v: &JVal) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &JVal, out: &mut String) {
    match v {
        JVal::Str(s) => write_string(s, out),
        JVal::Int(i) => out.push_str(&i.to_string()),
        JVal::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        JVal::Arr(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        JVal::Obj(members) => {
            let mut sorted: Vec<&(String, JVal)> = members.iter().collect();
            sorted.sort_by(|a, b| utf16_cmp(&a.0, &b.0));
            out.push('{');
            for (n, (k, val)) in sorted.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(val, out);
            }
            out.push('}');
        }
    }
}

/// JCS string escaping: the two mandatory escapes, the six short forms, and
/// `\u00xx` for the remaining control characters. Everything else is emitted
/// as literal UTF-8 — JCS does not escape non-ASCII.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}
