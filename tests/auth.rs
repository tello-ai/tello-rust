mod common;

use std::time::Duration;

use common::{next_event, within, FakeGateway, API_KEY};
use serde_json::json;
use tello::{Client, Error};

fn assert_key_absent(error: &Error) {
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains(API_KEY), "API key leaked: {shown}");
}

#[tokio::test]
async fn first_frame_is_auth_and_the_upgrade_carries_no_credentials() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        let first = conn.recv_json().await;
        conn.send_json(json!({ "type": "auth.ok", "version": "1.0", "accountId": "acct-1" }))
            .await;
        (conn, first)
    };
    let (client, (conn, first)) = tokio::join!(Client::connect(gateway.config()), server);
    let (client, _events) = client.expect("connect succeeds after auth.ok");

    assert_eq!(
        first,
        json!({ "event": "auth", "data": { "token": API_KEY } })
    );
    assert!(
        !conn.upgrade.uri.contains(API_KEY),
        "key leaked in the upgrade URI: {}",
        conn.upgrade.uri
    );
    assert!(conn.upgrade.headers.get("authorization").is_none());
    for (name, value) in &conn.upgrade.headers {
        let value = String::from_utf8_lossy(value.as_bytes());
        assert!(!value.contains(API_KEY), "key leaked in header {name}");
    }
    let debug = format!("{client:?}");
    assert!(!debug.contains(API_KEY), "key leaked in Debug: {debug}");
}

#[tokio::test]
async fn auth_ok_is_consumed_and_not_emitted() {
    let gateway = FakeGateway::start().await;
    let (_client, mut events, mut conn) = common::connect(&gateway).await;

    conn.send_json(common::stream_frame("call.created", json!({})))
        .await;

    let first = next_event(&mut events).await;
    assert_eq!(first.raw()["type"], "call.created", "got {first:?}");
}

#[tokio::test]
async fn unauthenticated_error_frame_fails_connect() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        conn.send_json(json!({
            "type": "error", "version": "1.0",
            "code": "unauthenticated", "message": "Authentication required",
        }))
        .await;
        conn.close_with(4401, "unauthenticated").await;
    };
    let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

    let error = result.expect_err("connect must fail");
    assert!(
        matches!(&error, Error::Authentication { message, ip: None, .. } if message == "Authentication required"),
        "got {error:?}"
    );
    assert_eq!(error.code(), Some("unauthenticated"));
    assert_key_absent(&error);
}

#[tokio::test]
async fn close_4401_fails_connect() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        conn.close_with(4401, "unauthenticated").await;
    };
    let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

    let error = result.expect_err("connect must fail");
    assert!(
        matches!(error, Error::Authentication { .. }),
        "got {error:?}"
    );
    assert_key_absent(&error);
}

#[tokio::test]
async fn missing_auth_ok_fails_connect_after_the_open_timeout() {
    let gateway = FakeGateway::start().await;
    let config = gateway
        .config()
        .with_open_timeout(Duration::from_millis(300));

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        // Hold the socket open without ever answering.
        conn
    };
    let (result, _conn) = tokio::join!(within("connect", Client::connect(config)), server);

    let error = result.expect_err("connect must fail");
    assert!(
        matches!(error, Error::Authentication { .. }),
        "got {error:?}"
    );
    assert_key_absent(&error);
}

#[tokio::test]
async fn ip_not_allowed_error_frame_fails_connect_with_the_ip() {
    for (ip, expected) in [
        (json!("203.0.113.7"), Some("203.0.113.7")),
        (json!(null), None),
    ] {
        let gateway = FakeGateway::start().await;

        let server = async {
            let mut conn = gateway.accept().await;
            conn.recv_json().await;
            conn.send_json(json!({
                "type": "error", "version": "1.0", "code": "ipNotAllowed",
                "message": "The request IP address is not in this server key's allowed IP list",
                "ip": ip,
            }))
            .await;
            conn.close_with(4401, "ip_not_allowed").await;
        };
        let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

        let error = result.expect_err("connect must fail");
        assert!(
            matches!(&error, Error::Authentication { code, ip: got, .. }
                if code == "ipNotAllowed" && got.as_deref() == expected),
            "got {error:?}"
        );
        assert_eq!(error.code(), Some("ipNotAllowed"));
        assert_key_absent(&error);
    }
}

#[tokio::test]
async fn close_4401_for_ip_not_allowed_fails_connect_with_its_code() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        conn.close_with(4401, "ip_not_allowed").await;
    };
    let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

    let error = result.expect_err("connect must fail");
    assert_eq!(error.code(), Some("ipNotAllowed"), "got {error:?}");
    assert!(matches!(error, Error::Authentication { ip: None, .. }));
}

#[tokio::test]
async fn internal_error_answering_auth_is_a_server_error() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        conn.send_json(json!({
            "type": "error", "version": "1.0",
            "code": "internalError", "message": "Internal error",
        }))
        .await;
        conn.close_with(1011, "internal_error").await;
    };
    let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

    let error = result.expect_err("connect must fail");
    assert!(
        matches!(&error, Error::Server { code, message } if code == "internalError" && message == "Internal error"),
        "got {error:?}"
    );
    assert_key_absent(&error);
}

#[tokio::test]
async fn close_1011_during_auth_is_a_server_error() {
    let gateway = FakeGateway::start().await;

    let server = async {
        let mut conn = gateway.accept().await;
        conn.recv_json().await;
        conn.close_with(1011, "internal_error").await;
    };
    let (result, ()) = tokio::join!(Client::connect(gateway.config()), server);

    let error = result.expect_err("connect must fail");
    assert!(
        matches!(&error, Error::Server { code, .. } if code == "internalError"),
        "got {error:?}"
    );
}
