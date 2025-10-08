use async_nats::jetstream::{self};
use escr_ext::{feature::Feature, replay::AdminReplay};
use esrc::{
    aggregate::Root,
    event::{PublishExt, ReplayOneExt},
};
use nats_dead_letter::postgres::SqlxDeadLetterStore;
use sqlx::postgres::PgPoolOptions;

use crate::{features::create_user::command::User, read_models::users::project::UserProject};

pub mod domain;
pub mod features;
pub mod read_models;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("Starting application server");

    // Database setup
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    // NATS connection setup
    let nats_url = std::env::var("NATS_URL").expect("NATS_URL must be set");
    let client = async_nats::connect(&nats_url).await?;
    let context = jetstream::new(client.clone());

    // Initialize NatsStore for dead letter management
    let admin_store = nats_dead_letter::NatsStore::try_new(context.clone(), "dead_letter").await?;
    let dead_letter_store = SqlxDeadLetterStore::new(db_pool.clone());

    println!("Dead letter automation started");
    dead_letter_store.migrate().await?;
    {
        let store = admin_store.clone();

        // Start dead letter automation to handle failed messages
        store.get_task_tracker().spawn(async move {
            // Start dead letter automation for a specific stream and consumer
            store
                .run_dead_letter_automation(dead_letter_store)
                .await
                .expect("dead letter automation should be able to start");
        });
    }

    // Initialize the features and event store for your actual application
    let mut event_store = esrc::nats::NatsStore::try_new(context.clone(), "users")
        .await?
        .update_durable_consumer_option(jetstream::consumer::pull::Config {
            backoff: vec![std::time::Duration::from_secs(2)],
            max_deliver: 2,
            deliver_policy: jetstream::consumer::DeliverPolicy::New,

            ..Default::default()
        });

    let feature = Feature::new(&event_store);

    // User feature setup
    let user_project = UserProject::new(db_pool.clone()).await;
    feature.start_automation(user_project.clone(), "user_creation");

    // Set up the AdminReplay for handling replay requests
    let replay_store = SqlxDeadLetterStore::new(db_pool.clone());

    let admin_replay =
        AdminReplay::<SqlxDeadLetterStore, UserProject>::new(replay_store, user_project, context);

    // Create Axum router with replay endpoints
    let app = admin_replay.router();

    // Start the HTTP server for replay management
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await?;
    println!("Replay API server started on http://0.0.0.0:3001");

    // Available endpoints:
    // PATCH /admin/dead-letters/replay/:event_id - Replay a specific event by its aggregate ID
    // POST /admin/dead-letters/replay-all - Replay all dead letter events
    println!("Available endpoints:");
    println!("  PATCH http://localhost:3001/admin/dead-letters/replay/<aggregate-id>");
    println!("  POST  http://localhost:3001/admin/dead-letters/replay-all");

    // Spawn the server in a background task
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Server should start successfully");
    });

    // Keep the application running
    println!("\nApplication is running. Press Ctrl+C to stop.");
    println!("You can use the replay endpoints to manage dead letter events.\n");

    // Execute commands for example purposes - This will create correctly
    let user_id = uuid::Uuid::now_v7();
    let root: Root<User> = event_store.read(user_id).await?;
    let create_user_command = features::create_user::CreateUser {
        user_id,
        name: "John Doe".to_string(),
        email: "john.doe@example.com".to_string(),
    };
    event_store.try_write(root, create_user_command).await?;

    // This command will fail and be sent to the dead letter queue
    tokio::spawn(async move {
        // Created a tokio::spawn to not panic in the main thread
        let user_id = uuid::Uuid::now_v7();
        let root: Root<User> = event_store.read(user_id).await.expect("Should read root");
        let create_user_command = features::create_user::CreateUser {
            user_id: uuid::Uuid::now_v7(),
            name: "comercial".to_string(),
            email: "comercial@example.com".to_string(),
        };
        match event_store.try_write(root, create_user_command).await {
            Ok(_) => println!("User created successfully"),
            Err(e) => println!("Failed to create user: {}", e),
        }
    });

    // In a production system, you would wait for shutdown signals
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nShutdown signal received...");
        }
        _ = server_handle => {
            println!("Server task completed");
        }
    }

    // Wait for all automations to exit gracefully
    println!("Waiting for graceful shutdown...");
    admin_store.wait_graceful_shutdown().await;

    println!("Application shut down successfully");
    Ok(())
}
