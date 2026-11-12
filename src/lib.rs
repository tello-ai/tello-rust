//! Rust SDK for Tello: the `/sdk` WebSocket protocol and the public REST API.
//!
//! [`Client`] speaks the WebSocket protocol: the API gateway streams each
//! callee turn of a live phone call to you, and your reply is spoken back
//! into the call. [`Api`] is a separate client of the public REST API (`/v1`)
//! for tools, agents, crews, crew generation and call drafts.
//!
//! ```no_run
//! use tello::{Answer, Client, Config, CreateCall, Event};
//!
//! # async fn run() -> Result<(), tello::Error> {
//! // Reads TELLO_API_KEY, and TELLO_URL or ws://localhost:3000/sdk.
//! let (client, mut events) = Client::connect(Config::from_env()).await?;
//!
//! let answerer = client.clone();
//! tokio::spawn(async move {
//!     while let Some(event) = events.recv().await {
//!         if let Event::UserTurn(turn) = event {
//!             let reply = format!("heard: {}", turn.text);
//!             if answerer.answer(Answer::new(reply)).await.is_err() {
//!                 break;
//!             }
//!         }
//!     }
//! });
//!
//! client
//!     .create_call(CreateCall::new("+821012345678").prompt("reservation check"))
//!     .await?;
//! client.wait_closed().await?;
//! client.close().await;
//! # Ok(())
//! # }
//! ```
//!
//! ```no_run
//! # async fn run() -> Result<(), tello::Error> {
//! // Reads TELLO_API_KEY, and TELLO_API_URL or https://api.telloai.io/v1.
//! let api = tello::Api::from_env()?;
//! let agents = api.agents().list().await?;
//! println!("{}", agents["data"]);
//! # Ok(())
//! # }
//! ```
//!
//! The WebSocket contract ships with the crate under `docs/`.

mod api;
mod client;
mod command;
mod config;
mod error;
mod event;

pub use api::{
    Agents, Api, ApiConfig, CallDrafts, CrewGenerations, Crews, GenerateAndWait, ListTools, Tools,
    DEFAULT_API_URL, ENV_API_URL,
};
pub use client::{Client, Events};
pub use command::{Answer, CreateCall, GetSummary, SendDtmf};
pub use config::{Config, DEFAULT_URL, ENV_API_KEY, ENV_URL};
pub use error::{ApiConnectionError, ApiError, Error, TransportError};
pub use event::{
    AnswerAccepted, CallCreated, CallEnded, CallStatus, CallStatusChanged, CallSummary,
    DtmfAccepted, ErrorEvent, Event, Turn,
};

/// The Tello WebSocket protocol version this SDK implements.
pub const PROTOCOL_VERSION: &str = "1.0";
