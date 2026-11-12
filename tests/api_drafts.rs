//! `Api::call_drafts`: create, revise, confirm.

mod fake_api;

use std::time::Duration;

use fake_api::{FakeApi, Reply};
use serde_json::{json, Value};
use tello::Error;

fn draft(id: &str, status: &str) -> Value {
    json!({"id": id, "status": status, "agentId": "3", "to": "+821012345678"})
}

fn call() -> Value {
    json!({"id": "c_901", "status": "dialing"})
}

fn is_uuid_v4(key: &str) -> bool {
    let groups: Vec<&str> = key.split('-').collect();
    let lengths: Vec<usize> = groups.iter().map(|group| group.len()).collect();
    lengths == [8, 4, 4, 4, 12]
        && key.chars().all(|c| c == '-' || c.is_ascii_hexdigit())
        && groups[2].starts_with('4')
        && matches!(groups[3].chars().next(), Some('8' | '9' | 'a' | 'b'))
}

#[tokio::test]
async fn create_posts_the_body_and_returns_the_draft() {
    let mut fake = FakeApi::start().await;
    let body = json!({"agentId": "3", "to": "+821012345678", "instructions": "예약 확인"});
    fake.reply(Reply::Json(201, draft("cd_481", "pendingConfirmation")));

    let created = fake
        .api()
        .call_drafts()
        .create(&body, None)
        .await
        .expect("create");

    assert_eq!(created, draft("cd_481", "pendingConfirmation"));
    let request = fake.request().await;
    assert_eq!(
        (request.method.as_str(), request.target.as_str()),
        ("POST", "/v1/call-drafts")
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("idempotency-key"), None);
    assert_eq!(request.json(), body);
}

#[tokio::test]
async fn every_draft_status_is_a_success() {
    for status in [
        "pendingConfirmation",
        "needsClarification",
        "refused",
        "confirmed",
    ] {
        let fake = FakeApi::start().await;
        fake.reply(Reply::Json(201, draft("cd_1", status)));

        let created = fake
            .api()
            .call_drafts()
            .create(&json!({}), Some("draft-1"))
            .await
            .expect(status);

        assert_eq!(created["status"], status);
    }
}

#[tokio::test]
async fn create_sends_the_idempotency_key_and_refuses_200() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, draft("cd_1", "pendingConfirmation")));

    let error = fake
        .api()
        .call_drafts()
        .create(&json!({}), Some("draft-2"))
        .await
        .expect_err("200");

    assert_eq!(error.code(), Some("invalid_response"));
    assert_eq!(
        fake.request().await.header("idempotency-key"),
        Some("draft-2")
    );
}

#[tokio::test]
async fn create_revise_and_confirm_use_the_long_timeout() {
    let fake = FakeApi::start().await;
    let slow = |status: u16, body: Value| {
        Reply::Delayed(
            Duration::from_millis(300),
            Box::new(Reply::Json(status, body)),
        )
    };
    fake.reply(slow(201, draft("cd_1", "pendingConfirmation")));
    fake.reply(slow(200, draft("cd_2", "pendingConfirmation")));
    fake.reply(slow(201, call()));
    let api = fake.api_with(|config| {
        config
            .with_timeout(Duration::from_millis(100))
            .with_long_timeout(Duration::from_secs(5))
    });
    let drafts = api.call_drafts();

    drafts.create(&json!({}), None).await.expect("create");
    drafts.revise("cd_1", "6시로", None).await.expect("revise");
    drafts.confirm("cd_2", None).await.expect("confirm");
}

#[tokio::test]
async fn revise_posts_the_request_and_returns_the_new_draft() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, draft("cd_482", "pendingConfirmation")));

    let revised = fake
        .api()
        .call_drafts()
        .revise(
            "cd_481",
            "4시도 안 되면 6시로 물어봐 주세요",
            Some("revise-1"),
        )
        .await
        .expect("revise");

    assert_eq!(revised["id"], "cd_482");
    let request = fake.request().await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/v1/call-drafts/cd_481/revise");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("idempotency-key"), Some("revise-1"));
    assert_eq!(
        request.json(),
        json!({"request": "4시도 안 되면 6시로 물어봐 주세요"})
    );
}

#[tokio::test]
async fn revise_without_a_key_sends_none_and_refuses_201() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, draft("cd_482", "pendingConfirmation")));

    let error = fake
        .api()
        .call_drafts()
        .revise("cd_481", "again", None)
        .await
        .expect_err("201");

    assert_eq!(error.code(), Some("invalid_response"));
    assert_eq!(fake.request().await.header("idempotency-key"), None);
}

#[tokio::test]
async fn confirm_posts_no_body_with_a_generated_uuid_key() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, call()));
    fake.reply(Reply::Json(201, call()));
    let api = fake.api();

    let placed = api
        .call_drafts()
        .confirm("cd_481", None)
        .await
        .expect("confirm");

    assert_eq!(placed, call());
    let request = fake.request().await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/v1/call-drafts/cd_481/confirm");
    assert_eq!(request.header("content-type"), None);
    assert!(request.body.is_empty());
    let first = request.header("idempotency-key").expect("a key").to_owned();
    assert!(is_uuid_v4(&first), "{first}");

    api.call_drafts()
        .confirm("cd_481", None)
        .await
        .expect("confirm");
    let second = fake.request().await;
    assert_ne!(second.header("idempotency-key"), Some(first.as_str()));
}

#[tokio::test]
async fn confirm_sends_the_key_it_is_given() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, call()));

    fake.api()
        .call_drafts()
        .confirm("cd_481", Some("confirm-cd_481"))
        .await
        .expect("confirm");

    assert_eq!(
        fake.request().await.header("idempotency-key"),
        Some("confirm-cd_481")
    );
}

#[tokio::test]
async fn a_failed_confirm_carries_the_generated_key() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        404,
        json!({"statusCode": 404, "message": "Draft not found", "error": "draft_not_found", "data": null}),
    ));
    fake.reply(Reply::Drop);
    let api = fake.api();

    let Err(Error::Api(error)) = api.call_drafts().confirm("cd_999999", None).await else {
        panic!("expected Error::Api");
    };
    assert_eq!(error.code, "draft_not_found");
    let sent = fake.request().await;
    assert_eq!(
        error.idempotency_key.as_deref(),
        sent.header("idempotency-key")
    );
    assert!(error.idempotency_key.as_deref().is_some_and(is_uuid_v4));

    let Err(Error::ApiConnection(error)) = api.call_drafts().confirm("cd_1", None).await else {
        panic!("expected Error::ApiConnection");
    };
    let sent = fake.request().await;
    assert_eq!(error.idempotency_key(), sent.header("idempotency-key"));
    assert!(error.idempotency_key().is_some_and(is_uuid_v4));
}

#[tokio::test]
async fn confirm_answered_with_200_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, call()));

    let Err(Error::Api(error)) = fake.api().call_drafts().confirm("cd_1", Some("k")).await else {
        panic!("expected Error::Api");
    };
    assert_eq!(error.code, "invalid_response");
    assert_eq!(error.idempotency_key.as_deref(), Some("k"));
}

#[tokio::test]
async fn invalid_keys_and_empty_ids_are_refused_without_a_request() {
    let mut fake = FakeApi::start().await;
    let api = fake.api();
    let drafts = api.call_drafts();

    for error in [
        drafts
            .create(&json!({}), Some("a,b"))
            .await
            .expect_err("create key"),
        drafts
            .revise("cd_1", "x", Some(""))
            .await
            .expect_err("revise key"),
        drafts
            .confirm("cd_1", Some("키"))
            .await
            .expect_err("confirm key"),
        drafts.revise("", "x", None).await.expect_err("revise id"),
        drafts.confirm("", None).await.expect_err("confirm id"),
    ] {
        assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
    }
    fake.assert_no_request();
}
