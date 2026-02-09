use std::sync::{Arc, Mutex};

use async_nats::jetstream::{consumer::Consumer, Context};
use futures::{Stream, StreamExt, TryStreamExt};
use stream_cancel::Trigger;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::task::TaskTracker;
use tracing::instrument;

use super::*;

/// A handle to an event store implementation on top of NATS.
///
/// This type implements the needed traits for reading and writing events from
/// various event streams, encoded as durable messages in a Jetstream instance.
#[derive(Clone)]
pub struct ExternalStore {
    stream: JetStream,

    graceful_shutdown: GracefulShutdown,
}

/// A structure to help with graceful shutdown of tasks.
#[derive(Clone)]
pub struct GracefulShutdown {
    task_tracker: TaskTracker,
    exit_rx: Arc<Mutex<Receiver<Trigger>>>,
    exit_tx: Sender<Trigger>,
}

impl ExternalStore {
    /// Create a new instance of a NATS event store.
    ///
    /// This uses an existing Jetstream context and a global prefix string. The
    /// method will attempt to use an existing stream with this name, or create
    /// a new one with default settings. All esrc event streams are created with
    /// this prefix, using the format `<prefix>.<event_name>`.
    #[instrument(skip_all, level = "debug")]
    pub async fn try_new(
        context: Context,
        stream_config: &StreamConfig,
    ) -> esrc::error::Result<Self> {
        let stream = context.get_or_create_stream(stream_config.clone()).await?;

        // if there is more than 1000 automations this should be increased
        let (exit_tx, exit_rx) = tokio::sync::mpsc::channel::<stream_cancel::Trigger>(1000);
        let task_tracker = tokio_util::task::TaskTracker::new();

        let graceful_shutdown = GracefulShutdown {
            exit_tx,
            exit_rx: Mutex::new(exit_rx).into(),
            task_tracker,
        };

        Ok(Self {
            stream,

            graceful_shutdown,
        })
    }

    /// get a handle to the task tracker used for graceful shutdown of tasks
    pub fn get_task_tracker(&self) -> TaskTracker {
        self.graceful_shutdown.task_tracker.clone()
    }

    /// wait for all tasks to shutdown gracefully
    /// This method will trigger all tasks registered for graceful shutdown
    /// and wait for them to finish.
    pub async fn wait_graceful_shutdown(self) {
        {
            let mut exit_rx = self
                .graceful_shutdown
                .exit_rx
                .lock()
                .expect("lock to not be poisoned");
            while let Ok(trigger) = exit_rx.try_recv() {
                println!("triggering graceful shutdown");
                trigger.cancel();
            }
        }

        self.graceful_shutdown.task_tracker.close();
        self.graceful_shutdown.task_tracker.wait().await;
    }

    #[instrument(skip_all, level = "debug")]
    async fn durable_consumer(
        &self,
        config: ConsumerConfig,
    ) -> esrc::error::Result<Consumer<ConsumerConfig>> {
        if config.durable_name.is_none() {
            return Err(esrc::error::Error::Format(
                "durable_name must be set in consumer config".into(),
            ));
        }
        if config.filter_subjects.is_empty() {
            return Err(esrc::error::Error::Format(
                "filter_subjects must be set in consumer config".into(),
            ));
        }

        Ok(self.stream.create_consumer(config).await?)
    }

    #[instrument(skip_all, level = "debug")]
    async fn subscribe(
        &self,
        config: ConsumerConfig,
    ) -> esrc::error::Result<impl Stream<Item = esrc::error::Result<Message>> + Send> {
        let consumer = self.durable_consumer(config).await?;
        let messages = consumer
            .messages()
            .await?
            .map_err(|e| esrc::error::Error::Format(e.into()));
        Ok(messages)
    }

    /// subscribe to the given subjects, and process incoming messages with the given projector.
    #[instrument(skip_all, level = "debug")]
    pub async fn run_project<P>(
        &self,
        projector: P,
        max_concurrency: impl Into<Option<usize>> + Send,
    ) -> esrc::error::Result<()>
    where
        P: TranslationProject + 'static,
    {
        let config = projector.consumer_config();

        let stream = std::pin::pin!(self.subscribe(config).await?);
        let (exit, incoming) = Valved::new(stream);
        self.graceful_shutdown
            .exit_tx
            .clone()
            .send(exit)
            .await
            .expect("should be able to send graceful trigger");

        // Configure throughput (concurrent workers)
        incoming
            .for_each_concurrent(max_concurrency, |message| {
                let projector = projector.clone();

                async move {
                    if let Err(e) = process_message(&projector, message).await {
                        tracing::error!("Failed to process message: {:?}", e);
                    }
                }
            })
            .await;

        Ok(())
    }
}
