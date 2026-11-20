//! ASTERIX block framing, ported from the legacy-feed carriage demonstration
//! (USAREUR-AF/hello-galaxy-legacy-feed `lib/asterixlint.py`, the same walk
//! as goatcore `ops/ci/check-topic-decode-health.py`).
//!
//! A data block is one octet CATEGORY, two octets LENGTH (big-endian, counting
//! the whole block including the 3-octet header), then records. A UDP datagram
//! carries one or more blocks end to end. Only the envelope is checked: a
//! block whose records are nonsense still frames.

use std::collections::BTreeSet;

/// `(category, start, len)` of every block, or why the framing cannot be true.
pub fn blocks(payload: &[u8]) -> Result<Vec<(u8, usize, usize)>, String> {
    let n = payload.len();
    if n < 3 {
        return Err(format!(
            "{n} byte(s) is shorter than one 3-octet block header"
        ));
    }
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < n {
        if n - off < 3 {
            return Err(format!(
                "{} trailing byte(s) after the last complete block",
                n - off
            ));
        }
        let cat = payload[off];
        let len = u16::from_be_bytes([payload[off + 1], payload[off + 2]]) as usize;
        if len < 3 {
            return Err(format!(
                "block at offset {off} declares LEN {len}, below the 3-octet header"
            ));
        }
        if off + len > n {
            return Err(format!(
                "block at offset {off} declares LEN {len} but only {} byte(s) remain",
                n - off
            ));
        }
        out.push((cat, off, len));
        off += len;
    }
    Ok(out)
}

/// `asterix:034,048` -> {34, 48}; `asterix` -> {} (every category).
pub fn declared_cats(format: &str) -> Result<BTreeSet<u8>, String> {
    let format = format.trim();
    let (kind, list) = match format.split_once(':') {
        Some((k, l)) => (k, Some(l)),
        None => (format, None),
    };
    if kind != "asterix" {
        return Err(format!(
            "format {format:?}: this adapter carries asterix only"
        ));
    }
    let mut out = BTreeSet::new();
    for part in list.into_iter().flat_map(|l| l.split(',')) {
        let p = part.trim().to_ascii_lowercase();
        let p = p.strip_prefix("cat").unwrap_or(&p);
        let cat: u8 = p
            .parse()
            .map_err(|_| format!("{part:?} is not an ASTERIX category number"))?;
        out.insert(cat);
    }
    Ok(out)
}

/// Check framing and declared categories, then group blocks by category in
/// order of first appearance. A frame that fails is not published.
pub fn lint_and_group(
    payload: &[u8],
    declared: &BTreeSet<u8>,
) -> Result<Vec<(u8, Vec<u8>)>, String> {
    let bl = blocks(payload).map_err(|r| format!("not ASTERIX block framing: {r}"))?;
    if !declared.is_empty() {
        if let Some((cat, _, _)) = bl.iter().find(|(c, _, _)| !declared.contains(c)) {
            return Err(format!(
                "block category {cat} present but the source declares {declared:?}"
            ));
        }
    }
    let mut groups: Vec<(u8, Vec<u8>)> = Vec::new();
    for (cat, start, len) in bl {
        let bytes = &payload[start..start + len];
        match groups.iter_mut().find(|(c, _)| *c == cat) {
            Some((_, buf)) => buf.extend_from_slice(bytes),
            None => groups.push((cat, bytes.to_vec())),
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(cat: u8, body: &[u8]) -> Vec<u8> {
        let len = (body.len() + 3) as u16;
        let mut b = vec![cat];
        b.extend_from_slice(&len.to_be_bytes());
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn framing_matches_the_shared_vectors() {
        assert_eq!(blocks(&block(48, &[1, 2, 3])).unwrap(), vec![(48, 0, 6)]);
        let two = [block(34, &[9]), block(48, &[1, 2])].concat();
        assert_eq!(blocks(&two).unwrap(), vec![(34, 0, 4), (48, 4, 5)]);
        assert!(blocks(&[48, 0]).unwrap_err().contains("shorter than one"));
        assert!(blocks(&[48, 0, 2])
            .unwrap_err()
            .contains("below the 3-octet header"));
        assert!(blocks(&[48, 0, 9, 1])
            .unwrap_err()
            .contains("only 4 byte(s) remain"));
        let trailing = [block(48, &[1]), vec![34, 0]].concat();
        assert!(blocks(&trailing).unwrap_err().contains("trailing"));
    }

    #[test]
    fn declared_categories_parse() {
        assert_eq!(
            declared_cats("asterix:034,048").unwrap(),
            BTreeSet::from([34, 48])
        );
        assert_eq!(
            declared_cats("asterix:cat62").unwrap(),
            BTreeSet::from([62])
        );
        assert!(declared_cats("asterix").unwrap().is_empty());
        assert!(declared_cats("asterix:radar").is_err());
        assert!(declared_cats("cot").is_err());
    }

    #[test]
    fn a_stray_category_drops_the_frame() {
        let decl = BTreeSet::from([34, 48]);
        let f = [block(48, &[1]), block(62, &[2])].concat();
        assert!(lint_and_group(&f, &decl)
            .unwrap_err()
            .contains("category 62"));
    }

    #[test]
    fn blocks_group_by_category_in_first_appearance_order() {
        let f = [block(34, &[1]), block(48, &[2]), block(34, &[3])].concat();
        let g = lint_and_group(&f, &BTreeSet::new()).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], (34, [block(34, &[1]), block(34, &[3])].concat()));
        assert_eq!(g[1], (48, block(48, &[2])));
    }
}
