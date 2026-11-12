//! `Api::tools`: list, get, create, update, delete.

mod fake_api;

use fake_api::{FakeApi, Reply};
use serde_json::json;
use tello::{Error, ListTools};

#[tokio::test]
async fn list_without_options_sends_no_query_and_returns_the_page() {
    let mut fake = FakeApi::start().await;
    let page = json!({"data": [{"id": "1", "type": "endCall"}], "nextCursor": null, "extra": 1});
    fake.reply(Reply::Json(200, page.clone()));

    let result = fake
        .api()
        .tools()
        .list(ListTools::new())
        .await
        .expect("list");

    assert_eq!(result, page);
    let request = fake.request().await;
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("GET", "/v1/tools")
    );
}

#[tokio::test]
async fn list_sends_limit_and_cursor() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({"data": [], "nextCursor": null})));

    fake.api()
        .tools()
        .list(ListTools::new().limit(1).cursor("abc=="))
        .await
        .expect("list");

    assert_eq!(
        fake.request().await.target,
        "/v1/tools?limit=1&cursor=abc%3D%3D"
    );
}

#[tokio::test]
async fn create_posts_the_body_and_returns_the_created_tool() {
    let mut fake = FakeApi::start().await;
    let body = json!({"type": "endCall", "name": "hang up", "functionName": "hang_up"});
    let created = json!({"id": "12", "type": "endCall", "revision": 1});
    fake.reply(Reply::Json(201, created.clone()));

    let tool = fake
        .api()
        .tools()
        .create(&body, None)
        .await
        .expect("create");

    assert_eq!(tool, created);
    let request = fake.request().await;
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("POST", "/v1/tools")
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("idempotency-key"), None);
    assert_eq!(request.json(), body);
}

#[tokio::test]
async fn create_sends_the_idempotency_key_it_is_given() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, json!({"id": "12"})));

    fake.api()
        .tools()
        .create(&json!({"type": "endCall"}), Some("tool-create-1"))
        .await
        .expect("create");

    assert_eq!(
        fake.request().await.header("idempotency-key"),
        Some("tool-create-1")
    );
}

#[tokio::test]
async fn a_failed_create_carries_its_idempotency_key() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        502,
        json!({"statusCode": 502, "message": "Write outcome unknown", "error": "write_outcome_unknown", "data": null}),
    ));
    fake.reply(Reply::Drop);
    let api = fake.api();

    match api.tools().create(&json!({}), Some("k-1")).await {
        Err(Error::Api(error)) => {
            assert_eq!(error.code, "write_outcome_unknown");
            assert_eq!(error.idempotency_key.as_deref(), Some("k-1"));
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
    match api.tools().create(&json!({}), Some("k-2")).await {
        Err(Error::ApiConnection(error)) => assert_eq!(error.idempotency_key(), Some("k-2")),
        other => panic!("expected Error::ApiConnection, got {other:?}"),
    }
}

#[tokio::test]
async fn create_answered_with_200_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({"id": "12"})));

    match fake.api().tools().create(&json!({}), Some("k")).await {
        Err(Error::Api(error)) => {
            assert_eq!(error.code, "invalid_response");
            assert_eq!(error.status, 200);
            assert_eq!(error.idempotency_key.as_deref(), Some("k"));
        }
        other => panic!("expected invalid_response, got {other:?}"),
    }
}

#[tokio::test]
async fn invalid_idempotency_keys_are_refused_without_a_request() {
    let mut fake = FakeApi::start().await;
    let api = fake.api();
    let too_long = "k".repeat(256);

    for key in ["", "a,b", "a\nb", "a\tb", "a\u{7f}b", too_long.as_str()] {
        let error = api
            .tools()
            .create(&json!({}), Some(key))
            .await
            .expect_err(key);
        assert!(
            matches!(error, Error::InvalidArgument(_)),
            "{key:?}: {error:?}"
        );
    }
    fake.assert_no_request();

    fake.reply(Reply::Json(201, json!({})));
    let longest = "k".repeat(255);
    api.tools()
        .create(&json!({}), Some(&longest))
        .await
        .expect("255 bytes is allowed");
    assert_eq!(
        fake.request().await.header("idempotency-key"),
        Some(longest.as_str())
    );
}

#[tokio::test]
async fn update_patches_only_the_given_fields() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({"id": "5", "description": "new"})));

    let tool = fake
        .api()
        .tools()
        .update("5", &json!({"description": "new"}))
        .await
        .expect("update");

    assert_eq!(tool["description"], "new");
    let request = fake.request().await;
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("PATCH", "/v1/tools/5")
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("idempotency-key"), None);
    assert_eq!(request.json(), json!({"description": "new"}));
}

#[tokio::test]
async fn update_answered_with_201_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, json!({"id": "5"})));

    let error = fake
        .api()
        .tools()
        .update("5", &json!({}))
        .await
        .expect_err("201");

    assert_eq!(error.code(), Some("invalid_response"));
}

#[tokio::test]
async fn delete_returns_unit_on_204() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Empty(204));

    fake.api().tools().delete("5").await.expect("delete");

    let request = fake.request().await;
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("DELETE", "/v1/tools/5")
    );
    assert_eq!(request.header("content-type"), None);
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn delete_answered_with_200_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, json!({})));

    let error = fake.api().tools().delete("5").await.expect_err("200");

    assert_eq!(error.code(), Some("invalid_response"));
}
