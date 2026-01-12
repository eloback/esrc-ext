use async_nats::jetstream::{self};
use esrc_ext::slice_runner::start_translation;

pub mod translations;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    tracing::info!("Starting application server");

    // * INFRA SETUP

    // NATS connection setup
    let nats_url = std::env::var("NATS_URL").expect("NATS_URL must be set");
    let client = async_nats::connect(&nats_url).await?;
    let context = jetstream::new(client.clone());

    // Initialize the features and event store for your actual application
    let event_store = esrc::nats::NatsStore::try_new(context.clone(), "users")
        .await?
        .update_durable_consumer_option(jetstream::consumer::pull::Config {
            backoff: vec![std::time::Duration::from_secs(2)],
            max_deliver: 2,
            deliver_policy: jetstream::consumer::DeliverPolicy::New,

            ..Default::default()
        });

    // External Store
    let external_stream = async_nats::jetstream::stream::Config {
        name: "external".to_string(),
        subjects: vec!["external.>".to_string()],
        retention: async_nats::jetstream::stream::RetentionPolicy::WorkQueue,
        allow_direct: true,
        ..Default::default()
    };

    let external_store =
        esrc_ext::translation::ExternalStore::try_new(context.clone(), &external_stream).await?;

    // * Translations Project
    // User translation setup
    let user_project = translations::create_user::UserProject::new(event_store.clone()).await;
    start_translation(&external_store, user_project.clone());

    // * Start Application
    // Spawn a task to handle CTRL+C for graceful shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutdown signal received...");
        }
    }

    // Wait for all automations to exit gracefully
    tracing::warn!("Waiting for graceful shutdown...");
    event_store.wait_graceful_shutdown().await;

    tracing::warn!("Application shut down successfully");
    Ok(())
}
