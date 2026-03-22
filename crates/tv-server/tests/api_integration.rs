use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tempfile::TempDir;
use tower::util::ServiceExt;
use tv_engine::Engine;
use tv_server::{routes, state::AppState};

async fn make_app() -> axum::Router {
    let engine = Engine::new().unwrap();
    let state = AppState::new(engine, None);
    routes::router(state)
}

fn write_test_parquet() -> (TempDir, std::path::PathBuf) {
    use arrow::array::{Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.parquet");
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("score", DataType::Float64, false),
    ]));
    let ids: Vec<i64> = (0..20).collect();
    let names: Vec<&str> = (0..20usize).map(|_| "test").collect();
    let scores: Vec<f64> = (0..20).map(|i| i as f64).collect();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(names)),
            Arc::new(Float64Array::from(scores)),
        ],
    )
    .unwrap();
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    (dir, path)
}

async fn register_parquet(app: axum::Router, path: &str) -> Value {
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    let req_body =
        format!(r#"{{"uri":"{escaped}","name":null,"profile":null,"credentials":null}}"#);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources")
                .header("Content-Type", "application/json")
                .body(Body::from(req_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn view_expr_json(source_id: &str) -> String {
    format!(r#"{{"source_id":"{source_id}","ops":[]}}"#)
}

#[tokio::test]
async fn health_check() {
    let app = make_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_sources_empty() {
    let app = make_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn register_source_parquet() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app, path.to_str().unwrap()).await;
    assert!(meta["id"].as_str().is_some());
    assert_eq!(meta["n_rows"].as_u64().unwrap(), 20);
}

#[tokio::test]
async fn list_sources_after_register() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    assert!(meta["id"].as_str().is_some());

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sources")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn get_source_by_id() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    let id = meta["id"].as_str().unwrap().to_string();
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/sources/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_source_not_found() {
    let app = make_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/sources/nonexistent-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_source() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    let id = meta["id"].as_str().unwrap().to_string();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/sources/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let get_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/sources/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn query_count_basic() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    let id = meta["id"].as_str().unwrap().to_string();
    let body = format!(r#"{{"view_expr":{}}}"#, view_expr_json(&id));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sources/{id}/query/count"))
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 20);
}

#[tokio::test]
async fn query_tile_returns_arrow_ipc() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    let id = meta["id"].as_str().unwrap().to_string();
    let body = format!(
        r#"{{"view_expr":{},"row":0,"col":0,"rows":10,"cols":3}}"#,
        view_expr_json(&id)
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sources/{id}/query/tiles"))
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response.headers().get("content-type").unwrap();
    assert!(ct.to_str().unwrap().contains("arrow"));
}

#[tokio::test]
async fn query_schema_basic() {
    let (_dir, path) = write_test_parquet();
    let app = make_app().await;
    let meta = register_parquet(app.clone(), path.to_str().unwrap()).await;
    let id = meta["id"].as_str().unwrap().to_string();
    let body = format!(r#"{{"view_expr":{}}}"#, view_expr_json(&id));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/sources/{id}/query/schema"))
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body["columns"].as_array().unwrap().len() >= 3);
}

#[tokio::test]
async fn register_invalid_uri_returns_error() {
    let app = make_app().await;
    let req_body =
        r#"{"uri":"/nonexistent/file.parquet","name":null,"profile":null,"credentials":null}"#;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sources")
                .header("Content-Type", "application/json")
                .body(Body::from(req_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_client_error() || response.status().is_server_error());
}
