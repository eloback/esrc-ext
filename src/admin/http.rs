use axum::{
    extract::{FromRef, Path, State},
    routing::{patch, post},
    Json, Router,
};
use discern::command::CommandBus;
use esrc::project::Project;
use nats_dead_letter::DeadLetterStore;
use uuid::Uuid;

use crate::{
    admin::{
        replay_dead_letter::{ReplayDeadLetterError, ReplaySummary},
        AdminCommands, AdminCommandsError, AdminHandler,
    },
    utils::problem_details::ProblemDetails,
};

#[derive(Clone)]
pub struct AdminAppState {
    pub command_bus: CommandBus,
}

pub trait HasAdminAppState {
    fn admin_state(&self) -> &AdminAppState;
}

// Implement FromRef for AdminAppState to work as a substate
impl<S> FromRef<S> for AdminAppState
where
    S: HasAdminAppState + Clone,
{
    fn from_ref(state: &S) -> Self {
        state.admin_state().clone()
    }
}

impl<DLS, P> AdminHandler<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    /// Define all the common admin routes
    pub fn setup_router<S>(self, router: &mut Router<S>)
    where
        S: HasAdminAppState + Clone + Send + Sync + 'static,
    {
        let new_router = std::mem::take(router)
            .route(
                "/admin/dead-letters/replay/{event_id}",
                patch(replay_one_handler::<S>),
            )
            .route(
                "/admin/dead-letters/replay-all",
                post(replay_all_handler::<S>),
            );

        *router = new_router;
    }
}

pub async fn replay_one_handler<S>(
    State(admin_app_state): State<AdminAppState>,
    Path(aggregate_id): Path<Uuid>,
) -> Result<Json<ReplaySummary>, ProblemDetails>
where
    S: HasAdminAppState,
{
    let command = AdminCommands::ReplayOneDeadLetter { aggregate_id };

    let summary = admin_app_state.command_bus.dispatch(command).await?;

    Ok(Json(summary))
}

pub async fn replay_all_handler<S>(
    State(admin_app_state): State<AdminAppState>,
) -> Result<Json<ReplaySummary>, ProblemDetails>
where
    S: HasAdminAppState,
{
    let command = AdminCommands::ReplayAllDeadLetter {};

    let summary = admin_app_state.command_bus.dispatch(command).await?;

    Ok(Json(summary))
}

impl From<AdminCommandsError> for ProblemDetails {
    fn from(error: AdminCommandsError) -> Self {
        match error {
            AdminCommandsError::ReplayDeadLetterError(e) => match e {
                ReplayDeadLetterError::NotFound => {
                    ProblemDetails::not_found("No dead letter events found".to_string())
                },
                ReplayDeadLetterError::NatsJetstream(e) => {
                    ProblemDetails::internal_server_error(format!("NATS JetStream error: {}", e))
                },
                ReplayDeadLetterError::DeadLetterStore(e) => {
                    ProblemDetails::internal_server_error(format!("Dead Letter Store error: {}", e))
                },
            },
        }
    }
}
