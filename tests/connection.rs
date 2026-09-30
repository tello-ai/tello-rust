mod common;

use std::time::Duration;

use common::{events_until, next_event, within, FakeGateway};
use futures_util::StreamExt;
use serde_json::json;
use tello::{Client, Event};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn idle_client_answers_server_pings_while_events_pile_up() {
    let gateway = FakeGateway::start().await;
    // Nobody reads `_events` in this test: the read task must not wait for a
    // consumer before it can answer pings.
    let (_client, _events, mut conn) = common::connect(&gateway).await;

    for turn in 0..2_000 {
        conn.send_json(common::stream_frame(
            "user.turn",
            json!({ "turnIndex": turn, "text": "unread" }),
        ))
        .await;
    }
    conn.send(Message::Ping(b"hb-1".to_vec().into())).await;
    conn.expect_pong(b"hb-1").await;

    conn.send(Message::Ping(b"hb-2".to_vec().into())).await;
    conn.expect_pong(b"hb-2").await;
}

#[tokio::test]
async fn close_sends_normal_close_and_ends_the_event_stream() {
    let gateway = FakeGateway::start().await;
    // Far beyond WAIT: close must finish through the handshake, not the timeout.
    let config = gateway
        .config()
        .with_close_timeout(Duration::from_secs(600));
    let (client, conn) = tokio::join!(Client::connect(config), gateway.accept_authed());
    let (client, mut events) = client.expect("connect");
    let mut conn = conn;

    let server = async move {
        let frame = conn.expect_close().await;
        // Reading once more flushes tungstenite's close reply; dropping the
        // connection then ends the TCP stream, as the gateway does.
        let _reply_flushed = within("close reply", conn.ws.next()).await;
        frame
    };
    let (_, frame) = tokio::join!(within("close", client.close()), server);

    assert_eq!(frame.map(|f| f.code), Some(CloseCode::Normal));
    assert_eq!(next_event(&mut events).await, Event::Disconnected);
    assert_eq!(within("stream end", events.recv()).await, None);
}

#[tokio::test]
async fn close_gives_up_after_the_close_timeout() {
    let gateway = FakeGateway::start().await;
    let config = gateway
        .config()
        .with_close_timeout(Duration::from_millis(200));
    let (client, conn) = tokio::join!(Client::connect(config), gateway.accept_authed());
    let (client, mut events) = client.expect("connect");

    // The server never reads, so it never answers the close frame.
    within("close", client.close()).await;

    assert!(events_until(&mut events, |e| *e == Event::Disconnected)
        .await
        .is_empty());
    drop(conn);
}

#[tokio::test]
async fn a_server_close_ends_the_connection_even_if_the_server_keeps_tcp_open() {
    let gateway = FakeGateway::start().await;
    let config = gateway
        .config()
        .with_close_timeout(Duration::from_millis(200));
    let (client, conn) = tokio::join!(Client::connect(config), gateway.accept_authed());
    let (client, mut events) = client.expect("connect");
    let mut conn = conn;

    // Close frame, but the server neither reads the reply nor drops TCP.
    conn.send_close(1001, "shutting_down").await;

    assert!(events_until(&mut events, |e| *e == Event::Disconnected)
        .await
        .is_empty());
    within("wait_closed", client.wait_closed())
        .await
        .expect("no call was active");
    drop(conn);
}
