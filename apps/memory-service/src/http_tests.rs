use super::*;
use crate::store::test_support::memory_store;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tower::ServiceExt;

fn app() -> Router {
    let state = AppState {
        store: Arc::new(memory_store("http")),
        requests: Arc::new(Semaphore::new(16)),
    };
    router(state)
}

async fn call(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let response = router.clone().oneshot(request).await.unwrap();
    let headers = response.headers().clone();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body, headers)
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder().method(method).uri(uri);
    match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn every_response_carries_the_boundary_headers() {
    let router = app();
    // healthz sits outside the gate by design; check it answers plainly.
    let (status, body, _) = call(&router, json_request("GET", "/healthz", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    // Gated endpoints carry the boundary headers.
    let (status, _, headers) = call(&router, json_request("GET", "/v1/stats", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn memory_crud_round_trip_and_errors() {
    let router = app();

    let (status, body, _) = call(
        &router,
        json_request(
            "POST",
            "/v1/memories",
            Some(json!({"type": "decision", "title": "投 JAAMAS", "content": "论文决定。", "project": "R2R", "importance": 5, "tags": ["论文"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["stored"], true);
    let id = body["id"].as_i64().unwrap();

    let (status, body, _) = call(
        &router,
        json_request("GET", "/v1/memories?project=R2R&limit=5", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["memories"][0]["id"], id);
    assert_eq!(body["memories"][0]["type"], "decision");

    let (status, body, _) = call(
        &router,
        json_request("GET", &format!("/v1/memories/{id}"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["memory"]["title"], "投 JAAMAS");
    assert_eq!(body["chain"].as_array().unwrap().len(), 1);

    let (status, body, _) = call(
        &router,
        json_request(
            "PATCH",
            &format!("/v1/memories/{id}"),
            Some(json!({"content": "更新后的决定。"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body, _) = call(
        &router,
        json_request(
            "POST",
            &format!("/v1/memories/{id}/supersede"),
            Some(json!({"title": "改投 COINE", "content": "改投决定。"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["oldId"], id);
    let new_id = body["newId"].as_i64().unwrap();
    assert_ne!(new_id, id);

    let (status, body, _) = call(
        &router,
        json_request("GET", &format!("/v1/memories/{id}"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["chain"].as_array().unwrap().len(),
        2,
        "chain covers both"
    );

    let (status, body, _) = call(
        &router,
        json_request("POST", &format!("/v1/memories/{new_id}/forget"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body, _) = call(&router, json_request("GET", "/v1/stats", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(body["active"], 0);
    assert_eq!(body["superseded"], 1);

    // Missing rows are 404 with the stable error envelope.
    let (status, body, _) = call(
        &router,
        json_request("PATCH", "/v1/memories/999", Some(json!({"content": "x"}))),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    // Bad input is 400, never a framework default page.
    let (status, body, _) = call(
        &router,
        json_request("POST", "/v1/memories", Some(json!({"title": "缺内容"}))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    let (status, body, _) = call(
        &router,
        json_request(
            "POST",
            "/v1/memories",
            Some(json!({"type": "bogus", "title": "t", "content": "c"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    let (status, body, _) =
        call(&router, json_request("GET", "/v1/memories?limit=abc", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");

    // Unknown fields are rejected.
    let (status, _, _) = call(
        &router,
        json_request(
            "POST",
            "/v1/memories",
            Some(json!({"title": "t", "content": "c", "extra": 1})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sync_and_personality_endpoints_answer() {
    let router = app();
    call(
        &router,
        json_request(
            "POST",
            "/v1/memories",
            Some(json!({"type": "preference", "title": "偏好", "content": "简洁代码。"})),
        ),
    )
    .await;
    call(
        &router,
        json_request(
            "POST",
            "/v1/memories",
            Some(json!({"type": "fact", "title": "事实", "content": "不入人格。"})),
        ),
    )
    .await;

    let (status, body, _) = call(&router, json_request("GET", "/v1/sync?since=0", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["memories"].as_array().unwrap().len(), 2);
    assert_eq!(body["currentSeq"], 2);

    let (status, body, _) = call(&router, json_request("GET", "/v1/sync?since=2", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["memories"].as_array().unwrap().len(), 0);

    let (status, body, _) = call(
        &router,
        json_request("GET", "/v1/personality/summary", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kinds: Vec<&str> = body["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["preference", "insight", "person"]);
    assert_eq!(body["sections"][0]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(body["sections"][1]["entries"].as_array().unwrap().len(), 0);

    let (status, body, _) = call(&router, json_request("GET", "/v1/sync?since=-1", None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn concurrency_gate_returns_busy_when_saturated() {
    let state = AppState {
        store: Arc::new(memory_store("http")),
        requests: Arc::new(Semaphore::new(1)),
    };
    let _permit = state.requests.clone().acquire_owned().await.unwrap();
    let router = router(state);
    let (status, body, _) = call(&router, json_request("GET", "/v1/stats", None)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "busy");
}

#[tokio::test]
async fn real_tcp_listener_serves_healthz() {
    let state = AppState {
        store: Arc::new(memory_store("http")),
        requests: Arc::new(Semaphore::new(16)),
    };
    let listener = tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    let client = reqwest::Client::new();
    let health = client
        .get(format!("http://{address}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), reqwest::StatusCode::OK);
    let body: Value = health.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    let stats = client
        .get(format!("http://{address}/v1/stats"))
        .send()
        .await
        .unwrap();
    assert_eq!(stats.status(), reqwest::StatusCode::OK);
    assert_eq!(stats.headers()["cache-control"], "no-store");
}
