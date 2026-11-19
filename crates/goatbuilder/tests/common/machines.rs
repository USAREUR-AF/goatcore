//! The shape of the `machines` list on `status` and `verify` (gate D, contract C7),
//! shared by tests/status_machines.rs and tests/status_real_hosts.rs.

use std::collections::BTreeMap;

/// The closed set of reasons, by state.
pub const REASONS: &[(&str, &[&str])] = &[
    ("converged", &["declared"]),
    (
        "pending",
        &[
            "unfinished",
            "other-ref",
            "no-record",
            "other-image",
            "served-off",
        ],
    ),
    ("unknown", &["unreachable", "no-answer", "unreadable"]),
];

/// Every entry has its six fields as strings, a state and reason from the
/// closed set, and names a `checks` row whose first word agrees with its state.
/// Returns (host, component) -> (state, reason).
pub fn machines(
    doc: &serde_json::Value,
    text: &str,
) -> BTreeMap<(String, String), (String, String)> {
    let list = doc["machines"]
        .as_array()
        .unwrap_or_else(|| panic!("no machines list:\n{text}"));
    let rows: BTreeMap<String, String> = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_string(),
                c["detail"].as_str().unwrap().to_lowercase(),
            )
        })
        .collect();
    let mut out = BTreeMap::new();
    for m in list {
        for f in [
            "environment",
            "host",
            "component",
            "state",
            "reason",
            "check",
        ] {
            assert!(
                m[f].is_string(),
                "machines entry without a string {f}: {m}\n{text}"
            );
        }
        let (state, reason) = (m["state"].as_str().unwrap(), m["reason"].as_str().unwrap());
        let allowed = REASONS
            .iter()
            .find(|(s, _)| *s == state)
            .unwrap_or_else(|| panic!("state {state} is not converged, pending or unknown: {m}"));
        assert!(
            allowed.1.contains(&reason),
            "reason {reason} is not one of {state}'s: {m}"
        );
        let detail = rows
            .get(m["check"].as_str().unwrap())
            .unwrap_or_else(|| panic!("entry names no checks row: {m}\n{text}"));
        let word = if state == "converged" {
            "converged"
        } else {
            state
        };
        assert!(
            detail.contains(word),
            "the row {} reads {detail:?}, the entry says {state}",
            m["check"]
        );
        out.insert(
            (
                m["host"].as_str().unwrap().to_string(),
                m["component"].as_str().unwrap().to_string(),
            ),
            (state.to_string(), reason.to_string()),
        );
    }
    out
}
