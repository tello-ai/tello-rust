//! `Api::agents` and `Api::crews`: list, get, create, update, delete.

mod fake_api;

use fake_api::{FakeApi, Reply};
use serde_json::{json, Value};
use tello::{Api, Error};

async fn list(api: &Api, resource: &str) -> Result<Value, Error> {
    match resource {
        "agents" => api.agents().list().await,
        _ => api.crews().list().await,
    }
}

async fn get(api: &Api, resource: &str, id: &str) -> Result<Value, Error> {
    match resource {
        "agents" => api.agents().get(id).await,
        _ => api.crews().get(id).await,
    }
}

async fn create(
    api: &Api,
    resource: &str,
    body: &Value,
    key: Option<&str>,
) -> Result<Value, Error> {
    match resource {
        "agents" => api.agents().create(body, key).await,
        _ => api.crews().create(body, key).await,
    }
}

async fn update(api: &Api, resource: &str, id: &str, patch: &Value) -> Result<Value, Error> {
    match resource {
        "agents" => api.agents().update(id, patch).await,
        _ => api.crews().update(id, patch).await,
    }
}

async fn delete(api: &Api, resource: &str, id: &str) -> Result<(), Error> {
    match resource {
        "agents" => api.agents().delete(id).await,
        _ => api.crews().delete(id).await,
    }
}

#[tokio::test]
async fn list_returns_the_page_as_sent() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;
        let page = json!({"data": [{"id": "3", "name": "front desk", "description": null, "isDefault": true}]});
        fake.reply(Reply::Json(200, page.clone()));

        assert_eq!(list(&fake.api(), resource).await.expect(resource), page);
        let request = fake.request().await;
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, format!("/v1/{resource}"));
    }
}

#[tokio::test]
async fn get_fetches_one_by_id() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;
        fake.reply(Reply::Json(200, json!({"id": "3"})));

        let item = get(&fake.api(), resource, "3").await.expect(resource);

        assert_eq!(item, json!({"id": "3"}));
        let request = fake.request().await;
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, format!("/v1/{resource}/3"));
    }
}

#[tokio::test]
async fn create_posts_the_body_with_an_optional_idempotency_key() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;
        let body = json!({"name": "front desk"});
        fake.reply(Reply::Json(201, json!({"id": "4", "name": "front desk"})));
        fake.reply(Reply::Json(201, json!({"id": "5"})));
        let api = fake.api();

        let created = create(&api, resource, &body, None).await.expect(resource);
        assert_eq!(created["id"], "4");
        let request = fake.request().await;
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, format!("/v1/{resource}"));
        assert_eq!(request.header("content-type"), Some("application/json"));
        assert_eq!(request.header("idempotency-key"), None);
        assert_eq!(request.json(), body);

        create(&api, resource, &body, Some("create-1"))
            .await
            .expect(resource);
        assert_eq!(
            fake.request().await.header("idempotency-key"),
            Some("create-1")
        );
    }
}

#[tokio::test]
async fn create_answered_with_200_is_an_invalid_response() {
    for resource in ["agents", "crews"] {
        let fake = FakeApi::start().await;
        fake.reply(Reply::Json(200, json!({"id": "4"})));

        let error = create(&fake.api(), resource, &json!({}), None)
            .await
            .expect_err(resource);

        assert_eq!(error.code(), Some("invalid_response"), "{resource}");
    }
}

#[tokio::test]
async fn invalid_create_keys_are_refused_without_a_request() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;

        let error = create(&fake.api(), resource, &json!({}), Some("a,b"))
            .await
            .expect_err(resource);

        assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
        fake.assert_no_request();
    }
}

#[tokio::test]
async fn update_patches_and_delete_answers_204() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;
        fake.reply(Reply::Json(200, json!({"id": "3", "name": "renamed"})));
        fake.reply(Reply::Empty(204));
        let api = fake.api();

        let updated = update(&api, resource, "3", &json!({"name": "renamed"}))
            .await
            .expect(resource);
        assert_eq!(updated["name"], "renamed");
        let request = fake.request().await;
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.target, format!("/v1/{resource}/3"));
        assert_eq!(request.json(), json!({"name": "renamed"}));

        delete(&api, resource, "3").await.expect(resource);
        let request = fake.request().await;
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.target, format!("/v1/{resource}/3"));
    }
}

#[tokio::test]
async fn errors_keep_the_snake_case_code_and_data() {
    for (resource, code) in [("agents", "agent_in_use"), ("crews", "crew_in_use")] {
        let fake = FakeApi::start().await;
        fake.reply(Reply::Json(
            409,
            json!({"statusCode": 409, "message": "In use", "error": code, "data": {"reason": "referenced"}}),
        ));

        match delete(&fake.api(), resource, "3").await {
            Err(Error::Api(error)) => {
                assert_eq!(error.status, 409);
                assert_eq!(error.code, code);
                assert_eq!(error.data, Some(json!({"reason": "referenced"})));
            }
            other => panic!("expected Error::Api, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn empty_ids_are_refused_without_a_request() {
    for resource in ["agents", "crews"] {
        let mut fake = FakeApi::start().await;
        let api = fake.api();

        for error in [
            get(&api, resource, "").await.expect_err("get"),
            update(&api, resource, "", &json!({}))
                .await
                .expect_err("update"),
            delete(&api, resource, "").await.expect_err("delete"),
        ] {
            assert!(matches!(error, Error::InvalidArgument(_)), "{error:?}");
        }
        fake.assert_no_request();
    }
}
