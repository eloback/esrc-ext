use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use esrc::prelude::async_nats;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use crate::common::AppState;

use super::*;

#[derive(Debug, Serialize, Deserialize)]
/// This can either be an struct or an enum depending on your webhook payload
pub struct WebhookPayload {
    id: Uuid,
    name: String,
    email: String,
}

#[instrument(name = "webbhook_controller", skip(state))]
pub async fn controller(
    State(state): State<AppState>,
    Json(body): Json<WebhookPayload>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let payload = ExternalEvents::UserCreated {
        user_id: body.id,
        name: body.name,
        email: body.email,
    };

    let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Serialization error: {}", e),
        )
    })?;

    let mut headers = async_nats::HeaderMap::new();
    headers.insert("Webhook-Type", "UserCreated");

    state
        .nats_client
        .publish_with_headers(
            "external.webhook_create_user",
            headers,
            payload_bytes.into(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("NATS error: {}", e),
            )
        })?;

    Ok((StatusCode::OK, "Event published".to_string()))
}
