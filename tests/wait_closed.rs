//! The `wait_closed` contract: which frames end the current call.
//!
//! Negative checks send a sentinel after the frames under test and read events
//! up to it. The read task updates call state before it emits an event, so once
//! the sentinel is seen every earlier frame has been applied; the short
//! [`QUIET`] window only gives the waiter task a chance to run.

mod common;

use common::{error_frame, events_until, stream_frame, within, FakeGateway, ServerConn, QUIET};
use serde_json::{json, Value};
use tello::{Client, CreateCall, Error, Event, Events, SendDtmf};
use tokio::task::JoinHandle;

fn spawn_wait(client: &Client) -> JoinHandle<Result<(), Error>> {
    let client = client.clone();
    tokio::spawn(async move { client.wait_closed().await })
}

async fn start_call(client: &Client, conn: &mut ServerConn, call: CreateCall) -> String {
    let request_id = client.create_call(call).await.expect("createCall sent");
    let frame = conn.recv_json().await;
    assert_eq!(frame["event"], "createCall");
    assert_eq!(frame["data"]["requestId"], request_id.as_str());
    request_id
}

fn sentinel() -> Value {
    stream_frame("user.turn", json!({ "turnIndex": 999, "text": "sentinel" }))
}

fn is_sentinel(event: &Event) -> bool {
    matches!(event, Event::UserTurn(turn) if turn.turn_index == 999)
}

/// Proves every frame sent before now was processed and the wait is still on.
async fn assert_still_waiting(
    conn: &mut ServerConn,
    events: &mut Events,
    waiter: &mut JoinHandle<Result<(), Error>>,
) -> Vec<Event> {
    conn.send_json(sentinel()).await;
    let seen = events_until(events, is_sentinel).await;
    assert!(
        tokio::time::timeout(QUIET, &mut *waiter).await.is_err(),
        "wait_closed ended early; events before it: {seen:?}"
    );
    seen
}

async fn finish(waiter: JoinHandle<Result<(), Error>>) -> Result<(), Error> {
    within("wait_closed", waiter).await.expect("waiter task")
}

fn error_codes(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Error(error) => Some(error.code.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn each_terminal_event_ends_the_wait() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    let terminals = [
        stream_frame("call.completed", json!({ "status": "completed" })),
        stream_frame("call.noAnswer", json!({ "status": "noAnswer" })),
        stream_frame(
            "call.failed",
            json!({ "status": "failed", "failureReason": "x" }),
        ),
        stream_frame(
            "call.statusChanged",
            json!({ "status": "cancelled", "previousStatus": "inProgress" }),
        ),
    ];
    for terminal in terminals {
        start_call(&client, &mut conn, CreateCall::new("+821012345678")).await;
        let mut waiter = spawn_wait(&client);

        conn.send_json(stream_frame("call.created", json!({})))
            .await;
        conn.send_json(stream_frame(
            "call.statusChanged",
            json!({ "status": "inProgress", "previousStatus": "queued" }),
        ))
        .await;
        assert_still_waiting(&mut conn, &mut events, &mut waiter).await;

        conn.send_json(terminal.clone()).await;
        assert!(finish(waiter).await.is_ok(), "{terminal}");
        events_until(&mut events, |event| event.raw() == &terminal).await;
    }
}

#[tokio::test]
async fn errors_of_other_commands_and_id_less_errors_do_not_end_the_wait() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    let call_id = start_call(
        &client,
        &mut conn,
        CreateCall::new("+821012345678").request_id("call-req"),
    )
    .await;
    let mut waiter = spawn_wait(&client);
    conn.send_json(stream_frame("call.created", json!({})))
        .await;

    client
        .send_dtmf(SendDtmf::new("12a").request_id("dtmf-req"))
        .await
        .expect("sendDtmf sent");
    conn.recv_json().await;
    conn.send_json(error_frame("dtmfDigitsInvalid", Some("dtmf-req")))
        .await;
    conn.send_json(error_frame("internalError", None)).await;
    // Even echoing this call's createCall, noActiveCall never ends it.
    conn.send_json(error_frame("noActiveCall", Some(&call_id)))
        .await;

    let seen = assert_still_waiting(&mut conn, &mut events, &mut waiter).await;
    assert_eq!(
        error_codes(&seen),
        ["dtmfDigitsInvalid", "internalError", "noActiveCall"],
        "errors are still delivered as events"
    );

    conn.send_json(stream_frame(
        "call.completed",
        json!({ "status": "completed" }),
    ))
    .await;
    assert!(finish(waiter).await.is_ok());
}

#[tokio::test]
async fn a_create_call_refusal_ends_the_wait_with_call_refused() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    let request_id = start_call(&client, &mut conn, CreateCall::new("+821012345678")).await;
    let waiter = spawn_wait(&client);
    conn.send_json(error_frame("insufficientCredit", Some(&request_id)))
        .await;

    let error = finish(waiter).await.expect_err("refused");
    assert!(
        matches!(&error, Error::CallRefused { code, .. } if code == "insufficientCredit"),
        "got {error:?}"
    );
    let delivered = events_until(&mut events, |event| matches!(event, Event::Error(_))).await;
    assert!(delivered.is_empty(), "the refusal is also an event");
}

#[tokio::test]
async fn a_stream_failure_after_call_created_ends_the_wait_with_server_error() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    let request_id = start_call(&client, &mut conn, CreateCall::new("+821012345678")).await;
    let waiter = spawn_wait(&client);
    conn.send_json(stream_frame("call.created", json!({})))
        .await;
    conn.send_json(error_frame("internalError", Some(&request_id)))
        .await;

    let error = finish(waiter).await.expect_err("stream failed");
    assert!(
        matches!(&error, Error::Server { code, .. } if code == "internalError"),
        "got {error:?}"
    );
}

#[tokio::test]
async fn a_second_create_call_is_refused_without_ending_the_live_call() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    let first = start_call(
        &client,
        &mut conn,
        CreateCall::new("+821012345678").request_id("first"),
    )
    .await;
    let mut waiter = spawn_wait(&client);
    conn.send_json(stream_frame("call.created", json!({})))
        .await;

    let second = start_call(
        &client,
        &mut conn,
        CreateCall::new("+821099999999").request_id("second"),
    )
    .await;
    conn.send_json(error_frame("callAlreadyActive", Some(&second)))
        .await;
    let seen = assert_still_waiting(&mut conn, &mut events, &mut waiter).await;
    assert_eq!(error_codes(&seen), ["callAlreadyActive"]);

    // The first call's stream fails later: its createCall id still ends it.
    conn.send_json(error_frame("internalError", Some(&first)))
        .await;
    let error = finish(waiter).await.expect_err("first call failed");
    assert!(
        matches!(&error, Error::Server { code, .. } if code == "internalError"),
        "got {error:?}"
    );
}

#[tokio::test]
async fn an_error_echoing_a_previous_calls_create_call_does_not_end_the_next_call() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    start_call(
        &client,
        &mut conn,
        CreateCall::new("+821012345678").request_id("old"),
    )
    .await;
    conn.send_json(stream_frame(
        "call.completed",
        json!({ "status": "completed" }),
    ))
    .await;
    assert!(within("first call", client.wait_closed()).await.is_ok());

    start_call(
        &client,
        &mut conn,
        CreateCall::new("+821012345678").request_id("new"),
    )
    .await;
    let mut waiter = spawn_wait(&client);
    conn.send_json(error_frame("internalError", Some("old")))
        .await;
    assert_still_waiting(&mut conn, &mut events, &mut waiter).await;

    conn.send_json(stream_frame(
        "call.completed",
        json!({ "status": "completed" }),
    ))
    .await;
    assert!(finish(waiter).await.is_ok());
}

#[tokio::test]
async fn a_socket_drop_mid_call_ends_the_wait_with_connection_closed() {
    let gateway = FakeGateway::start().await;
    let (client, mut events, mut conn) = common::connect(&gateway).await;

    start_call(&client, &mut conn, CreateCall::new("+821012345678")).await;
    let waiter = spawn_wait(&client);
    conn.send_json(stream_frame("call.created", json!({})))
        .await;
    drop(conn);

    let error = finish(waiter).await.expect_err("dropped");
    assert!(
        matches!(error, Error::ConnectionClosed { .. }),
        "got {error:?}"
    );
    events_until(&mut events, |event| *event == Event::Disconnected).await;
}

#[tokio::test]
async fn close_4429_mid_call_ends_the_wait_with_session_replaced() {
    let gateway = FakeGateway::start().await;
    let (client, _events, mut conn) = common::connect(&gateway).await;

    start_call(&client, &mut conn, CreateCall::new("+821012345678")).await;
    let waiter = spawn_wait(&client);
    conn.close_with(4429, "session replaced").await;

    let error = finish(waiter).await.expect_err("replaced");
    assert!(
        matches!(error, Error::SessionReplaced { .. }),
        "got {error:?}"
    );
}
