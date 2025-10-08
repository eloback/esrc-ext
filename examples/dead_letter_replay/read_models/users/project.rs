use esrc::{
    project::{Context, Project},
    Envelope,
};
use sqlx::PgPool;

use crate::domain::user::Events;

#[derive(Debug, thiserror::Error)]
pub enum UserErrors {
    #[error(transparent)]
    EventStore(#[from] esrc::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

// Example project implementation
#[derive(Clone)]
pub struct UserProject {
    db_pool: PgPool,
}

impl UserProject {
    pub async fn new(db_pool: PgPool) -> Self {
        let _ = sqlx::query(
            "CREATE TABLE IF NOT EXISTS users(
                id uuid                        NOT NULL,
                name text                       NOT NULL,
                email text                      NOT NULL,
                created_at timestamptz DEFAULT NOW(),
                updated_at timestamptz DEFAULT NOW(),
                PRIMARY KEY (id)
            );",
        )
        .execute(&db_pool)
        .await
        .expect("Failed to create users table");

        Self { db_pool }
    }
}

impl Project for UserProject {
    type EventGroup = Events;
    type Error = UserErrors;

    async fn project<'de, E: Envelope>(
        &mut self,
        context: Context<'de, E, Self::EventGroup>,
    ) -> Result<(), Self::Error> {
        println!("Processing event: {:?}", &Context::id(&context));

        match context.clone() {
            Self::EventGroup::UserCreated {
                user_id,
                name,
                email,
            } => {
                if name == "comercial" {
                    panic!("Simulated failure for testing dead letter handling");
                }

                // Example: Insert user into database
                sqlx::query(
                    "INSERT INTO users (id, name, email) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING"
                ).bind(user_id)
                .bind(name)
                .bind(email)
                .execute(&self.db_pool)
                .await?;
            },
            Self::EventGroup::UserUpdated {
                user_id,
                name,
                email,
            } => {
                // Example: Update user in database
                if let Some(name) = &name {
                    let _ = sqlx::query("UPDATE users SET name = $1 WHERE id = $2")
                        .bind(name)
                        .bind(user_id)
                        .execute(&self.db_pool)
                        .await
                        .map_err(|e| format!("Failed to update user name: {}", e));
                }

                if let Some(email) = &email {
                    sqlx::query("UPDATE users SET email = $1 WHERE id = $2")
                        .bind(email)
                        .bind(user_id)
                        .execute(&self.db_pool)
                        .await?;
                }
            },
        }

        Ok(())
    }
}
