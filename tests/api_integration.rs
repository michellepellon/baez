// ABOUTME: Integration tests for the Granola public API client
// ABOUTME: Uses wiremock to verify HTTP request/response handling for list and get-note

use baez::api::ApiClient;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_list_notes_paginates_until_no_more() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/notes"))
        .and(header("Authorization", "Bearer test_key"))
        .and(query_param("page_size", "30"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "notes": [{
                "id": "not_aaaaaaaaaaaaaa",
                "title": "First",
                "created_at": "2026-05-10T10:00:00Z",
                "updated_at": "2026-05-10T11:00:00Z"
            }],
            "hasMore": true,
            "cursor": "abc"
        })))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/notes"))
        .and(query_param("cursor", "abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "notes": [{
                "id": "not_bbbbbbbbbbbbbb",
                "title": "Second",
                "created_at": "2026-05-11T10:00:00Z",
                "updated_at": "2026-05-11T11:00:00Z"
            }],
            "hasMore": false,
            "cursor": null
        })))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let result = tokio::task::spawn_blocking(move || {
        let client = ApiClient::new("test_key".into(), Some(uri))
            .unwrap()
            .disable_throttle();
        client.list_notes(None)
    })
    .await
    .unwrap();

    let notes = result.unwrap();
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].id, "not_aaaaaaaaaaaaaa");
    assert_eq!(notes[1].id, "not_bbbbbbbbbbbbbb");
}

#[tokio::test]
async fn test_list_notes_includes_updated_after_filter() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/notes"))
        .and(query_param("updated_after", "2026-05-01T00:00:00Z"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "notes": [],
            "hasMore": false,
            "cursor": null
        })))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let result = tokio::task::spawn_blocking(move || {
        let client = ApiClient::new("test_key".into(), Some(uri))
            .unwrap()
            .disable_throttle();
        client.list_notes(Some("2026-05-01T00:00:00Z"))
    })
    .await
    .unwrap();

    assert_eq!(result.unwrap().len(), 0);
}

#[tokio::test]
async fn test_get_note_includes_transcript_query() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/notes/not_xyz"))
        .and(query_param("include", "transcript"))
        .and(header("Authorization", "Bearer test_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "not_xyz",
            "title": "Test",
            "created_at": "2026-05-11T10:00:00Z",
            "updated_at": "2026-05-11T11:00:00Z",
            "attendees": [{"name": "Alice", "email": "alice@example.com"}],
            "summary_markdown": "## Summary\n- thing",
            "transcript": [
                {"speaker": {"source": "microphone", "diarization_label": "Speaker 1"},
                 "text": "Hello",
                 "start_time": "2026-05-11T10:00:01Z"}
            ]
        })))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let result = tokio::task::spawn_blocking(move || {
        let client = ApiClient::new("test_key".into(), Some(uri))
            .unwrap()
            .disable_throttle();
        client.get_note("not_xyz")
    })
    .await
    .unwrap();

    let note = result.unwrap();
    assert_eq!(note.id, "not_xyz");
    assert_eq!(note.attendees.len(), 1);
    let transcript = note.transcript.unwrap();
    assert_eq!(transcript.len(), 1);
    assert_eq!(transcript[0].text, "Hello");
}

#[tokio::test]
async fn test_get_note_401_propagates_as_api_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/notes/not_xyz"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    let result = tokio::task::spawn_blocking(move || {
        let client = ApiClient::new("bad_key".into(), Some(uri))
            .unwrap()
            .disable_throttle();
        client.get_note("not_xyz")
    })
    .await
    .unwrap();

    match result {
        Err(baez::Error::Api { status, .. }) => assert_eq!(status, 401),
        other => panic!("expected Error::Api 401, got: {:?}", other),
    }
}
