//! The statement a GitHub artifact attestation carries (docs/design/
//! goatbuilder-build-targets.md section 3.9, "Attestations"). signature.rs has
//! verified the envelope it is in: its signature, the log's entry, the
//! certificate and the signer's identity at the pin's tag. Held here: that it
//! is SLSA provenance v1 in an in-toto v1 statement, that one of its subjects
//! is this file under its name and the SHA-256 computed from its bytes, and
//! that the provenance names the pin's repository and tag and the workflow the
//! certificate says was run (its Build Config URI, Fulcio's 1.3.6.1.4.1.57264.1.18:
//! the top-level workflow, which is the SAN's own unless the signer is a
//! reusable workflow it calls). Every other predicate is refused.
//!
//! Read with serde: a field this reads that is repeated, or of the wrong type,
//! is refused; a field it does not read is not judged.

use serde::Deserialize;

use super::{refuse, Component, Refusal};

/// The DSSE payload type of an in-toto statement.
pub const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
/// The one statement type read.
pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
/// The one predicate read.
pub const PREDICATE_TYPE: &str = "https://slsa.dev/provenance/v1";
/// GitHub Actions' build type, the one whose parameters name the workflow.
pub const BUILD_TYPE: &str = "https://actions.github.io/buildtypes/workflow/v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Statement {
    #[serde(rename = "_type")]
    kind: String,
    subject: Vec<Subject>,
    #[serde(rename = "predicateType")]
    predicate_type: String,
    predicate: Predicate,
}

#[derive(Deserialize)]
struct Subject {
    name: String,
    digest: SubjectDigest,
}

#[derive(Deserialize)]
struct SubjectDigest {
    sha256: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Predicate {
    build_definition: BuildDefinition,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildDefinition {
    build_type: String,
    external_parameters: External,
}

#[derive(Deserialize)]
struct External {
    workflow: Workflow,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Workflow {
    #[serde(rename = "ref")]
    reference: String,
    repository: String,
    path: String,
}

/// At most 60 characters of a claim, escaped.
fn shown(s: &str) -> String {
    format!("{:?}", s.chars().take(60).collect::<String>())
}

/// `payload` (of `payload_type`) is SLSA provenance of the file `name`, whose
/// SHA-256 is `sha256`, built at `expect`'s tag of its repository by the
/// workflow `build_config` names (`https://github.com/<repo>/<path>@<ref>`).
pub(super) fn check_statement(payload_type: &str, payload: &[u8], name: &str, sha256: &str, expect: &Component, build_config: &str) -> Result<(), Refusal> {
    let bad = |why: String| refuse("statement", why);
    if payload_type != PAYLOAD_TYPE {
        return Err(bad(format!("the envelope's payload is {}, not {PAYLOAD_TYPE}", shown(payload_type))));
    }
    let text = std::str::from_utf8(payload).map_err(|_| bad("the statement is not UTF-8 text".into()))?;
    let st: Statement = serde_json::from_str(text).map_err(|e| {
        bad(format!("it is not an in-toto statement this reads: a field is missing, repeated or of the wrong type, at line {} column {}", e.line(), e.column()))
    })?;
    if st.kind != STATEMENT_TYPE {
        return Err(bad(format!("the statement is {}, not {STATEMENT_TYPE}", shown(&st.kind))));
    }
    if st.predicate_type != PREDICATE_TYPE {
        return Err(bad(format!("its predicate is {}, not {PREDICATE_TYPE}: no other is read", shown(&st.predicate_type))));
    }
    let named: Vec<&Subject> = st.subject.iter().filter(|s| s.name == name).collect();
    let [subject] = named[..] else {
        return Err(bad(format!("{} of its {} subjects are named {name}, not exactly one", named.len(), st.subject.len())));
    };
    let want = sha256.to_ascii_lowercase();
    if subject.digest.sha256.as_deref().map(str::to_ascii_lowercase).as_deref() != Some(want.as_str()) {
        return Err(bad(format!("its subject {name} is not the file's sha256 {want}")));
    }
    let b = &st.predicate.build_definition;
    if b.build_type != BUILD_TYPE {
        return Err(bad(format!("its build type is {}, not {BUILD_TYPE}", shown(&b.build_type))));
    }
    let w = &b.external_parameters.workflow;
    let repository = format!("https://github.com/{}", expect.repo);
    let tag = format!("refs/tags/{}", expect.tag);
    // The workflow the certificate says ran: `<repository>/<path>@<the tag>`.
    let path = build_config
        .strip_suffix(&format!("@{tag}"))
        .and_then(|u| Some((u.get(..repository.len())?, u.get(repository.len()..)?)))
        .filter(|(r, rest)| r.eq_ignore_ascii_case(&repository) && rest.starts_with("/.github/workflows/"))
        .map(|(_, rest)| &rest[1..])
        .unwrap_or("");
    if !w.repository.eq_ignore_ascii_case(&repository) || w.reference != tag || path.is_empty() || w.path != path {
        return Err(bad(format!(
            "its provenance names the workflow {} of {} at {}, and the certificate {} of {repository} at {tag}",
            shown(&w.path),
            shown(&w.repository),
            shown(&w.reference),
            shown(build_config),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SHA: &str = "b9a154333523a503727eca50511c17ce6aa6bbb655d93b92fc4cd600c9939965";
    const C: Component = Component {
        repo: "example-org/example-tool",
        tag: "tool/v1.2.0",
        signer: "https://github.com/example-org/example-tool/.github/workflows/release.yml",
    };

    fn statement() -> serde_json::Value {
        json!({
            "_type": STATEMENT_TYPE,
            "subject": [{"name": "tool.tar.gz", "digest": {"sha256": SHA}}, {"name": "other.tar.gz", "digest": {"sha256": "0".repeat(64)}}],
            "predicateType": PREDICATE_TYPE,
            "predicate": {"buildDefinition": {"buildType": BUILD_TYPE, "externalParameters": {"workflow": {
                "ref": "refs/tags/tool/v1.2.0", "repository": "https://github.com/example-org/example-tool", "path": ".github/workflows/release.yml"}},
                "internalParameters": {"github": {}}}, "runDetails": {}}
        })
    }

    const RAN: &str = "https://github.com/example-org/example-tool/.github/workflows/release.yml@refs/tags/tool/v1.2.0";

    fn check(v: &serde_json::Value) -> Result<(), Refusal> {
        check_statement(PAYLOAD_TYPE, v.to_string().as_bytes(), "tool.tar.gz", SHA, &C, RAN)
    }

    #[test]
    fn slsa_provenance_of_this_file_by_this_workflow_at_this_tag_is_accepted() {
        check(&statement()).unwrap();
        // GitHub's names are not case-sensitive; a digest in capitals is the same digest.
        let mut v = statement();
        v["predicate"]["buildDefinition"]["externalParameters"]["workflow"]["repository"] = json!("https://github.com/Example-Org/Example-Tool");
        v["subject"][0]["digest"]["sha256"] = json!(SHA.to_ascii_uppercase());
        check(&v).unwrap();
    }

    #[test]
    fn another_predicate_file_digest_or_workflow_is_refused() {
        let wf = "/predicate/buildDefinition/externalParameters/workflow";
        for (what, change) in [
            ("statement type", Box::new(|v: &mut serde_json::Value| v["_type"] = json!("https://in-toto.io/Statement/v0.1")) as Box<dyn Fn(&mut serde_json::Value)>),
            ("predicate", Box::new(|v| v["predicateType"] = json!("https://spdx.dev/Document"))),
            ("predicate v0.2", Box::new(|v| v["predicateType"] = json!("https://slsa.dev/provenance/v0.2"))),
            ("no subject of the name", Box::new(|v| v["subject"][0]["name"] = json!("tool.tar.gz.bak"))),
            ("two of the name", Box::new(|v| v["subject"][1]["name"] = json!("tool.tar.gz"))),
            ("another digest", Box::new(|v| v["subject"][0]["digest"]["sha256"] = json!("1".repeat(64)))),
            ("no sha256", Box::new(|v| v["subject"][0]["digest"] = json!({"sha512": "a"}))),
            ("build type", Box::new(|v| v["predicate"]["buildDefinition"]["buildType"] = json!("https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1"))),
            ("repository", Box::new(move |v| v.pointer_mut(wf).unwrap()["repository"] = json!("https://github.com/example-org/other"))),
            ("tag", Box::new(move |v| v.pointer_mut(wf).unwrap()["ref"] = json!("refs/tags/tool/v1.1.0"))),
            ("branch", Box::new(move |v| v.pointer_mut(wf).unwrap()["ref"] = json!("refs/heads/main"))),
            ("workflow", Box::new(move |v| v.pointer_mut(wf).unwrap()["path"] = json!(".github/workflows/other.yml"))),
            ("an unknown key at the top", Box::new(|v| v["extra"] = json!(1))),
            ("subject not a list", Box::new(|v| v["subject"] = json!({"name": "tool.tar.gz"}))),
            ("no predicate", Box::new(|v| { v.as_object_mut().unwrap().remove("predicate"); })),
        ] {
            let mut v = statement();
            change(&mut v);
            let e = check(&v).unwrap_err();
            assert_eq!(e.check, "statement", "{what}: {e}");
        }
        // A repeated field this reads, the payload type, and text that is not UTF-8.
        let dup = statement().to_string().replacen("\"predicateType\":", "\"predicateType\":\"https://spdx.dev/Document\",\"predicateType\":", 1);
        assert_eq!(check_statement(PAYLOAD_TYPE, dup.as_bytes(), "tool.tar.gz", SHA, &C, RAN).unwrap_err().check, "statement");
        assert!(check_statement("application/json", statement().to_string().as_bytes(), "tool.tar.gz", SHA, &C, RAN).unwrap_err().detail.contains("not application/vnd.in-toto+json"));
        assert_eq!(check_statement(PAYLOAD_TYPE, b"\xff\xfe", "tool.tar.gz", SHA, &C, RAN).unwrap_err().check, "statement");
        // The workflow that ran, as the certificate names it, is the one the
        // provenance must name: another, another repository's, or at another
        // ref is refused; the signer's own reusable workflow is not it.
        for ran in [
            "https://github.com/example-org/example-tool/.github/workflows/other.yml@refs/tags/tool/v1.2.0",
            "https://github.com/example-org/other/.github/workflows/release.yml@refs/tags/tool/v1.2.0",
            "https://github.com/example-org/example-tool/.github/workflows/release.yml@refs/heads/main",
            "https://github.com/example-org/example-tool/release.yml@refs/tags/tool/v1.2.0",
            "",
        ] {
            let e = check_statement(PAYLOAD_TYPE, statement().to_string().as_bytes(), "tool.tar.gz", SHA, &C, ran).unwrap_err();
            assert_eq!(e.check, "statement", "{ran}: {e}");
        }
        let reusable = Component { signer: "https://github.com/example-org/example-tool/.github/workflows/_release-binary.yml", ..C };
        check_statement(PAYLOAD_TYPE, statement().to_string().as_bytes(), "tool.tar.gz", SHA, &reusable, RAN).unwrap();
        // The sender's text is shown escaped.
        let mut v = statement();
        v["predicateType"] = json!("\u{1b}[31mred");
        assert!(!check(&v).unwrap_err().detail.contains('\u{1b}'));
    }
}
