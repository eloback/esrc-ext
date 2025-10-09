use async_nats::{jetstream, HeaderMap, Message, Subject};
use esrc::{nats::NatsEnvelope, project::Project};
use nats_dead_letter::DeadLetterStore;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReplaySummary {
    pub total_events: usize,
    pub successful_replays: usize,
    pub failed_replays: usize,
    pub processed_aggregates: Vec<uuid::Uuid>,
    pub errors: Vec<String>,
}

#[derive(Clone)]
pub struct ReplayDeadLetter<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    dead_letter_store: DLS,
    project: P,
    context: jetstream::Context,
}

impl<DLS, P> ReplayDeadLetter<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    pub fn new(dead_letter_store: DLS, project: P, context: jetstream::Context) -> Self {
        Self {
            dead_letter_store,
            project,
            context,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayDeadLetterError {
    #[error("No dead letter events found")]
    NotFound,
    #[error(transparent)]
    NatsJetstream(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    DeadLetterStore(Box<dyn std::error::Error + Send + Sync>),
}

impl<DLS, P> ReplayDeadLetter<DLS, P>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
{
    /// Replay all the dead letters events for a given aggregate ID
    pub async fn replay_one(
        &self,
        aggregate_id: uuid::Uuid,
    ) -> Result<ReplaySummary, ReplayDeadLetterError> {
        // TODO: Add a method on the nats-dead-letter crate to get by aggregate ID
        // Get all the events from the dead letter store, we'll filter later
        let events = self
            .dead_letter_store
            .get_dead_letters(None, None, None, None)
            .await
            .map_err(|e| ReplayDeadLetterError::DeadLetterStore(e.into()))?;

        let aggregate_events = events
            .into_iter()
            .filter(|e| e.aggregate_id == Some(aggregate_id))
            .collect::<Vec<_>>();

        if aggregate_events.is_empty() {
            return Err(ReplayDeadLetterError::NotFound);
        }

        let mut summary = ReplaySummary {
            total_events: aggregate_events.len(),
            successful_replays: 0,
            failed_replays: 0,
            processed_aggregates: vec![aggregate_id],
            errors: Vec::new(),
        };

        for event in aggregate_events {
            let prefix = event.prefix.ok_or(ReplayDeadLetterError::NatsJetstream(
                "Event prefix is missing".into(),
            ))?;
            let subject = Subject::from(event.subject.clone());
            let mut headers = HeaderMap::new();
            for (key, value) in
                event
                    .headers
                    .clone()
                    .ok_or(ReplayDeadLetterError::NatsJetstream(
                        "Event headers are missing".into(),
                    ))?
            {
                headers.insert(key, value);
            }
            let reply_subject = Subject::from(format!(
                "$JS.ACK._._.{}.{}.{}.{}.1.{}.0.replay",
                event.stream,                           // stream name
                event.consumer,                         // consumer name
                event.delivery_count,                   // delivered count
                event.stream_sequence,                  // stream sequence
                event.timestamp.unix_timestamp_nanos()  // timestamp in nanoseconds
            ));

            // Create a NATS message from the dead letter event
            let nats_core_message = Message {
                subject: subject.clone(),
                reply: Some(reply_subject),
                payload: event.payload.clone().into(),
                headers: Some(headers),
                status: None,
                description: None,
                length: event.payload.len(),
            };

            let jetstream_message = jetstream::Message {
                message: nats_core_message,
                context: self.context.clone(),
            };

            // Create an escr envelope
            let envelope =
                NatsEnvelope::try_from_message(&prefix, jetstream_message).map_err(|e| {
                    ReplayDeadLetterError::NatsJetstream(
                        format!("Failed to create envelope: {}", e).into(),
                    )
                })?;

            // From the envelope, convert it to a context
            let context = esrc::project::Context::try_with_envelope(&envelope).map_err(|e| {
                ReplayDeadLetterError::NatsJetstream(
                    format!("Failed to create context: {}", e).into(),
                )
            })?;

            let mut project = self.project.clone();
            match project.project(context).await {
                Ok(_) => {
                    summary.successful_replays += 1;
                    if let Some(id) = event.id
                        && let Err(e) = self
                            .dead_letter_store
                            .remove_dead_letter(&id.to_string())
                            .await
                    {
                        summary
                            .errors
                            .push(format!("Failed to remove dead letter {}: {}", id, e));
                    }
                },
                Err(e) => {
                    summary.failed_replays += 1;
                    summary
                        .errors
                        .push(format!("Failed to replay event: {}", e));
                },
            }
        }

        Ok(summary)
    }

    /// Replay all the dead letters events from all aggregates
    pub async fn replay_all(&self) -> Result<ReplaySummary, ReplayDeadLetterError> {
        // Get all the events from the dead letter store
        let events = self
            .dead_letter_store
            .get_dead_letters(None, None, None, None)
            .await
            .map_err(|e| ReplayDeadLetterError::DeadLetterStore(e.into()))?;

        // Filter based on aggregates
        let mut aggregates_events = std::collections::HashMap::new();
        for event in events {
            if let Some(aggregate_id) = event.aggregate_id {
                aggregates_events
                    .entry(aggregate_id)
                    .or_insert_with(Vec::new)
                    .push(event);
            }
        }

        if aggregates_events.is_empty() {
            return Err(ReplayDeadLetterError::NotFound);
        }

        let mut summary = ReplaySummary {
            total_events: 0,
            successful_replays: 0,
            failed_replays: 0,
            processed_aggregates: aggregates_events.keys().cloned().collect(),
            errors: Vec::new(),
        };

        // Replay events for each aggregate
        for (aggregate_id, events) in aggregates_events {
            println!("Replaying events for aggregate ID: {}", aggregate_id);
            summary.total_events += events.len();

            for event in events {
                let prefix = event.prefix.ok_or("Event prefix is missing").map_err(|e| {
                    ReplayDeadLetterError::NatsJetstream(
                        format!("Event prefix is missing: {}", e).into(),
                    )
                })?;
                let subject = Subject::from(event.subject);
                let mut headers = HeaderMap::new();
                for (key, value) in event.headers.clone().expect("Headers should be present") {
                    headers.insert(key, value);
                }
                let reply_subject = Subject::from(format!(
                    "$JS.ACK._._.{}.{}.{}.{}.1.{}.0.replay",
                    event.stream,                           // stream name
                    event.consumer,                         // consumer name
                    event.delivery_count,                   // delivered count
                    event.stream_sequence,                  // stream sequence
                    event.timestamp.unix_timestamp_nanos()  // timestamp in nanoseconds
                ));

                // Create a NATS message from the dead letter event
                let nats_core_message = Message {
                    subject: subject.clone(),
                    reply: Some(reply_subject),
                    payload: event.payload.clone().into(),
                    headers: Some(headers),
                    status: None,
                    description: None,
                    length: event.payload.len(),
                };
                let jetstream_message = jetstream::Message {
                    message: nats_core_message,
                    context: self.context.clone(),
                };

                // Create an escr envelope
                let envelope =
                    NatsEnvelope::try_from_message(&prefix, jetstream_message).map_err(|e| {
                        ReplayDeadLetterError::NatsJetstream(
                            format!("Failed to create envelope: {}", e).into(),
                        )
                    })?;

                // From the envelope, convert it to a context
                let context =
                    esrc::project::Context::try_with_envelope(&envelope).map_err(|e| {
                        ReplayDeadLetterError::NatsJetstream(
                            format!("Failed to create context: {}", e).into(),
                        )
                    })?;

                let mut project = self.project.clone();
                match project.project(context).await {
                    Ok(_) => {
                        summary.successful_replays += 1;
                        if let Some(id) = event.id
                            && let Err(e) = self
                                .dead_letter_store
                                .remove_dead_letter(&id.to_string())
                                .await
                        {
                            summary
                                .errors
                                .push(format!("Failed to remove dead letter {}: {}", id, e));
                        }
                    },
                    Err(e) => {
                        summary.failed_replays += 1;
                        summary.errors.push(format!(
                            "Failed to replay event for aggregate {}: {}",
                            aggregate_id, e
                        ));
                    },
                }
            }
        }

        Ok(summary)
    }
}
