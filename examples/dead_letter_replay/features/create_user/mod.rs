use uuid::Uuid;

pub mod command;

pub struct CreateUser {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Errors {
    #[error("User already exists")]
    UserAlreadyExists,
}
