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
pub struct UserProject<T>
where
    T: Clone + Publish + ReplayOne + Sync + 'static,
{
    /// Event store that you can publish event to an stream
    _event_store: T,
}

impl<T> UserProject<T>
where
    T: Clone + Publish + ReplayOne + Sync + 'static,
{
    pub async fn new(event_store: T) -> Self {
        Self {
            _event_store: event_store,
        }
    }
}

impl<T> TranslationProject for UserProject<T>
where
    T: Clone + Publish + ReplayOne + Sync + 'static,
{
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
            durable_name: Some("user-project".to_string()),
            filter_subjects: vec!["external.create_user".to_string()],
            deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::All,
            ..Default::default()
        }
    }
}
