//! `crews().generate`, `crew_generations().get` and `crews().generate_and_wait`.

mod fake_api;

use std::time::Duration;

use fake_api::{FakeApi, Reply};
use serde_json::json;
use tello::{Error, GenerateAndWait};

const GOAL: &str = "치과 예약 고객에게 전화해 내일 예약을 확인한다";

fn job(status: &str) -> serde_json::Value {
    json!({"id": "57", "status": status, "goal": GOAL})
}

fn fast() -> GenerateAndWait {
    GenerateAndWait::new()
        .poll_interval(Duration::from_millis(10))
        .timeout(Duration::from_secs(3))
}

#[tokio::test]
async fn generate_posts_the_input_and_accepts_202() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, job("running")));
    let input = json!({"goal": GOAL});

    let started = fake
        .api()
        .crews()
        .generate(&input, None)
        .await
        .expect("generate");

    assert_eq!(started, job("running"));
    let request = fake.request().await;
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/v1/crews/generate");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("idempotency-key"), None);
    assert_eq!(request.json(), input);
}

#[tokio::test]
async fn generate_accepts_200_for_the_job_of_the_same_goal() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, job("running")));

    let started = fake
        .api()
        .crews()
        .generate(&json!({"goal": GOAL}), Some("gen-1"))
        .await
        .expect("generate");

    assert_eq!(started["id"], "57");
    assert_eq!(
        fake.request().await.header("idempotency-key"),
        Some("gen-1")
    );
}

#[tokio::test]
async fn generate_answered_with_201_is_an_invalid_response() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(201, job("running")));

    let error = fake
        .api()
        .crews()
        .generate(&json!({"goal": GOAL}), None)
        .await
        .expect_err("201");

    assert_eq!(error.code(), Some("invalid_response"));
}

#[tokio::test]
async fn generate_uses_the_long_timeout() {
    let fake = FakeApi::start().await;
    let slow = || {
        Reply::Delayed(
            Duration::from_millis(300),
            Box::new(Reply::Json(202, job("running"))),
        )
    };
    fake.reply(slow());
    fake.reply(slow());
    let api = fake.api_with(|config| {
        config
            .with_timeout(Duration::from_millis(100))
            .with_long_timeout(Duration::from_secs(5))
    });

    api.crews()
        .generate(&json!({"goal": GOAL}), None)
        .await
        .expect("within the long timeout");
    let error = api.crew_generations().get("57").await.expect_err("short");
    assert!(matches!(error, Error::ApiConnection(_)), "{error:?}");
}

#[tokio::test]
async fn get_fetches_the_job() {
    let mut fake = FakeApi::start().await;
    let done = json!({"id": "57", "status": "succeeded", "crew": {"id": "35"}, "error": null});
    fake.reply(Reply::Json(200, done.clone()));

    let job = fake.api().crew_generations().get("57").await.expect("get");

    assert_eq!(job, done);
    let request = fake.request().await;
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/v1/crew-generations/57");
}

#[tokio::test]
async fn get_of_an_unknown_job_keeps_the_snake_case_code() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        404,
        json!({"statusCode": 404, "message": "Not found", "error": "crew_generation_not_found", "data": null}),
    ));

    let error = fake
        .api()
        .crew_generations()
        .get("999999")
        .await
        .expect_err("404");

    assert_eq!(error.code(), Some("crew_generation_not_found"));
}

#[tokio::test]
async fn generate_and_wait_polls_until_the_job_is_not_running() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, job("running")));
    fake.reply(Reply::Json(200, job("running")));
    let done = json!({"id": "57", "status": "succeeded", "crew": {"id": "35"}});
    fake.reply(Reply::Json(200, done.clone()));

    let finished = fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": GOAL}), fast().idempotency_key("gen-2"))
        .await
        .expect("wait");

    assert_eq!(finished, done);
    let generate = fake.request().await;
    assert_eq!(generate.target, "/v1/crews/generate");
    assert_eq!(generate.header("idempotency-key"), Some("gen-2"));
    for _ in 0..2 {
        let poll = fake.request().await;
        assert_eq!(
            (poll.method.as_str(), poll.target.as_str()),
            ("GET", "/v1/crew-generations/57")
        );
        assert_eq!(poll.header("idempotency-key"), None);
    }
    fake.assert_no_request();
}

#[tokio::test]
async fn generate_and_wait_returns_a_finished_job_without_polling() {
    let mut fake = FakeApi::start().await;
    fake.reply(Reply::Json(200, job("succeeded")));

    let finished = fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": GOAL}), fast())
        .await
        .expect("wait");

    assert_eq!(finished["status"], "succeeded");
    fake.request().await;
    fake.assert_no_request();
}

#[tokio::test]
async fn generate_and_wait_returns_a_failed_job_as_ok() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, job("running")));
    let failed = json!({"id": "57", "status": "failed", "error": {"code": "generation_failed", "message": "no"}});
    fake.reply(Reply::Json(200, failed.clone()));

    let finished = fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": GOAL}), fast())
        .await
        .expect("a failed job is not an error");

    assert_eq!(finished, failed);
}

#[tokio::test]
async fn generate_and_wait_times_out_with_the_job_id() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, job("running")));
    for _ in 0..100 {
        fake.reply(Reply::Json(200, job("running")));
    }
    let options = GenerateAndWait::new()
        .poll_interval(Duration::from_millis(20))
        .timeout(Duration::from_millis(150));

    let error = fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": GOAL}), options)
        .await
        .expect_err("timeout");

    match error {
        Error::CrewGenerationTimeout { id, timeout } => {
            assert_eq!(id, "57");
            assert_eq!(timeout, Duration::from_millis(150));
        }
        other => panic!("expected Error::CrewGenerationTimeout, got {other:?}"),
    }
}

#[tokio::test]
async fn generate_and_wait_raises_crew_generation_in_progress_with_its_data() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(
        409,
        json!({"statusCode": 409, "message": "In progress", "error": "crew_generation_in_progress", "data": {"id": "56"}}),
    ));

    match fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": "another"}), fast())
        .await
    {
        Err(Error::Api(error)) => {
            assert_eq!(error.status, 409);
            assert_eq!(error.code, "crew_generation_in_progress");
            assert_eq!(error.data, Some(json!({"id": "56"})));
        }
        other => panic!("expected Error::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn generate_and_wait_refuses_a_running_job_without_an_id() {
    let fake = FakeApi::start().await;
    fake.reply(Reply::Json(202, json!({"status": "running"})));

    let error = fake
        .api()
        .crews()
        .generate_and_wait(&json!({"goal": GOAL}), fast())
        .await
        .expect_err("no id");

    assert_eq!(error.code(), Some("invalid_response"));
}
