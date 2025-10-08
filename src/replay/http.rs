use std::sync::Arc;

use axum::{
    extract::Path,
    routing::{patch, post},
    Json, Router,
};

use uuid::Uuid;

use crate::utils::problem_details::ProblemDetails;

use super::*;

impl<DLS, P, G> AdminReplay<DLS, P, G>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    /// Returns the Axum router configured with replay routes
    pub fn router(self) -> Router {
        let admin_replay = Arc::new(self);
        let admin_replay_clone = admin_replay.clone();

        Router::new()
            .route(
                "/admin/dead-letters/replay/:event_id",
                patch(|Path(event_id): Path<Uuid>| async move {
                    replay_one_handler(admin_replay.clone(), event_id).await
                }),
            )
            .route(
                "/admin/dead-letters/replay-all",
                post(|| async move { replay_all_handler(admin_replay_clone).await }),
            )
    }
}

async fn replay_one_handler<DLS, P, G>(
    admin_replay: Arc<AdminReplay<DLS, P, G>>,
    event_id: Uuid,
) -> Result<(), ProblemDetails>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    let mut admin_replay = admin_replay.clone();
    let admin_replay = Arc::get_mut(&mut admin_replay).ok_or_else(|| {
        ProblemDetails::internal_server_error(
            "Failed to get mutable reference to AdminReplay".to_string(),
        )
    })?;

    admin_replay.replay_one(event_id).await.map_err(|e| {
        ProblemDetails::internal_server_error(format!("Failed to replay event: {}", e))
    })?;

    Ok(())
}

async fn replay_all_handler<DLS, P, G>(
    admin_replay: Arc<AdminReplay<DLS, P, G>>,
) -> Result<Json<()>, ProblemDetails>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    let mut admin_replay = admin_replay.clone();
    let admin_replay = Arc::get_mut(&mut admin_replay).ok_or_else(|| {
        ProblemDetails::internal_server_error(
            "Failed to get mutable reference to AdminReplay".to_string(),
        )
    })?;

    admin_replay.replay_all().await.map_err(|e| {
        ProblemDetails::internal_server_error(format!("Failed to replay all events: {}", e))
    })?;

    Ok(Json(()))
}
