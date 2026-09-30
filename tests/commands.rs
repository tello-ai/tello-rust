mod common;

use common::{within, FakeGateway};
use serde_json::json;
use tello::{Answer, CreateCall, Error, GetSummary, SendDtmf};

fn assert_uuid_v4(value: &serde_json::Value) {
    let text = value.as_str().expect("requestId is a string");
    let id = uuid::Uuid::parse_str(text).expect("requestId is a UUID");
    assert_eq!(id.get_version_num(), 4, "{text}");
}

#[tokio::test]
async fn create_call_sends_every_field_and_keeps_the_caller_request_id() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    let request_id = client
        .create_call(
            CreateCall::new("+821012345678")
                .prompt("reservation check")
                .metadata(json!({ "src": "test" }))
                .request_id("req-1"),
        )
        .await
        .expect("send");

    assert_eq!(request_id, "req-1");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "createCall", "data": {
            "to": "+821012345678",
            "prompt": "reservation check",
            "metadata": { "src": "test" },
            "requestId": "req-1",
        }})
    );
}

#[tokio::test]
async fn create_call_always_sends_a_generated_request_id() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    let generated = client
        .create_call(CreateCall::new("+821012345678"))
        .await
        .expect("send");
    let frame = conn.recv_json().await;
    assert_uuid_v4(&frame["data"]["requestId"]);
    assert_eq!(frame["data"]["requestId"], generated.as_str());
    assert_eq!(
        frame,
        json!({ "event": "createCall", "data": {
            "to": "+821012345678", "prompt": "", "requestId": generated,
        }})
    );

    // An empty caller value counts as unset.
    let replaced = client
        .create_call(CreateCall::new("+821012345678").request_id(""))
        .await
        .expect("send");
    let frame = conn.recv_json().await;
    assert_uuid_v4(&frame["data"]["requestId"]);
    assert_eq!(frame["data"]["requestId"], replaced.as_str());
    assert_ne!(replaced, generated);
}

#[tokio::test]
async fn answer_omits_absent_optional_fields() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    client.answer(Answer::new("yo")).await.expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "answer", "data": { "text": "yo" } })
    );

    client
        .answer(Answer::new("yo").message_id("m1").request_id("r1"))
        .await
        .expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "answer", "data": { "text": "yo", "messageId": "m1", "requestId": "r1" } })
    );
}

#[tokio::test]
async fn send_dtmf_omits_absent_optional_fields() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    client
        .send_dtmf(SendDtmf::new("1234#"))
        .await
        .expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "sendDtmf", "data": { "digits": "1234#" } })
    );

    client
        .send_dtmf(SendDtmf::new("1234#").message_id("m1").request_id("r1"))
        .await
        .expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "sendDtmf", "data": { "digits": "1234#", "messageId": "m1", "requestId": "r1" } })
    );
}

#[tokio::test]
async fn cancel_sends_empty_data() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    client.cancel().await.expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "cancel", "data": {} })
    );
}

#[tokio::test]
async fn get_summary_omits_absent_request_id() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    client
        .get_summary(GetSummary::new("call-1"))
        .await
        .expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "getSummary", "data": { "callId": "call-1" } })
    );

    client
        .get_summary(GetSummary::new("call-1").request_id("s1"))
        .await
        .expect("send");
    assert_eq!(
        conn.recv_json().await,
        json!({ "event": "getSummary", "data": { "callId": "call-1", "requestId": "s1" } })
    );
}

#[tokio::test]
async fn commands_after_the_connection_closed_fail_with_connection_closed() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, conn) = common::connect(&gateway).await;

    drop(conn);
    common::events_until(&mut events, |event| *event == tello::Event::Disconnected).await;

    let error = within("answer", client.answer(Answer::new("late")))
        .await
        .expect_err("closed");
    assert!(
        matches!(error, Error::ConnectionClosed { .. }),
        "got {error:?}"
    );
}
