use async_nats::jetstream::{self};
use esrc_ext::feature::Feature;

pub mod common;
pub mod features;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

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
    let consumer = async_nats::jetstream::consumer::pull::Config {
        deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::All,
        ..Default::default()
    };

    let external_stream = async_nats::jetstream::stream::Config {
        name: "external".to_string(),
        subjects: vec!["external.webhook_create_user".to_string()],
        retention: async_nats::jetstream::stream::RetentionPolicy::WorkQueue,
        allow_direct: true,
        ..Default::default()
    };

    let external_store =
        esrc_ext::translation::ExternalStore::try_new(context.clone(), &external_stream, &consumer)
            .await?;

    let mut router = axum::Router::new();

    // * Dependencies
    // Create CommandBus and attach handlers
    let feature = Feature::new(&event_store);

    // * Translations Project
    {
        // User translation setup
        let user_project = features::create_user::setup(&mut router);
        feature.start_translation(&external_store, user_project.clone(), "create_user");
    }

    // * Start Application
    let app_state = common::AppState {
        nats_client: client.clone(),
    };

    // Available endpoints:
    // POST /webhook/create_user - to receive webhook requests for user creation
    tracing::info!("Starting HTTP server on port 3001");
    tracing::info!("Available endpoints:");
    tracing::info!("  POST http://localhost:3001/webhook/create_user");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, router.with_state(app_state))
            .await
            .expect("Server should start successfully");
    });

    // Spawn a task to handle CTRL+C for graceful shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutdown signal received...");
        }
        _ = server_handle => {
            println!("Server task completed");
        }
    }

    // Wait for all automations to exit gracefully
    tracing::warn!("Waiting for graceful shutdown...");
    event_store.wait_graceful_shutdown().await;

    tracing::warn!("Application shut down successfully");
    Ok(())
}
