use async_nats::{jetstream, HeaderMap, Message, Subject};
use esrc::{nats::NatsEnvelope, project::Project, version::DeserializeVersion, Event, EventGroup};
use nats_dead_letter::DeadLetterStore;
use std::marker::PhantomData;

pub mod http;

#[derive(Clone)]
pub struct AdminReplay<DLS, P, G>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    dead_letter_store: DLS,
    project: P,
    context: jetstream::Context,
    _phantom: PhantomData<G>,
}

impl<DLS, P, G> AdminReplay<DLS, P, G>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    pub fn new(dead_letter_store: DLS, project: P, context: jetstream::Context) -> Self {
        Self {
            dead_letter_store,
            project,
            context,
            _phantom: PhantomData,
        }
    }
}

impl<DLS, P, G> AdminReplay<DLS, P, G>
where
    DLS: DeadLetterStore + Send + Sync + 'static,
    P: Project + Send + Sync + 'static,
    G: EventGroup + Event + DeserializeVersion + Send + Sync + 'static,
{
    /// Replay all the dead letters events for a given aggregate ID
    async fn replay_one(
        &mut self,
        aggregate_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Add a method on the nats-dead-letter crate to get by aggregate ID
        // Get all the events from the dead letter store, we'll filter later
        let events = self
            .dead_letter_store
            .get_dead_letters(None, None, None, None)
            .await?;

        // FIXME: This must get all the events for a given aggregate ID
        let aggregate_events = events
            .into_iter()
            .filter(|e| e.aggregate_id == Some(aggregate_id))
            .collect::<Vec<_>>();

        if aggregate_events.is_empty() {
            return Err(format!(
                "No dead letter events found for aggregate ID {}",
                aggregate_id
            )
            .into());
        }

        for event in aggregate_events {
            let prefix = event.prefix.ok_or("Event prefix is missing")?;
            let subject = Subject::from(event.subject);
            let mut headers = HeaderMap::new();
            for (key, value) in event.headers.clone().expect("Headers should be present") {
                headers.insert(key, value);
            }

            // Create a NATS message from the dead letter event
            let nats_core_message = Message {
                subject,
                reply: None,
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
            let envelope = NatsEnvelope::try_from_message(&prefix, jetstream_message)?;

            // From the envelope, convert it to a context
            let context = esrc::project::Context::try_with_envelope(&envelope)?;

            self.project.project(context).await?;

            if let Some(id) = event.id {
                self.dead_letter_store
                    .remove_dead_letter(&id.to_string())
                    .await?;
            }
        }

        Ok(())
    }

    /// Replay all the dead letters events from all aggregates
    async fn replay_all(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Get all the events from the dead letter store
        let events = self
            .dead_letter_store
            .get_dead_letters(None, None, None, None)
            .await?;

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

        // Replay events for each aggregate
        for (aggregate_id, events) in aggregates_events {
            println!("Replaying events for aggregate ID: {}", aggregate_id);

            for event in events {
                let prefix = event.prefix.ok_or("Event prefix is missing")?;
                let subject = Subject::from(event.subject);
                let mut headers = HeaderMap::new();
                for (key, value) in event.headers.clone().expect("Headers should be present") {
                    headers.insert(key, value);
                }

                // Create a NATS message from the dead letter event
                let nats_core_message = Message {
                    subject,
                    reply: None,
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
                let envelope = NatsEnvelope::try_from_message(&prefix, jetstream_message)?;

                // From the envelope, convert it to a context
                let context = esrc::project::Context::try_with_envelope(&envelope)?;

                self.project.project(context).await?;

                if let Some(id) = event.id {
                    self.dead_letter_store
                        .remove_dead_letter(&id.to_string())
                        .await?;
                }
            }
        }

        todo!()
    }
}
