use async_nats::jetstream;
use discern::command::Command;
use discern::command::CommandHandler;
use esrc::project::Project;
use nats_dead_letter::DeadLetterStore;
use uuid::Uuid;

use crate::admin::replay_dead_letter::{ReplayDeadLetter, ReplayDeadLetterError, ReplaySummary};

pub mod http;
pub mod replay_dead_letter;

#[derive(Clone)]
pub struct AdminHandler<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    dead_letter_replay: ReplayDeadLetter<DLS, P>,
}

impl<DLS, P> AdminHandler<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    pub fn new(dead_letter_store: DLS, project: P, context: jetstream::Context) -> Self {
        let dead_letter_replay = ReplayDeadLetter::new(dead_letter_store, project, context);

        Self { dead_letter_replay }
    }
}

#[discern::async_trait]
impl<DLS, P> CommandHandler<AdminCommands> for AdminHandler<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    async fn handle(&self, command: AdminCommands) -> Result<ReplaySummary, AdminCommandsError> {
        match command {
            AdminCommands::ReplayOneDeadLetter { aggregate_id } => {
                let summary = self.dead_letter_replay.replay_one(aggregate_id).await?;

                Ok(summary)
            },
            AdminCommands::ReplayAllDeadLetter {} => {
                let summary = self.dead_letter_replay.replay_all().await?;

                Ok(summary)
            },
        }
    }
}

#[derive(Debug)]
pub enum AdminCommands {
    ReplayOneDeadLetter { aggregate_id: Uuid },
    ReplayAllDeadLetter {},
}

#[derive(Debug, thiserror::Error)]
pub enum AdminCommandsError {
    #[error(transparent)]
    ReplayDeadLetterError(#[from] ReplayDeadLetterError),
}

impl Command for AdminCommands {
    type Metadata = ReplaySummary;
    type Error = AdminCommandsError;
}
