use crate::error::EngineError;
use object_store::aws::AmazonS3Builder;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::http::HttpBuilder;
use object_store::ObjectStore;
use std::sync::Arc;
use tv_core::{Credentials, SourceKind};
use url::Url;

type StoreResult = Result<Option<(Url, Arc<dyn ObjectStore>)>, EngineError>;

pub fn build_object_store(kind: &SourceKind, creds: &Credentials, uri: &str) -> StoreResult {
    match kind {
        SourceKind::S3 | SourceKind::HuggingFace => build_s3(creds, uri),
        SourceKind::Gcs => build_gcs(creds, uri),
        SourceKind::Http => build_http(uri),
        _ => Ok(None),
    }
}

fn build_s3(creds: &Credentials, uri: &str) -> StoreResult {
    let mut builder = AmazonS3Builder::new().with_url(uri);
    if let Some(k) = &creds.access_key {
        builder = builder.with_access_key_id(k);
    }
    if let Some(s) = &creds.secret_key {
        builder = builder.with_secret_access_key(s);
    }
    if let Some(t) = &creds.session_token {
        builder = builder.with_token(t);
    }
    if let Some(r) = &creds.region {
        builder = builder.with_region(r);
    }
    if let Some(e) = &creds.endpoint {
        builder = builder.with_endpoint(e);
    }
    let store = builder
        .build()
        .map_err(|e| EngineError::Query(e.to_string()))?;
    let url = Url::parse(uri).map_err(|e| EngineError::Query(e.to_string()))?;
    Ok(Some((url, Arc::new(store))))
}

fn build_gcs(creds: &Credentials, uri: &str) -> StoreResult {
    let mut builder = GoogleCloudStorageBuilder::new().with_url(uri);
    if let Some(k) = &creds.access_key {
        builder = builder.with_service_account_key(k);
    }
    let store = builder
        .build()
        .map_err(|e| EngineError::Query(e.to_string()))?;
    let url = Url::parse(uri).map_err(|e| EngineError::Query(e.to_string()))?;
    Ok(Some((url, Arc::new(store))))
}

fn build_http(uri: &str) -> StoreResult {
    let url = Url::parse(uri).map_err(|e| EngineError::Query(e.to_string()))?;
    let store = HttpBuilder::new()
        .with_url(uri)
        .build()
        .map_err(|e| EngineError::Query(e.to_string()))?;
    Ok(Some((url, Arc::new(store))))
}
