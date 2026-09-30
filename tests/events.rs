mod common;

use common::{error_frame, next_event, stream_frame, within, FakeGateway};
use serde_json::{json, Value};
use tello::{CallStatus, Event};

#[tokio::test]
async fn every_frame_type_parses_into_its_typed_variant() {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;

    let frames = vec![
        stream_frame("call.created", json!({})),
        stream_frame(
            "call.statusChanged",
            json!({ "status": "inProgress", "previousStatus": "queued" }),
        ),
        stream_frame(
            "user.turn",
            json!({ "turnIndex": 1, "text": "예약 확인하려고요" }),
        ),
        stream_frame(
            "agent.turn",
            json!({ "turnIndex": 2, "text": "네, 성함 부탁드립니다." }),
        ),
        stream_frame(
            "answer.accepted",
            json!({ "requestId": "r-answer", "messageId": "m1" }),
        ),
        stream_frame(
            "dtmf.accepted",
            json!({ "requestId": "r-dtmf", "messageId": "m1b", "digits": "1234#" }),
        ),
        stream_frame("call.completed", json!({ "status": "completed" })),
        stream_frame(
            "call.noAnswer",
            json!({ "status": "noAnswer", "failureReason": "timeout" }),
        ),
        stream_frame(
            "call.failed",
            json!({ "status": "failed", "failureReason": "carrier" }),
        ),
        json!({
            "type": "call.summary", "version": "1.0", "requestId": "r-sum",
            "callId": "call-1", "status": "completed", "durationSeconds": 42,
            "transcript": "hello", "summary": "short", "creditCharged": 1.5,
        }),
        json!({
            "type": "error", "version": "1.0", "code": "callRejected",
            "message": "Call rejected", "requestId": "r-err", "question": "Who is calling?",
        }),
        json!({ "type": "future.thing", "version": "1.0", "x": 1 }),
    ];
    for frame in &frames {
        conn.send_json(frame.clone()).await;
    }

    let mut received = Vec::new();
    for _ in &frames {
        received.push(next_event(&mut events).await);
    }
    for (event, frame) in received.iter().zip(&frames) {
        assert_eq!(event.raw(), frame, "raw frame is kept");
    }

    let mut received = received.into_iter();
    let mut next = || received.next().expect("event");

    let Event::CallCreated(created) = next() else {
        panic!("call.created")
    };
    assert_eq!(
        (
            created.session_id.as_str(),
            created.call_id.as_str(),
            created.timestamp.as_str()
        ),
        ("sess-1", "call-1", "2026-07-08T04:00:00.000Z")
    );

    let Event::CallStatusChanged(changed) = next() else {
        panic!("call.statusChanged")
    };
    assert_eq!(changed.status, CallStatus::InProgress);
    assert_eq!(changed.previous_status, CallStatus::Queued);
    assert_eq!(changed.call_id, "call-1");

    let Event::UserTurn(user) = next() else {
        panic!("user.turn")
    };
    assert_eq!(
        (user.turn_index, user.text.as_str()),
        (1, "예약 확인하려고요")
    );

    let Event::AgentTurn(agent) = next() else {
        panic!("agent.turn")
    };
    assert_eq!(
        (agent.turn_index, agent.text.as_str()),
        (2, "네, 성함 부탁드립니다.")
    );

    let Event::AnswerAccepted(answer) = next() else {
        panic!("answer.accepted")
    };
    assert_eq!(answer.request_id.as_deref(), Some("r-answer"));
    assert_eq!(answer.message_id, "m1");

    let Event::DtmfAccepted(dtmf) = next() else {
        panic!("dtmf.accepted")
    };
    assert_eq!(dtmf.request_id.as_deref(), Some("r-dtmf"));
    assert_eq!(
        (dtmf.message_id.as_str(), dtmf.digits.as_str()),
        ("m1b", "1234#")
    );

    let Event::CallCompleted(completed) = next() else {
        panic!("call.completed")
    };
    assert_eq!(completed.status, CallStatus::Completed);
    assert_eq!(completed.failure_reason, None);

    let Event::CallNoAnswer(no_answer) = next() else {
        panic!("call.noAnswer")
    };
    assert_eq!(no_answer.status, CallStatus::NoAnswer);
    assert_eq!(no_answer.failure_reason.as_deref(), Some("timeout"));

    let Event::CallFailed(failed) = next() else {
        panic!("call.failed")
    };
    assert_eq!(failed.status, CallStatus::Failed);
    assert_eq!(failed.failure_reason.as_deref(), Some("carrier"));

    let Event::CallSummary(summary) = next() else {
        panic!("call.summary")
    };
    assert_eq!(summary.request_id.as_deref(), Some("r-sum"));
    assert_eq!(summary.call_id, "call-1");
    assert_eq!(summary.status, CallStatus::Completed);
    assert_eq!(summary.duration_seconds, Some(42));
    assert_eq!(summary.transcript.as_deref(), Some("hello"));
    assert_eq!(summary.summary.as_deref(), Some("short"));
    assert_eq!(summary.credit_charged, Some(1.5));

    let Event::Error(error) = next() else {
        panic!("error")
    };
    assert_eq!(error.code, "callRejected");
    assert_eq!(error.message, "Call rejected");
    assert_eq!(error.request_id.as_deref(), Some("r-err"));
    assert_eq!(error.question.as_deref(), Some("Who is calling?"));

    assert!(matches!(next(), Event::Unknown(raw) if raw["type"] == "future.thing"));
}

#[tokio::test]
async fn call_summary_null_fields_and_absent_optionals_are_none() {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;

    conn.send_json(json!({
        "type": "call.summary", "version": "1.0", "callId": "call-1", "status": "failed",
        "durationSeconds": null, "transcript": null, "summary": null, "creditCharged": null,
    }))
    .await;
    conn.send_json(stream_frame(
        "answer.accepted",
        json!({ "messageId": "m1" }),
    ))
    .await;
    conn.send_json(error_frame("toRequired", None)).await;

    let Event::CallSummary(summary) = next_event(&mut events).await else {
        panic!("call.summary")
    };
    assert_eq!(summary.request_id, None);
    assert_eq!(summary.status, CallStatus::Failed);
    assert_eq!(
        (
            summary.duration_seconds,
            summary.transcript,
            summary.summary,
            summary.credit_charged
        ),
        (None, None, None, None)
    );

    let Event::AnswerAccepted(answer) = next_event(&mut events).await else {
        panic!("answer.accepted")
    };
    assert_eq!(answer.request_id, None);

    let Event::Error(error) = next_event(&mut events).await else {
        panic!("error")
    };
    assert_eq!((error.request_id, error.question), (None, None));
}

#[tokio::test]
async fn unknown_status_is_kept_verbatim() {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;

    conn.send_json(stream_frame(
        "call.statusChanged",
        json!({ "status": "onHold", "previousStatus": "inProgress" }),
    ))
    .await;

    let Event::CallStatusChanged(changed) = next_event(&mut events).await else {
        panic!("call.statusChanged")
    };
    assert_eq!(changed.status, CallStatus::Other("onHold".to_owned()));
    assert_eq!(changed.status.as_str(), "onHold");
}

#[tokio::test]
async fn disconnected_is_the_last_event_then_the_stream_ends() {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;

    conn.send_json(stream_frame("call.created", json!({})))
        .await;
    conn.close_with(1000, "bye").await;

    assert!(matches!(
        next_event(&mut events).await,
        Event::CallCreated(_)
    ));
    let disconnected = next_event(&mut events).await;
    assert_eq!(disconnected, Event::Disconnected);
    assert_eq!(disconnected.raw(), &Value::Null);
    assert_eq!(within("stream end", events.recv()).await, None);
}

#[tokio::test]
async fn events_is_a_stream() {
    use futures_util::StreamExt;

    let gateway = FakeGateway::start().await;
    let (_client, events, mut conn) = common::connect(&gateway).await;

    conn.send_json(stream_frame(
        "user.turn",
        json!({ "turnIndex": 7, "text": "hi" }),
    ))
    .await;
    conn.close_with(1000, "bye").await;

    let collected: Vec<Event> = within("stream", events.collect()).await;
    assert!(
        matches!(&collected[..], [Event::UserTurn(turn), Event::Disconnected] if turn.turn_index == 7)
    );
}
