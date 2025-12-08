use super::*;

/// A data model that can be "projected" onto.
///
/// That is, receive events for all aggregate IDs for an event or events, and
/// process them to trigger a side effect or construct a read model. The exact
/// purpose is implementation specific; the trait only handles receiving the
/// Envelopes for the specified events.
#[trait_variant::make(Send)]
pub trait TranslationProject: Send + Clone {
    /// The message(s) that can be processed by this object.
    type MessageGroup: for<'a> Deserialize<'a> + Send;
    /// The type to return as an `Err` when the projection fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Apply a received message, triggering implementation specific behavior.
    ///
    /// Returning an error from this method should stop further messages from
    /// being processed in the associated event store.
    async fn project(
        &mut self,
        event: Self::MessageGroup,
        metadata: Option<serde_json::Value>,
    ) -> Result<(), Self::Error>;
}

/// recieves a message, processes it with the given projector, and acknowledges it.
#[instrument(skip_all, name = "translation", level = "info", err(Debug))]
pub async fn process_message<P: TranslationProject>(
    projector: &mut P,
    message: Result<Message, esrc::error::Error>,
) -> esrc::error::Result<()> {
    let envelope = message?;
    // propagate otel span if exists
    opentelemetry_nats::attach_span_context(&envelope);

    // extract headers and event
    let headers = envelope
        .headers
        .as_ref()
        .and_then(|v| serde_json::to_value(v).ok());
    let event: P::MessageGroup = serde_json::from_slice(&envelope.payload).unwrap();

    projector
        .project(event, headers)
        .await
        .map_err(|e| esrc::error::Error::External(e.into()))?;
    let _ = envelope.ack().await;
    Ok(())
}
