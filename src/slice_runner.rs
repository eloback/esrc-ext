use esrc::prelude::*;

use crate::translation::ExternalStore;

pub const DEFAULT_READ_MODEL_PROJECTOR_VERSION: u32 = DEFAULT_VIEW_PROJECTOR_VERSION;

pub fn start_automation<A>(
    store: &NatsStore,
    project: A,
    feature_name: &'static str,
    max_concurrency: impl Into<Option<usize>> + Send + 'static,
) where
    A: Project + 'static,
{
    let store = store.clone();
    store.get_task_tracker().spawn(async move {
        store
            .start_automation(project, feature_name, max_concurrency)
            .await
            .expect("automation should be able to start");
    });
}

pub fn start_read_model_automation<A>(store: &NatsStore, project: A, feature_name: &'static str)
where
    A: Project + 'static,
{
    start_read_model_automation_with_version(
        store,
        project,
        feature_name,
        DEFAULT_READ_MODEL_PROJECTOR_VERSION,
    );
}

pub fn start_read_model_automation_with_version<A>(
    store: &NatsStore,
    project: A,
    feature_name: &'static str,
    projector_version: u32,
) where
    A: Project + 'static,
{
    let store = store.clone();
    store.get_task_tracker().spawn(async move {
        if let Err(error) = store
            .start_view_automation_with_identity(
                project,
                feature_name,
                ViewProjectorIdentity::new(feature_name, projector_version),
            )
            .await
        {
            tracing::error!(
                feature_name,
                projector_version,
                error = ?error,
                "read-model automation stopped"
            );
        }
    });
}

pub fn start_dead_letter_automation<A>(
    store: &NatsStore,
    durable_name: &'static str,
    stream_name: &'static str,
    feature_name: &'static str,
    dead_letter_store: A,
) where
    A: esrc::nats::DeadLetterStore + Clone + 'static,
{
    let store = store.clone();
    let dead_letter_store = dead_letter_store.clone();

    store.get_task_tracker().spawn(async move {
        // Start dead letter automation for a specific stream and consumer
        store
            .run_dead_letter_automation(dead_letter_store, durable_name, stream_name, feature_name)
            .await
            .expect("dead letter automation should be able to start");
    });
}

pub fn start_translation<A>(
    external_store: &ExternalStore,
    project: A,
    max_concurrency: impl Into<Option<usize>> + Send + 'static,
) where
    A: crate::translation::TranslationProject + 'static,
{
    let external_store = external_store.clone();
    external_store.get_task_tracker().spawn(async move {
        external_store
            .run_project(project, max_concurrency)
            .await
            .expect("translation should be able to start");
    });
}
