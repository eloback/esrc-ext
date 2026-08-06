use esrc::prelude::*;
use esrc_ext::translation::TranslationProject;

use super::*;

#[derive(Debug, thiserror::Error)]
pub enum UserErrors {
    #[error(transparent)]
    EventStore(#[from] Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

// Example project implementation
#[derive(Clone)]
pub struct UserProject;

impl TranslationProject for UserProject {
    type MessageGroup = ExternalEvents;
    type Error = UserErrors;

    async fn project(
        &mut self,
        event: Self::MessageGroup,
        metadata: Option<serde_json::Value>,
    ) -> Result<(), Self::Error> {
        println!("Processing event: {:?}", event);
        println!("With metadata: {:?}", metadata);

        Ok(())
    }

    fn consumer_config(&self) -> async_nats::jetstream::consumer::pull::Config {
        async_nats::jetstream::consumer::pull::Config {
            durable_name: Some("webhook_create_user".to_string()),
            filter_subjects: vec!["external.webhook_create_user".to_string()],
            deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::All,
            ..Default::default()
        }
    }
}
