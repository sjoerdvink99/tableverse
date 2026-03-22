use crate::state::AppState;
use axum::extract::State;
use axum::Json;

pub async fn list_profiles(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let names = tv_engine::profiles::list_profile_names();
    Json(serde_json::json!({ "profiles": names }))
}
