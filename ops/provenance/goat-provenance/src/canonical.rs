//! Canonical JSON: object keys sorted by their bytes, no insignificant
//! whitespace, strings escaped minimally (`"`, `\`, and control characters;
//! everything else as UTF-8).
//!
//! Written out by hand rather than relying on `serde_json`'s map order: the
//! `preserve_order` feature is enabled elsewhere in this workspace, and Cargo
//! unifies features, so `serde_json::Map` keeps insertion order here.
//!
//! A Go producer gets the same bytes from `encoding/json` with sorted map keys
//! only if it turns HTML escaping off (`Encoder.SetEscapeHTML(false)`); the
//! default encoder writes `<`, `>`, `&` as `<`, `>`, `&`.

use serde_json::Value;

/// The canonical serialization of `v`.
pub fn to_vec(v: &Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    write(v, &mut out);
    out
}

fn write(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => write_str(s, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write(item, out);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_str(k, out);
                out.push(b':');
                write(&map[k.as_str()], out);
            }
            out.push(b'}');
        }
    }
}

fn write_str(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\u{08}' => out.extend_from_slice(b"\\b"),
            '\u{0c}' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_at_every_depth_and_whitespace_is_gone() {
        let v = json!({"z": 1, "a": {"y": [true, null], "b": "x"}, "m": -3});
        assert_eq!(
            String::from_utf8(to_vec(&v)).unwrap(),
            r#"{"a":{"b":"x","y":[true,null]},"m":-3,"z":1}"#
        );
    }

    #[test]
    fn insertion_order_does_not_change_the_bytes() {
        let mut a = serde_json::Map::new();
        a.insert("b".into(), json!(2));
        a.insert("a".into(), json!(1));
        let mut b = serde_json::Map::new();
        b.insert("a".into(), json!(1));
        b.insert("b".into(), json!(2));
        assert_eq!(to_vec(&Value::Object(a)), to_vec(&Value::Object(b)));
    }

    #[test]
    fn strings_escape_only_what_json_requires() {
        let v = json!({"k": "a\"b\\c\nd\u{01}<&>é"});
        assert_eq!(
            String::from_utf8(to_vec(&v)).unwrap(),
            "{\"k\":\"a\\\"b\\\\c\\nd\\u0001<&>é\"}"
        );
    }

    #[test]
    fn keys_sort_by_bytes_not_by_locale() {
        let v = json!({"b": 1, "B": 2, "_": 3});
        assert_eq!(
            String::from_utf8(to_vec(&v)).unwrap(),
            r#"{"B":2,"_":3,"b":1}"#
        );
    }
}
