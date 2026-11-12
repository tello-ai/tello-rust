//! Request headers, URLs, timeouts and error mapping shared by every REST call.

mod fake_api;

use std::time::Duration;

use fake_api::{FakeApi, Reply, API_KEY};
use serde_json::json;
use tello::{Api, ApiConfig, Error};

const USER_AGENT: &str = concat!("tello-rust-sdk/", env!("CARGO_PKG_VERSION"));

fn api_error(error: Error) -> tello::ApiError {
    match error {
        Error::Api(error) => *error,
        other => panic!("expected Error::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn sends_the_bearer_key_accept_and_sdk_user_agent() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({"id": "7", "name": "hang up"})));

    let tool = fake.api().tools().get("7").await.expect("get");

    assert_eq!(tool, json!({"id": "7", "name": "hang up"}));
    let request = fake.request().await;
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/v1/tools/7");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {API_KEY}").as_str())
    );
    assert_eq!(request.header("accept"), Some("application/json"));
    assert_eq!(request.header("user-agent"), Some(USER_AGENT));
    assert!(USER_AGENT.starts_with("tello-rust-sdk/0."), "{USER_AGENT}");
    assert_eq!(request.header("content-type"), None);
    assert_eq!(request.header("idempotency-key"), None);
}

#[tokio::test]
async fn path_ids_are_percent_encoded() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({})));

    fake.api().tools().get("a/b c?%").await.expect("get");

    assert_eq!(fake.request().await.target, "/v1/tools/a%2Fb%20c%3F%25");
}

#[tokio::test]
async fn an_empty_id_is_refused_without_a_request() {
    let mut fake = FakeApi::start().await;

    let error = fake.api().tools().get("").await.expect_err("empty id");

    assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
    fake.assert_no_request();
}

#[tokio::test]
async fn a_trailing_slash_on_the_base_url_is_ignored() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({})));
    let url = format!("{}/", fake.url());

    let api = Api::new(ApiConfig::new(API_KEY).with_api_url(url)).expect("api");
    api.tools().get("7").await.expect("get");

    assert_eq!(fake.request().await.target, "/v1/tools/7");
}

#[tokio::test]
async fn an_error_envelope_becomes_an_api_error_with_the_snake_case_code() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        404,
        json!({
            "statusCode": 404,
            "message": "Tool not found",
            "error": "tool_not_found",
            "data": {"reason": "missing"}
        }),
    ));

    let error = fake.api().tools().get("9").await.expect_err("404");

    assert_eq!(error.code(), Some("tool_not_found"));
    let error = api_error(error);
    assert_eq!(error.status, 404);
    assert_eq!(error.code, "tool_not_found");
    assert_eq!(error.message, "Tool not found");
    assert_eq!(error.data, Some(json!({"reason": "missing"})));
    assert_eq!(error.idempotency_key, None);
    fake.request().await;
}

#[tokio::test]
async fn a_null_envelope_data_is_none() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        401,
        json!({"statusCode": 401, "message": "A valid API key is required", "error": "unauthenticated", "data": null}),
    ));

    let error = api_error(fake.api().tools().get("9").await.expect_err("401"));

    assert_eq!(error.code, "unauthenticated");
    assert_eq!(error.data, None);
}

#[tokio::test]
async fn a_non_json_error_is_coded_by_its_http_status() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Body(
        502,
        "text/html",
        "<html>bad gateway</html>".into(),
    ));

    let error = api_error(fake.api().tools().get("9").await.expect_err("502"));

    assert_eq!(error.status, 502);
    assert_eq!(error.code, "http_502");
    assert_eq!(error.message, "Bad Gateway");
    assert_eq!(error.data, None);
}

#[tokio::test]
async fn a_json_error_without_an_error_code_is_coded_by_its_http_status() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(429, json!({"message": "slow down"})));

    let error = api_error(fake.api().tools().get("9").await.expect_err("429"));

    assert_eq!(error.code, "http_429");
    assert_eq!(error.message, "Too Many Requests");
}

#[tokio::test]
async fn an_undocumented_success_status_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, json!({"id": "7"})));

    let error = api_error(fake.api().tools().get("7").await.expect_err("202"));

    assert_eq!(error.status, 202);
    assert_eq!(error.code, "invalid_response");
    assert_eq!(error.data, None);
}

#[tokio::test]
async fn a_success_body_that_is_not_a_json_object_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!([1, 2])));
    fake.reply(Reply::Body(200, "text/html", "<html></html>".into()));
    let api = fake.api();

    for _ in 0..2 {
        let error = api_error(api.tools().get("7").await.expect_err("not an object"));
        assert_eq!(error.code, "invalid_response");
        assert_eq!(error.status, 200);
    }
}

#[tokio::test]
async fn a_dropped_connection_is_a_connection_error() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Drop);

    let error = fake.api().tools().get("7").await.expect_err("dropped");

    match &error {
        Error::ApiConnection(error) => {
            assert_eq!(error.idempotency_key(), None);
            assert!(!error.is_timeout());
            assert!(std::error::Error::source(error).is_some());
        }
        other => panic!("expected Error::ApiConnection, got {other:?}"),
    }
    assert_eq!(error.code(), None);
    fake.request().await;
}

#[tokio::test]
async fn a_refused_connection_is_a_connection_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/v1", listener.local_addr().expect("addr"));
    drop(listener);

    let api = Api::new(ApiConfig::new(API_KEY).with_api_url(url)).expect("api");
    let error = api.tools().get("7").await.expect_err("refused");

    assert!(matches!(error, Error::ApiConnection(_)), "{error:?}");
}

#[tokio::test]
async fn a_request_past_its_timeout_is_a_timed_out_connection_error() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Delayed(
        Duration::from_secs(2),
        Box::new(Reply::Json(200, json!({}))),
    ));
    let api = fake.api_with(|config| config.with_timeout(Duration::from_millis(100)));

    let error = api.tools().get("7").await.expect_err("timeout");

    match error {
        Error::ApiConnection(error) => assert!(error.is_timeout(), "{error:?}"),
        other => panic!("expected Error::ApiConnection, got {other:?}"),
    }
}

#[test]
fn only_https_or_loopback_http_base_urls_are_accepted() {
    for url in [
        "https://api.telloai.io/v1",
        "https://api.telloai.io/v1/",
        "http://127.0.0.1:8080/v1",
        "http://localhost:8080/v1",
        "http://[::1]:8080/v1",
    ] {
        Api::new(ApiConfig::new(API_KEY).with_api_url(url))
            .unwrap_or_else(|error| panic!("{url}: {error}"));
    }
    for url in [
        "http://api.telloai.io/v1",
        "http://10.0.0.1/v1",
        "ftp://api.telloai.io/v1",
        "wss://api.telloai.io/v1",
        "not a url",
        "",
    ] {
        let error = Api::new(ApiConfig::new(API_KEY).with_api_url(url)).expect_err(url);
        assert!(matches!(error, Error::Config(_)), "{url}: {error:?}");
    }
}

#[test]
fn the_api_key_never_shows_in_debug_output() {
    let config = ApiConfig::new(API_KEY).with_api_url("https://api.telloai.io/v1");
    let api = Api::new(config.clone()).expect("api");

    for debug in [format!("{config:?}"), format!("{api:?}")] {
        assert!(!debug.contains(API_KEY), "{debug}");
        assert!(debug.contains("https://api.telloai.io/v1"), "{debug}");
    }
}

#[test]
fn default_timeouts_and_url() {
    let config = ApiConfig::new(API_KEY);
    assert_eq!(config.timeout(), Duration::from_secs(30));
    assert_eq!(config.long_timeout(), Duration::from_secs(120));
    assert_eq!(tello::DEFAULT_API_URL, "https://api.telloai.io/v1");
}
