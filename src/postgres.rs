use esrc::prelude::*;
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction};
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

    /// Create the view table and add projection checkpoints to an existing table.
    ///
    /// Existing rows receive a null checkpoint. Run this migration while projectors are stopped so
    /// a historical redelivery cannot be applied to an already-materialized legacy row.
    pub async fn setup(self) -> Result<()> {
        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        sqlx::query(&format!(
            "CREATE TABLE IF NOT EXISTS {}(
                    view_id uuid                        NOT NULL,
                    payload jsonb                       NOT NULL,
                    stream_sequence bigint,
                    PRIMARY KEY (view_id)
                );",
            self.name
        ))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(&format!(
            "ALTER TABLE {} ADD COLUMN IF NOT EXISTS stream_sequence bigint",
            self.name
        ))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn load(&self, id: Uuid) -> Result<Option<V>> {
        sqlx::query(&format!(
            "select payload from {} where view_id = $1",
            self.name
        ))
        .bind(id)
        .fetch_optional(&self.db)
        .await?
        .map(|e| e.get::<Value, _>("payload"))
        .map(|e| serde_json::from_value(e))
        .transpose()
        .map_err(PgViewProjectorError::from)
    }

    pub async fn save(&self, id: Uuid, view: &V) -> Result<()> {
        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        self.save_in_transaction(&mut transaction, id, view).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn delete(&self) -> Result<()> {
        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        sqlx::query(&format!("delete from {}", self.name))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Delete a view together with its projection checkpoint.
    ///
    /// A pending or redelivered event can recreate the row. Stop or otherwise coordinate the
    /// durable consumer when deletion is intended to remain permanent.
    pub async fn delete_one(&self, id: Uuid) -> Result<()> {
        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        self.delete_one_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn delete_one_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> Result<()> {
        sqlx::query(&format!("delete from {} where view_id = $1", self.name))
            .bind(id)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    async fn lock(&self, transaction: &mut Transaction<'_, Postgres>) -> Result<()> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(&self.name)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    async fn load_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        id: Uuid,
    ) -> Result<Option<(V, Option<i64>)>> {
        sqlx::query(&format!(
            "select payload, stream_sequence from {} where view_id = $1 for update",
            self.name
        ))
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await?
        .map(|row| {
            serde_json::from_value(row.get::<Value, _>("payload"))
                .map(|view| (view, row.get::<Option<i64>, _>("stream_sequence")))
        })
        .transpose()
        .map_err(PgViewProjectorError::from)
    }

    async fn save_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        id: Uuid,
        view: &V,
    ) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO {} (view_id, payload) values ($1, $2) ON CONFLICT (view_id) DO UPDATE SET payload = EXCLUDED.payload",
            self.name
        ))
        .bind(id)
        .bind(serde_json::to_value(view)?)
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    async fn save_projected_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        id: Uuid,
        view: &V,
        sequence: Option<i64>,
    ) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO {} (view_id, payload, stream_sequence) values ($1, $2, $3) ON CONFLICT (view_id) DO UPDATE SET payload = EXCLUDED.payload, stream_sequence = EXCLUDED.stream_sequence",
            self.name
        ))
        .bind(id)
        .bind(serde_json::to_value(view)?)
        .bind(sequence)
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    /// Rebuild one view from an ordered stream of events for the same aggregate.
    ///
    /// Sequence numbers need only be strictly increasing: a view can subscribe to a subset of an
    /// aggregate's event types, so gaps are valid. Coordinate replay with the NATS durable because
    /// rebuilding this row does not move the consumer cursor.
    pub async fn replay<'de, E>(
        &self,
        id: Uuid,
        events: Vec<Context<'de, E, V::EventGroup>>,
    ) -> Result<()>
    where
        E: Envelope,
    {
        let mut previous_sequence = None;
        for event in &events {
            if Context::id(event) != id {
                return Err(PgViewProjectorError::ReplayAggregateMismatch);
            }
            let sequence = sequence_as_i64(Context::sequence(event))?;
            if previous_sequence.is_some_and(|previous| sequence <= previous) {
                return Err(PgViewProjectorError::ReplayOutOfOrder);
            }
            previous_sequence = Some(sequence);
        }

        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        let mut rm = V::default();
        for event in events {
            rm.apply(event);
        }
        self.save_projected_in_transaction(&mut transaction, id, &rm, previous_sequence)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum PgViewProjectorError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("event stream sequence exceeds PostgreSQL bigint range")]
    InvalidSequence,
    #[error("replay contains an event for another aggregate")]
    ReplayAggregateMismatch,
    #[error("replay event stream sequences are not strictly increasing")]
    ReplayOutOfOrder,
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
        let sequence = sequence_as_i64(Context::sequence(&context))?;
        tracing::Span::current().record("view_id", id.to_string());
        let mut transaction = self.db.begin().await?;
        self.lock(&mut transaction).await?;
        let loaded = self.load_in_transaction(&mut transaction, *id).await?;
        if loaded
            .as_ref()
            .and_then(|(_, last_sequence)| *last_sequence)
            .is_some_and(|last_sequence| sequence <= last_sequence)
        {
            transaction.commit().await?;
            tracing::debug!(sequence, "view event already committed, skipping apply");
            return Ok(());
        }

        let mut rm = loaded.map(|(view, _)| view).unwrap_or_default();
        let changed = rm.apply(context);
        if !changed {
            transaction.commit().await?;
            tracing::debug!(sequence, "view not changed, skipping persistence");
            return Ok(());
        }
        self.save_projected_in_transaction(&mut transaction, *id, &rm, Some(sequence))
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn sequence_as_i64(sequence: Sequence) -> Result<i64> {
    i64::try_from(u64::from(sequence)).map_err(|_| PgViewProjectorError::InvalidSequence)
}
