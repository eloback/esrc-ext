use axum::Router;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod http;
pub mod project;

// * Re-exports
pub use project::*;

use crate::common::AppState;

#[derive(Debug, Serialize, Deserialize)]
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

pub fn setup(router: &mut Router<AppState>) -> UserProject {
    let project = UserProject;

    // Since router does not implement Clone, we take ownership and replace it
    let new_router = std::mem::take(router).route(
        "/webhook/create_user",
        axum::routing::post(http::controller),
    );
    *router = new_router;

    project
}
