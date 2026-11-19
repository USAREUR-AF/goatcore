//! Replay is checked against the metadata object, never treated as audit evidence.
use crate::{
    audit_event::AuditEvent,
    audit_tail::{fetch_presigned_object, redact_presigned_url, PresignedFetch, SkippedObject},
    portal::PresignedObject,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub async fn fetch_replay_events(objects: &[PresignedObject]) -> Result<PresignedFetch> {
    anyhow::ensure!(cfg!(test), "replay reads are gated pending owner governance");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let results = futures::stream::iter(objects.iter().map(|object| {
        let http = &http;
        async move { (object, fetch_one(http, object).await) }
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    let mut out = PresignedFetch::default();
    for (object, result) in results {
        match result {
            Ok(events) => out.events.extend(events),
            Err(reason) => out.skipped.push(SkippedObject {
                object: redact_presigned_url(&object.url),
                reason,
            }),
        }
    }
    Ok(out)
}

async fn fetch_one(
    http: &reqwest::Client,
    object: &PresignedObject,
) -> std::result::Result<Vec<AuditEvent>, String> {
    let body = fetch_presigned_object(http, &object.url).await?;
    let audit = if object.source == "replay" {
        if object.audit_url.is_empty() {
            return Err("missing audit verification URL".into());
        }
        Some(fetch_presigned_object(http, &object.audit_url).await?)
    } else {
        None
    };
    verify(object, &body, audit.as_deref()).map_err(|e| e.to_string())
}

fn rows(body: &[u8]) -> Result<Vec<AuditEvent>> {
    body.split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).context("invalid history record"))
        .collect()
}

fn verify(
    object: &PresignedObject,
    body: &[u8],
    audit_body: Option<&[u8]>,
) -> Result<Vec<AuditEvent>> {
    let events = rows(body)?;
    anyhow::ensure!(!events.is_empty(), "empty history object");
    match (object.schema_version, object.source.as_str()) {
        (1, "legacy") => {
            anyhow::ensure!(
                events.iter().all(|e| e.schema_version <= 1),
                "legacy object contains non-legacy records"
            );
        }
        (2, "metadata") => {
            anyhow::ensure!(
                object.key.ends_with("-v2-meta.ndjson"),
                "metadata object filename mismatch"
            );
            for (i, event) in events.iter().enumerate() {
                validate_metadata(event, &object.key, i)?;
                anyhow::ensure!(
                    event.payload_capture == "disabled",
                    "metadata-only object claims replay capture"
                );
            }
        }
        (2, "replay") => {
            anyhow::ensure!(
                object.key.ends_with("-v2-replay.ndjson"),
                "replay object filename mismatch"
            );
            let audit = rows(audit_body.context("missing audit verification object")?)?;
            anyhow::ensure!(
                events.len() == audit.len(),
                "replay/audit record count mismatch"
            );
            for (i, (event, metadata)) in events.iter().zip(&audit).enumerate() {
                validate_metadata(metadata, &object.key, i)?;
                let mut projection = event.clone();
                projection.payload_bytes_b64 = None;
                anyhow::ensure!(
                    &projection == metadata,
                    "replay metadata differs from audit record"
                );
                match metadata.payload_capture.as_str() {
                    "captured" => {
                        let bytes = STANDARD
                            .decode(
                                event
                                    .payload_bytes_b64
                                    .as_ref()
                                    .context("captured payload missing")?,
                            )
                            .context("invalid payload encoding")?;
                        anyhow::ensure!(
                            bytes.len() == metadata.payload_len,
                            "replay payload length mismatch"
                        );
                        anyhow::ensure!(
                            format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
                                == metadata.payload_hash,
                            "replay payload hash mismatch"
                        );
                    }
                    "oversize" => anyhow::ensure!(
                        event.payload_bytes_b64.is_none(),
                        "oversize replay unexpectedly contains payload"
                    ),
                    _ => anyhow::bail!("invalid replay capture state"),
                }
            }
        }
        _ => anyhow::bail!("unsupported history source or schema"),
    }
    Ok(events)
}

fn validate_metadata(event: &AuditEvent, key: &str, ordinal: usize) -> Result<()> {
    anyhow::ensure!(
        event.schema_version == 2 && event.event_id == format!("{key}#{ordinal}"),
        "audit schema or event identity mismatch"
    );
    anyhow::ensure!(
        event.payload_bytes_b64.is_none(),
        "V2 audit object contains a body"
    );
    anyhow::ensure!(
        matches!(
            event.onboarding_classification.as_str(),
            "onboarding_hello" | "other"
        ),
        "invalid onboarding classification"
    );
    let expected = if event.src_zid.is_empty() {
        "unavailable"
    } else {
        "publisher_asserted"
    };
    anyhow::ensure!(
        event.source_provenance == expected,
        "invalid publisher provenance"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PresignedObject, AuditEvent) {
        let key = "site/2026/10/26/12/1793000000000-1-v2-replay.ndjson";
        (
            PresignedObject {
                key: key.into(),
                url: "http://localhost/replay".into(),
                audit_url: "http://localhost/audit".into(),
                source: "replay".into(),
                schema_version: 2,
            },
            AuditEvent {
                schema_version: 2,
                event_id: format!("{key}#0"),
                payload_capture: "captured".into(),
                source_provenance: "unavailable".into(),
                onboarding_classification: "other".into(),
                payload_len: 6,
                payload_hash: format!("sha256:{}", hex::encode(Sha256::digest(b"secret"))),
                ..Default::default()
            },
        )
    }
    #[test]
    fn replay_requires_matching_audit_and_detects_tampering_and_missing_body() {
        let (object, metadata) = fixture();
        let audit = serde_json::to_vec(&metadata).unwrap();
        let mut replay = metadata.clone();
        replay.payload_bytes_b64 = Some(STANDARD.encode(b"secret"));
        assert!(verify(&object, &serde_json::to_vec(&replay).unwrap(), Some(&audit)).is_ok());
        replay.payload_bytes_b64 = Some(STANDARD.encode(b"forged"));
        assert!(verify(&object, &serde_json::to_vec(&replay).unwrap(), Some(&audit)).is_err());
        assert!(verify(&object, &audit, Some(&audit)).is_err());
        assert!(verify(&object, &audit, None).is_err());
        replay.payload_bytes_b64 = Some(STANDARD.encode(b"secret"));
        replay.src_zid = "forged".into();
        assert!(verify(&object, &serde_json::to_vec(&replay).unwrap(), Some(&audit)).is_err());
    }

    #[tokio::test]
    async fn mixed_window_reads_each_source_and_reports_missing_replay() {
        use std::io::{Read, Write};
        let (mut replay_object, metadata) = fixture();
        let mut replay = metadata.clone();
        replay.payload_bytes_b64 = Some(STANDARD.encode(b"secret"));
        let mut disabled = metadata.clone();
        let meta_key = replay_object.key.replace("-replay.", "-meta.");
        disabled.event_id = format!("{meta_key}#0");
        disabled.payload_capture = "disabled".into();
        let mut legacy = metadata.clone();
        legacy.schema_version = 0;
        let responses = std::collections::HashMap::from([
            ("/legacy", serde_json::to_vec(&legacy).unwrap()),
            ("/metadata", serde_json::to_vec(&disabled).unwrap()),
            ("/replay", serde_json::to_vec(&replay).unwrap()),
            ("/audit", serde_json::to_vec(&metadata).unwrap()),
        ]);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..5 {
                let (mut sock, _) = listener.accept().unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut raw = Vec::new();
                let mut buf = [0; 4096];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).unwrap();
                    assert!(n > 0);
                    raw.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8(raw).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let (status, body) = match responses.get(path) {
                    Some(body) => ("200 OK", body.as_slice()),
                    None => ("404 Not Found", &b""[..]),
                };
                write!(
                    sock,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                sock.write_all(body).unwrap();
            }
        });
        replay_object.url = format!("{base}/replay");
        replay_object.audit_url = format!("{base}/audit");
        let mut missing = replay_object.clone();
        missing.url = format!("{base}/missing?X-Amz-Signature=secret");
        let result = fetch_replay_events(&[
            PresignedObject {
                key: "legacy.ndjson".into(),
                url: format!("{base}/legacy"),
                source: "legacy".into(),
                schema_version: 1,
                audit_url: String::new(),
            },
            PresignedObject {
                key: meta_key,
                url: format!("{base}/metadata"),
                source: "metadata".into(),
                schema_version: 2,
                audit_url: String::new(),
            },
            replay_object,
            missing,
        ])
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("404"));
        assert!(!result.skipped[0].object.contains("secret"));
        assert!(result.events[1].payload_bytes_b64.is_none());
        assert_eq!(
            result.events[2].payload_bytes_b64,
            Some(STANDARD.encode(b"secret"))
        );
    }

    #[tokio::test]
    async fn transport_failure_does_not_disclose_presigned_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let (mut object, _) = fixture();
        object.url = format!("http://{address}/missing?X-Amz-Signature=private-signature");
        let result = fetch_replay_events(&[object]).await.unwrap();
        assert!(result.events.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("request failed"));
        assert!(!result.skipped[0].reason.contains("private-signature"));
        assert!(!result.skipped[0].object.contains("private-signature"));
    }

    #[test]
    fn oversize_replay_and_disabled_metadata_have_no_payload() {
        let (object, mut event) = fixture();
        event.payload_capture = "oversize".into();
        let audit = serde_json::to_vec(&event).unwrap();
        assert!(verify(&object, &audit, Some(&audit)).is_ok());
        event.payload_bytes_b64 = Some(STANDARD.encode(b"secret"));
        assert!(verify(&object, &serde_json::to_vec(&event).unwrap(), Some(&audit)).is_err());
        let mut object = object;
        object.key = object.key.replace("-replay.", "-meta.");
        object.source = "metadata".into();
        event.event_id = format!("{}#0", object.key);
        event.payload_bytes_b64 = None;
        event.payload_capture = "disabled".into();
        assert!(verify(&object, &serde_json::to_vec(&event).unwrap(), None).is_ok());
    }
}
