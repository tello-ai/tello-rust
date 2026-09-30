//! Minimal end-to-end example: connect, start a call, answer each user turn.
//!
//! This places a real call. Replace the placeholder recipient below with a
//! controlled test number first, then run it against a gateway:
//!
//! ```sh
//! TELLO_API_KEY=tello_live_xxx TELLO_URL=ws://localhost:3000/sdk \
//!     cargo run --example basic_call
//! ```

use tello::{Answer, Client, Config, CreateCall, Event};

/// Placeholder recipient. Replace it before running.
const TO: &str = "+821012345678";

#[tokio::main]
async fn main() -> Result<(), tello::Error> {
    // Config::from_env() reads TELLO_API_KEY; the URL is TELLO_URL, falling
    // back to ws://localhost:3000/sdk. connect() returns only after auth.ok.
    let (client, mut events) = Client::connect(Config::from_env()).await?;

    // Consume events on their own task so wait_closed below never blocks them.
    let answerer = client.clone();
    let consumer = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                Event::UserTurn(turn) => {
                    println!("[user #{}] {}", turn.turn_index, turn.text);
                    let reply = Answer::new("확인했습니다. 계속 말씀해주세요.");
                    if let Err(error) = answerer.answer(reply).await {
                        eprintln!("[answer failed] {error}");
                    }
                }
                Event::AgentTurn(turn) => println!("[agent #{}] {}", turn.turn_index, turn.text),
                Event::CallCreated(created) => println!("[created] {}", created.call_id),
                Event::CallStatusChanged(changed) => println!(
                    "[status] {} -> {}",
                    changed.previous_status.as_str(),
                    changed.status.as_str()
                ),
                Event::CallCompleted(ended) => println!("[completed] {}", ended.call_id),
                Event::CallNoAnswer(ended) | Event::CallFailed(ended) => println!(
                    "[ended] {} {}",
                    ended.status.as_str(),
                    ended.failure_reason.unwrap_or_default()
                ),
                Event::Error(error) => println!("[error] {}: {}", error.code, error.message),
                Event::Disconnected => println!("[disconnected]"),
                _ => {}
            }
        }
    });

    let request_id = client
        .create_call(CreateCall::new(TO).prompt("예약 확인"))
        .await?;
    println!("[createCall] requestId {request_id}");

    // Returns when the call reaches a terminal event, or with the error that
    // ended it, so a refused createCall does not hang.
    let outcome = client.wait_closed().await;
    client.close().await;
    if let Err(error) = consumer.await {
        eprintln!("[event task] {error}");
    }
    outcome
}
