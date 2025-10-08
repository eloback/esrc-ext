use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod project;

/// Command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUser {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
}
