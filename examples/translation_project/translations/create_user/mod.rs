use serde::Deserialize;
use uuid::Uuid;

pub mod project;

// * Re-exports
pub use project::*;

#[derive(Debug, Deserialize)]
#[serde(tag = "_type", content = "content")]
pub enum ExternalEvents {
    UserCreated {
        user_id: Uuid,
        name: String,
        email: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Errors {
    #[error("User already exists")]
    UserAlreadyExists,
}
