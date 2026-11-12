use std::time::Duration;

use reqwest::Method;
use serde_json::Value;
use tokio::time::Instant;

use super::{invalid_response, Api, Crews, Request, ACCEPTED_OR_OK};
use crate::Error;

const RUNNING: &str = "running";

/// Crew generation jobs: `/v1/crew-generations`. Get one with
/// [`Api::crew_generations`].
#[derive(Debug, Clone, Copy)]
pub struct CrewGenerations<'a> {
    pub(super) api: &'a Api,
}

impl CrewGenerations<'_> {
    /// `GET /crew-generations/{id}`: the job,
    /// `{"id", "status": "running" | "succeeded" | "failed", "crew"?, "error"?, …}`.
    pub async fn get(self, id: &str) -> Result<Value, Error> {
        self.api.get_item("crew-generations", id).await
    }
}

/// How [`Crews::generate_and_wait`] follows the job: poll every 2s, give up
/// after 600s, no idempotency key unless set.
///
/// ```
/// use std::time::Duration;
///
/// let options = tello::GenerateAndWait::new()
///     .poll_interval(Duration::from_secs(5))
///     .timeout(Duration::from_secs(300))
///     .idempotency_key("crew-dental-001");
/// ```
#[derive(Debug, Clone)]
pub struct GenerateAndWait {
    poll_interval: Duration,
    timeout: Duration,
    idempotency_key: Option<String>,
}

impl Default for GenerateAndWait {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(2),
            timeout: Duration::from_secs(600),
            idempotency_key: None,
        }
    }
}

impl GenerateAndWait {
    /// Poll every 2s for up to 600s, with no idempotency key.
    pub fn new() -> Self {
        Self::default()
    }

    /// The wait between two job lookups.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// How long to follow the job once it has started.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The `Idempotency-Key` of the generate request.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

impl Crews<'_> {
    /// `POST /crews/generate`: starts building, deploying and publishing a
    /// crew from `input` (`{"goal": …}` and the other documented fields).
    /// Returns the job, `{"id", "status": "running", …}`; follow it with
    /// [`Api::crew_generations`] or use
    /// [`generate_and_wait`](Self::generate_and_wait).
    ///
    /// The account builds one crew at a time. The same goal while it runs
    /// returns that job (`200`); another goal is refused with `409`
    /// `crew_generation_in_progress`, whose `data.id` is the running job.
    pub async fn generate(
        self,
        input: &Value,
        idempotency_key: Option<&str>,
    ) -> Result<Value, Error> {
        self.api
            .object(Self::generate_request(input, idempotency_key))
            .await
    }

    /// [`generate`](Self::generate), then looks the job up every
    /// `poll_interval` until it is no longer `running`, and returns it.
    ///
    /// A `failed` job is returned, not raised: branch on its `status`. Past
    /// `timeout` this fails with
    /// [`Error::CrewGenerationTimeout`](crate::Error::CrewGenerationTimeout)
    /// carrying the job id; the job keeps running on the server. Errors of
    /// the requests, e.g. `crew_generation_in_progress`, are returned as they
    /// are.
    pub async fn generate_and_wait(
        self,
        input: &Value,
        options: GenerateAndWait,
    ) -> Result<Value, Error> {
        let (status, mut job) = self
            .api
            .object_with_status(Self::generate_request(
                input,
                options.idempotency_key.as_deref(),
            ))
            .await?;
        if !is_running(&job) {
            return Ok(job);
        }
        let Some(id) = job.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return Err(invalid_response(
                status,
                "the running crew generation job has no id",
                options.idempotency_key.as_deref(),
            ));
        };
        let deadline = Instant::now() + options.timeout;
        let generations = self.api.crew_generations();
        while is_running(&job) {
            let poll = async {
                tokio::time::sleep(options.poll_interval).await;
                generations.get(&id).await
            };
            job = match tokio::time::timeout_at(deadline, poll).await {
                Ok(polled) => polled?,
                Err(_) => {
                    return Err(Error::CrewGenerationTimeout {
                        id,
                        timeout: options.timeout,
                    })
                }
            };
        }
        Ok(job)
    }

    fn generate_request<'a>(input: &'a Value, idempotency_key: Option<&'a str>) -> Request<'a> {
        Request::new(Method::POST, &["crews", "generate"], ACCEPTED_OR_OK)
            .body(input)
            .idempotency_key(idempotency_key)
            .long()
    }
}

fn is_running(job: &Value) -> bool {
    job.get("status").and_then(Value::as_str) == Some(RUNNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_wait_polls_every_2s_for_up_to_600s() {
        let options = GenerateAndWait::new();
        assert_eq!(options.poll_interval, Duration::from_secs(2));
        assert_eq!(options.timeout, Duration::from_secs(600));
        assert_eq!(options.idempotency_key, None);
    }
}
