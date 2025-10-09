use std::sync::Arc;

use axum::{
    extract::Path,
    routing::{patch, post},
    Json, Router,
};

use uuid::Uuid;

use crate::utils::problem_details::ProblemDetails;

use super::*;

impl<DLS, P> AdminReplay<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    /// Returns the Axum router configured with replay routes
    pub fn router(self) -> Router {
        let admin_replay = Arc::new(self);

        Router::new()
            .route(
                "/admin/dead-letters/replay/:event_id",
                patch({
                    let admin_replay = admin_replay.clone();
                    move |Path(event_id): Path<Uuid>| async move {
                        replay_one_handler(admin_replay, event_id).await
                    }
                }),
            )
            .route(
                "/admin/dead-letters/replay-all",
                post({
                    let admin_replay = admin_replay.clone();
                    move || async move { replay_all_handler(admin_replay).await }
                }),
            )
    }
}

async fn replay_one_handler<DLS, P>(
    admin_replay: Arc<AdminReplay<DLS, P>>,
    event_id: Uuid,
) -> Result<Json<ReplaySummary>, ProblemDetails>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    let summary = admin_replay.replay_one(event_id).await?;

    Ok(Json(summary))
}

async fn replay_all_handler<DLS, P>(
    admin_replay: Arc<AdminReplay<DLS, P>>,
) -> Result<Json<ReplaySummary>, ProblemDetails>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    let summary = admin_replay.replay_all().await?;

    Ok(Json(summary))
}

impl From<AdminReplayError> for ProblemDetails {
    fn from(error: AdminReplayError) -> Self {
        match error {
            AdminReplayError::NotFound => {
                ProblemDetails::not_found("No dead letter events found".to_string())
            },
            AdminReplayError::NatsJetstream(e) => {
                ProblemDetails::internal_server_error(format!("NATS JetStream error: {}", e))
            },
            AdminReplayError::DeadLetterStore(e) => {
                ProblemDetails::internal_server_error(format!("Dead Letter Store error: {}", e))
            },
        }
    }
}
