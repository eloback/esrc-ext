use esrc::prelude::async_nats;
use esrc::prelude::async_nats::jetstream::Message;
use serde::Deserialize;
use stream_cancel::Valved;
use tracing::instrument;

pub mod external_nats;
pub mod project;

// Re-exports
pub use external_nats::*;
pub use project::*;

// Type Aliases
type JetStream = async_nats::jetstream::stream::Stream;
type StreamConfig = async_nats::jetstream::stream::Config;
type ConsumerConfig = async_nats::jetstream::consumer::pull::Config;
