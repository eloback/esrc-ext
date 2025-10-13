use esrc::{
    event::event_model::view::View,
    project::{Context, Project},
    Envelope,
};
use serde_json::Value;
use sqlx::Row;
use std::marker::PhantomData;
use uuid::Uuid;

/// Postgres view esrc::Project
#[derive(Clone)]
pub struct PgViewProjector<V: View> {
    view: PhantomData<V>,
    name: String,
    db: sqlx::PgPool,
}

impl<V: View> PgViewProjector<V> {
    pub fn new(name: String, db: sqlx::PgPool) -> Self {
        Self {
            view: PhantomData,
            name,
            db,
        }
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.db
    }

    pub async fn setup(self) -> Result<()> {
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {}(
                    view_id uuid                        NOT NULL,
                    payload jsonb                       NOT NULL,
                    PRIMARY KEY (view_id)
                );",
            self.name
        ))
        .execute(&self.db)
        .await?;
        Ok(())
    }

    pub async fn load(&self, id: Uuid) -> Result<V> {
        let row = sqlx::query(&format!(
            "select payload from {} where view_id = $1",
            self.name
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?;
        Ok({
            if let Some(row) = row {
                let data = row.get::<Value, _>("payload");
                serde_json::from_value(data).expect("Failed to deserialize payload")
            } else {
                V::default()
            }
        })
    }

    pub async fn save(&self, id: Uuid, view: &V) -> Result<()> {
        sqlx::query(&format!(
                "INSERT INTO {} (view_id, payload) values ($1, $2) ON CONFLICT (view_id) DO UPDATE SET payload = EXCLUDED.payload", self.name
            ))
            .bind(id)
            .bind(serde_json::to_value(view).expect("view should be serializable"))
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn delete(&self) -> Result<()> {
        sqlx::query(&format!("delete from {}", self.name))
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn delete_one(&self, id: Uuid) -> Result<()> {
        sqlx::query(&format!("delete from {} where view_id = $1", self.name))
            .bind(id)
            .execute(&self.db)
            .await?;
        Ok(())
    }

    pub async fn replay<'de, E>(
        &self,
        id: Uuid,
        events: Vec<Context<'de, E, V::EventGroup>>,
    ) -> Result<()>
    where
        E: Envelope,
    {
        self.delete_one(id).await?;

        let mut rm = V::default();

        for event in events {
            rm.apply(event);
        }

        self.save(id, &rm).await?;

        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum PgViewProjectorError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, PgViewProjectorError>;

impl<V: View + Sync + Send> Project for PgViewProjector<V> {
    type EventGroup = V::EventGroup;
    type Error = PgViewProjectorError;

    #[tracing::instrument(name = "::view", skip_all, fields(view_id=tracing::field::Empty, read_mode_name=self.name), ret, err(Debug))]
    async fn project<'de, E: Envelope>(
        &mut self,
        context: Context<'de, E, Self::EventGroup>,
    ) -> Result<()> {
        let id = &Context::id(&context);
        tracing::Span::current().record("view_id", id.to_string());
        let mut rm: V = self.load(*id).await.unwrap();
        let changed = rm.apply(context);
        if !changed {
            tracing::debug!("view not changed, skipping save");
            return Ok(());
        }
        self.save(*id, &rm).await.unwrap();
        Ok(())
    }
}
