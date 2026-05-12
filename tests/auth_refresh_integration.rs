// ABOUTME: Integration tests for the Granola token auto-refresh flow
// ABOUTME: Uses wiremock to mock both the Granola API and the WorkOS auth endpoint

use baez::api::ApiClient;
use baez::{Credentials, RefreshableState};
use reqwest::blocking::Client;
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Write a fake supabase.json with the given access_token and refresh_token.
fn write_session_file(dir: &std::path::Path, access: &str, refresh: &str) -> PathBuf {
    let path = dir.join("supabase.json");
    let workos_str = serde_json::json!({
        "access_token": access,
        "refresh_token": refresh,
        "expires_in": 21599_u64,
        "obtained_at": 1700000000000_i64,
        "token_type": "Bearer",
    })
    .to_string();
    let content = serde_json::json!({
        "workos_tokens": workos_str,
        "session_id": "session_abc",
        "user_info": "{\"name\":\"Test\"}"
    })
    .to_string();
    fs::write(&path, content).unwrap();
    path
}

fn build_credentials(
    access: &str,
    refresh: &str,
    supabase_path: PathBuf,
    auth_domain: &str,
) -> Credentials {
    let http_client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    Credentials::Refreshable(RefCell::new(RefreshableState {
        access_token: access.to_string(),
        refresh_token: refresh.to_string(),
        supabase_path,
        http_client,
        verbose: false,
        auth_domain: auth_domain.to_string(),
    }))
}

#[tokio::test]
async fn test_happy_path_401_triggers_refresh_then_retry() {
    let mock_server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let session_path = write_session_file(temp.path(), "old_at", "old_rt");

    // First call with old token returns 401; second call with new token returns 200.
    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .and(header("Authorization", "Bearer old_at"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"message\":\"Unauthorized\"}"))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .and(header("Authorization", "Bearer new_at"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "docs": [{
                "id": "doc1",
                "title": "Test",
                "created_at": "2025-10-28T15:04:05Z",
                "updated_at": "2025-10-29T01:23:45Z"
            }]
        })))
        .mount(&mock_server)
        .await;

    // WorkOS refresh returns new tokens.
    Mock::given(method("POST"))
        .and(path("/user_management/authenticate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "new_at",
            "refresh_token": "new_rt",
            "expires_in": 21599
        })))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let auth_uri = mock_server.uri();
    let session_path_clone = session_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        let creds = build_credentials("old_at", "old_rt", session_path_clone, &auth_uri);
        let client = ApiClient::new(creds, Some(uri)).unwrap().disable_throttle();
        client.list_documents()
    })
    .await
    .unwrap();

    let docs = result.expect("list_documents should succeed after refresh+retry");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, "doc1");

    // supabase.json should have been updated with the new tokens.
    let content = fs::read_to_string(&session_path).unwrap();
    let top: serde_json::Value = serde_json::from_str(&content).unwrap();
    let workos: serde_json::Value =
        serde_json::from_str(top["workos_tokens"].as_str().unwrap()).unwrap();
    assert_eq!(workos["access_token"].as_str().unwrap(), "new_at");
    assert_eq!(workos["refresh_token"].as_str().unwrap(), "new_rt");
    // session_id and user_info preserved.
    assert_eq!(top["session_id"].as_str().unwrap(), "session_abc");
    assert!(top["user_info"].as_str().unwrap().contains("Test"));
}

#[tokio::test]
async fn test_cas_adoption_skips_workos_call() {
    // Scenario: another process refreshed supabase.json between our load and 401.
    // refresh_with_cas should detect the new tokens and adopt them without
    // calling WorkOS at all.
    let mock_server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let session_path = write_session_file(temp.path(), "old_at", "old_rt");

    // First call (Bearer old_at) gets 401; second call (Bearer cas_at) succeeds.
    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .and(header("Authorization", "Bearer old_at"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"message\":\"Unauthorized\"}"))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .and(header("Authorization", "Bearer cas_at"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"docs": []})))
        .mount(&mock_server)
        .await;

    // WorkOS endpoint registered with .expect(0) — the test fails if it's called.
    Mock::given(method("POST"))
        .and(path("/user_management/authenticate"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock_server)
        .await;

    // Rewrite supabase.json with new tokens BEFORE the request. This simulates
    // another process having refreshed since we constructed credentials.
    write_session_file(temp.path(), "cas_at", "cas_rt");

    let uri = mock_server.uri();
    let auth_uri = mock_server.uri();
    let session_path_clone = session_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        let creds = build_credentials("old_at", "old_rt", session_path_clone, &auth_uri);
        let client = ApiClient::new(creds, Some(uri)).unwrap().disable_throttle();
        client.list_documents()
    })
    .await
    .unwrap();

    let docs = result.expect("list_documents should succeed via CAS adoption");
    assert_eq!(docs.len(), 0);
    // wiremock will panic on drop if /user_management/authenticate was called.
}

#[tokio::test]
async fn test_refresh_rejected_returns_auth_error() {
    let mock_server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let session_path = write_session_file(temp.path(), "old_at", "old_rt");

    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"message\":\"Unauthorized\"}"))
        .mount(&mock_server)
        .await;

    // WorkOS rejects the refresh_token. 400 is terminal per is_retryable_refresh.
    Mock::given(method("POST"))
        .and(path("/user_management/authenticate"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "{\"error\":\"invalid_grant\",\"error_description\":\"refresh_token expired\"}",
        ))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let auth_uri = mock_server.uri();

    let result = tokio::task::spawn_blocking(move || {
        let creds = build_credentials("old_at", "old_rt", session_path, &auth_uri);
        let client = ApiClient::new(creds, Some(uri)).unwrap().disable_throttle();
        client.list_documents()
    })
    .await
    .unwrap();

    let err = result.expect_err("expected refresh rejection to bubble up");
    match err {
        baez::Error::Auth(msg) => {
            assert!(
                msg.contains("refresh failed"),
                "expected 'refresh failed' in error, got: {}",
                msg
            );
            assert!(
                msg.contains("400"),
                "expected status 400 in error, got: {}",
                msg
            );
        }
        other => panic!("expected Error::Auth, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_workos_5xx_retries_then_auth_error() {
    // Tests that 5xx from WorkOS is retried (per is_retryable_refresh) and
    // surfaces as Error::Auth after retries are exhausted. NOTE: the refresh
    // path uses 2s initial backoff, so this test takes ~6s.
    let mock_server = MockServer::start().await;
    let temp = TempDir::new().unwrap();
    let session_path = write_session_file(temp.path(), "old_at", "old_rt");

    Mock::given(method("POST"))
        .and(path("/v2/get-documents"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"message\":\"Unauthorized\"}"))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/user_management/authenticate"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let auth_uri = mock_server.uri();

    let result = tokio::task::spawn_blocking(move || {
        let creds = build_credentials("old_at", "old_rt", session_path, &auth_uri);
        let client = ApiClient::new(creds, Some(uri)).unwrap().disable_throttle();
        client.list_documents()
    })
    .await
    .unwrap();

    let err = result.expect_err("expected retries to exhaust and surface error");
    match err {
        baez::Error::Auth(msg) => {
            assert!(
                msg.contains("refresh failed"),
                "expected 'refresh failed' in error, got: {}",
                msg
            );
            assert!(
                msg.contains("500"),
                "expected status 500 in error after retries, got: {}",
                msg
            );
        }
        other => panic!("expected Error::Auth, got: {:?}", other),
    }
}
