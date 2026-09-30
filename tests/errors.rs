mod common;

use common::{next_event, FakeGateway};
use serde_json::{json, Value};
use tello::{Error, Event};

/// The SDK exception class named by errors.v1.json for each error variant.
fn declared_exception(error: &Error) -> &'static str {
    match error {
        Error::Authentication { .. } => "AuthenticationError",
        Error::Validation { .. } => "ValidationError",
        Error::CallAlreadyActive { .. } => "CallAlreadyActiveError",
        Error::NoActiveCall { .. } => "NoActiveCallError",
        Error::CallRejected { .. } => "CallRejectedError",
        Error::CallRefused { .. } => "CallRefusedError",
        Error::CallProvider { .. } => "CallProviderError",
        Error::Server { .. } => "TelloServerError",
        other => panic!("not a gateway error: {other:?}"),
    }
}

fn catalog() -> Vec<Value> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/errors/errors.v1.json");
    let text = std::fs::read_to_string(path).expect("read errors.v1.json");
    let catalog: Value = serde_json::from_str(&text).expect("parse errors.v1.json");
    catalog["errors"].as_array().expect("errors array").clone()
}

/// Sends `frames` through the fake gateway and returns the error events.
async fn received_errors(frames: &[Value]) -> Vec<tello::ErrorEvent> {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;
    for frame in frames {
        conn.send_json(frame.clone()).await;
    }
    let mut received = Vec::new();
    for _ in frames {
        match next_event(&mut events).await {
            Event::Error(error) => received.push(error),
            other => panic!("expected an error event, got {other:?}"),
        }
    }
    received
}

#[tokio::test]
async fn every_catalog_code_maps_to_its_declared_error() {
    let catalog = catalog();
    assert_eq!(catalog.len(), 19, "errors.v1.json declares 19 codes");

    let frames: Vec<Value> = catalog
        .iter()
        .map(|entry| {
            let mut frame = json!({
                "type": "error", "version": "1.0",
                "code": entry["code"], "message": entry["message"], "requestId": "r-1",
            });
            if entry["extra"]
                .as_array()
                .is_some_and(|extra| extra.contains(&json!("question")))
            {
                frame["question"] = json!("Who is calling?");
            }
            frame
        })
        .collect();

    let received = received_errors(&frames).await;

    for (entry, event) in catalog.iter().zip(&received) {
        let code = entry["code"].as_str().expect("code");
        let error = event.to_error();
        assert_eq!(
            declared_exception(&error),
            entry["sdkException"],
            "code {code} mapped to {error:?}"
        );
        assert_eq!(error.code(), Some(code), "{error:?}");
        assert!(
            error
                .to_string()
                .contains(entry["message"].as_str().expect("message")),
            "message kept for {code}: {error}"
        );
    }
}

#[tokio::test]
async fn call_rejected_carries_the_question() {
    let received = received_errors(&[
        json!({ "type": "error", "version": "1.0", "code": "callRejected",
                "message": "Call rejected", "question": "Who is calling?" }),
        json!({ "type": "error", "version": "1.0", "code": "callRejected",
                "message": "Call rejected" }),
    ])
    .await;

    assert!(matches!(
        received[0].to_error(),
        Error::CallRejected { question: Some(q), .. } if q == "Who is calling?"
    ));
    assert!(matches!(
        received[1].to_error(),
        Error::CallRejected { question: None, .. }
    ));
}

#[tokio::test]
async fn unknown_codes_map_to_server_error_keeping_the_code() {
    let received = received_errors(&[json!({
        "type": "error", "version": "1.0", "code": "somethingNew", "message": "New failure",
    })])
    .await;

    let error = received[0].to_error();
    assert!(
        matches!(&error, Error::Server { code, message } if code == "somethingNew" && message == "New failure"),
        "got {error:?}"
    );
    assert_eq!(error.code(), Some("somethingNew"));
}

#[test]
fn sdk_raised_errors_have_no_gateway_code() {
    let closed = Error::ConnectionClosed {
        message: "gone".into(),
    };
    let replaced = Error::SessionReplaced {
        message: "session replaced".into(),
    };
    assert_eq!(closed.code(), None);
    assert_eq!(replaced.code(), None);
    assert_eq!(Error::Config("no key".into()).code(), None);
}
