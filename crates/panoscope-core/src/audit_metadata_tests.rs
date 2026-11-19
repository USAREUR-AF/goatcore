use crate::{
    audit_tail::{AuditTail, AuditTailConfig},
    portal::PresignedObject,
    PortalClient,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[allow(clippy::disallowed_methods)]
async fn server(handler: impl Fn(&str) -> (u16, String, String) + Send + Sync + 'static) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let h = handler.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let n = socket.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buffer[..n]);
                    if raw.windows(4).any(|v| v == b"\r\n\r\n") {
                        break;
                    }
                }
                let (status, headers, body) = h(&String::from_utf8_lossy(&raw));
                let response=format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    format!("http://{addr}")
}
fn object(url: String) -> PresignedObject {
    PresignedObject {
        key: "first".into(),
        url,
        source: "legacy-projection".into(),
        schema_version: 1,
        audit_url: String::new(),
    }
}
#[tokio::test]
async fn history_pages_are_authenticated_and_metadata_only() {
    let reads = Arc::new(AtomicUsize::new(0));
    let count = reads.clone();
    let base=server(move |request| {
        assert!(request.to_ascii_lowercase().contains("authorization: bearer fixture")); count.fetch_add(1,Ordering::SeqCst);
        if request.starts_with("GET /api/audit/presign") {
            assert!(request.contains("projection=1")); let more=!request.contains("after=first");let key=if more{"first"}else{"second"};
            let body=serde_json::json!({"bucket":"audit","since":"2026-10-26T00:00:00Z","key":"","ttl_seconds":0,"count":1,"truncated":more,"objects":[{"key":key,"url":format!("/api/audit/object?key={key}"),"source":"legacy-projection","schema_version":1}]}).to_string();
            (200,String::new(),body)
        } else { (200,String::new(),r#"{"vendor":"slot","key_expr":"slot/data","ts":"2026-10-26T00:00:00Z","payload_hash":"legacy","payload_len":7,"sample_kind":"unknown"}"#.into()) }
    }).await;
    let tail = AuditTail::new(AuditTailConfig {
        enabled: true,
        portal_url: base,
        portal_api_key: "fixture".into(),
        ..Default::default()
    })
    .unwrap();
    let (events, stats) = tail.recent_events(1, None).await.unwrap();
    assert_eq!(stats.objects_fetched, 2);
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.payload_bytes_b64.is_none()));
    assert_eq!(reads.load(Ordering::SeqCst), 4);
}
#[tokio::test]
async fn projection_rejects_body_fields_and_redirects_without_forwarding_auth() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let other = server(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        (200, String::new(), String::new())
    })
    .await;
    let target = other.clone();
    let base = server(move |request| {
        if request.contains("key=redirect") {
            (302, format!("Location: {target}/stolen\r\n"), String::new())
        } else {
            (200, String::new(), r#"{"payload_bytes_b64":null}"#.into())
        }
    })
    .await;
    let portal = PortalClient::new(base, "fixture".into());
    for url in [
        "/api/audit/object?key=redirect".into(),
        "/api/audit/object?key=body".into(),
        format!("{other}/api/audit/object?key=foreign"),
    ] {
        let result = portal.audit_metadata(&[object(url)]).await.unwrap();
        assert!(result.events.is_empty());
        assert_eq!(result.skipped.len(), 1);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn history_listing_does_not_follow_redirects() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let target = server(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
        (200, String::new(), "{}".into())
    })
    .await;
    let base =
        server(move |_| (302, format!("Location: {target}/stolen\r\n"), String::new())).await;
    let portal = PortalClient::new(base, "fixture".into());
    assert!(portal
        .audit_history_page("2026-10-26T00:00:00Z", None)
        .await
        .is_err());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn history_refusals_preserve_the_server_reason_and_status() {
    let base =
        server(|request| {
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture"));
            if request.starts_with("GET /api/replay/presign") {
                (503, String::new(), serde_json::json!({"error": {
                "code": "replay_gated", "message": "replay is unavailable pending owner governance"
            }}).to_string())
            } else {
                (426, String::new(), serde_json::json!({"error": {
                "code": "upgrade_required", "message": "metadata projection support is required"
            }}).to_string())
            }
        })
        .await;
    let portal = PortalClient::new(base, "fixture".into());
    let replay = portal.replay_presign(Some("10m"), None).await.unwrap_err();
    assert_eq!(
        replay.http_status(),
        Some(reqwest::StatusCode::SERVICE_UNAVAILABLE)
    );
    assert_eq!(replay.error_code(), Some("replay_gated"));
    assert_eq!(replay.path(), "/api/replay/presign");
    assert_eq!(
        replay.server_message(),
        Some("replay is unavailable pending owner governance")
    );
    let upgrade = portal.audit_presign(Some("10m"), None).await.unwrap_err();
    assert_eq!(
        upgrade.http_status(),
        Some(reqwest::StatusCode::UPGRADE_REQUIRED)
    );
    assert_eq!(upgrade.error_code(), Some("upgrade_required"));
}

#[tokio::test]
async fn projection_failures_identify_which_connection_failed() {
    let base = server(|request| {
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture"));
        let status = if request.contains("key=denied") {
            403
        } else {
            502
        };
        (
            status,
            String::new(),
            serde_json::json!({"error": {
                "message": "synthetic projection refusal"
            }})
            .to_string(),
        )
    })
    .await;
    let portal = PortalClient::new(base, "fixture".into());
    for (key, expected) in [
        ("denied", "portal HTTP 403"),
        ("storage", "portal HTTP 502"),
    ] {
        let result = portal
            .audit_metadata(&[object(format!("/api/audit/object?key={key}"))])
            .await
            .unwrap();
        assert!(result.events.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(
            result.skipped[0].reason.starts_with(expected),
            "{:?}",
            result.skipped
        );
        assert!(result.skipped[0]
            .reason
            .contains("synthetic projection refusal"));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let offline = PortalClient::new(format!("http://{address}"), "fixture".into());
    let result = offline
        .audit_metadata(&[object("/api/audit/object?key=offline".into())])
        .await
        .unwrap();
    assert!(result.events.is_empty());
    assert!(result.skipped[0]
        .reason
        .starts_with("portal request failed (no HTTP status)"));
}
