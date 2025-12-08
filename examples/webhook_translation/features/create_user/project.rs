use esrc_ext::translation::TranslationProject;

use super::*;

#[derive(Debug, thiserror::Error)]
pub enum UserErrors {
    #[error(transparent)]
    EventStore(#[from] esrc::Error),
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
}
