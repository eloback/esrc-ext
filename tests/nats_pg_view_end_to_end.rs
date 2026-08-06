use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use esrc::event::event_model::view::Changed;
use esrc::event::event_model::{Translation, ViewAutomation};
use esrc::nats::async_nats;
use esrc::nats::async_nats::jetstream::consumer::pull::Config as ConsumerConfig;
use esrc::nats::NatsStore;
use esrc::prelude::{Context, Envelope, Event, View};
use esrc::version::{DeserializeVersion, SerializeVersion};
use esrc_ext::postgres::PgViewProjector;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use uuid::Uuid;

const EVENT_COUNT: usize = 100;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
static APPLY_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CURRENT_APPLIES: AtomicUsize = AtomicUsize::new(0);
static MAX_APPLIES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, DeserializeVersion, Event, Serialize, SerializeVersion)]
struct CombinedEvent {
    ordinal: u64,
    sent_at_us: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CombinedView {
    ordinals: Vec<u64>,
    stream_sequences: Vec<u64>,
    projection_latencies_us: Vec<u64>,
}

impl View for CombinedView {
    type EventGroup = CombinedEvent;

    fn apply<'de, E>(&mut self, context: Context<'de, E, Self::EventGroup>) -> Changed
    where
        E: Envelope,
    {
        APPLY_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        let current = CURRENT_APPLIES.fetch_add(1, Ordering::SeqCst) + 1;
        MAX_APPLIES.fetch_max(current, Ordering::SeqCst);
        self.ordinals.push(context.ordinal);
        self.stream_sequences
            .push(u64::from(Context::sequence(&context)));
        self.projection_latencies_us
            .push(unix_time_us().saturating_sub(context.sent_at_us));
        CURRENT_APPLIES.fetch_sub(1, Ordering::SeqCst);
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "requires isolated NATS and PostgreSQL services plus the local esrc Cargo patch"]
async fn two_clients_project_postgres_sequentially_through_failure_and_recovery(
) -> Result<(), Box<dyn std::error::Error>> {
    APPLY_ATTEMPTS.store(0, Ordering::SeqCst);
    CURRENT_APPLIES.store(0, Ordering::SeqCst);
    MAX_APPLIES.store(0, Ordering::SeqCst);
    let started = Instant::now();
    let pg_pool = sqlx::PgPool::connect(&env::var("DATABASE_URL")?).await?;
    let table = format!("milestone0017_{}", Uuid::now_v7().simple());
    let projector = PgViewProjector::<CombinedView>::new(table.clone(), pg_pool.clone());
    projector.clone().setup().await?;
    sqlx::query(&format!(
        "ALTER TABLE {table} ADD CONSTRAINT reject_first_sequence CHECK (stream_sequence <> 1)"
    ))
    .execute(&pg_pool)
    .await?;

    let context_one = connect_nats().await?;
    let context_two = connect_nats().await?;
    let prefix: &'static str =
        Box::leak(format!("MILESTONE0017{}", Uuid::now_v7().simple()).into_boxed_str());
    let durable = format!("pg-view-{}", Uuid::now_v7().simple());
    let consumer_config = ConsumerConfig {
        ack_wait: Duration::from_millis(100),
        ..Default::default()
    };
    let store_one = NatsStore::try_new(context_one.clone(), prefix)
        .await?
        .update_durable_consumer_option(consumer_config.clone());
    let store_two = NatsStore::try_new(context_two, prefix)
        .await?
        .update_durable_consumer_option(consumer_config);
    let first = start_view(store_one.clone(), projector.clone(), durable.clone());
    wait_for_consumer(&context_one, prefix, &durable).await?;
    let second = start_view(store_two, projector.clone(), durable.clone());
    wait_for_waiting_pulls(&context_one, prefix, &durable, 2).await?;

    let view_id = Uuid::now_v7();
    let mut publisher = store_one;
    let mut publish_latencies = Vec::with_capacity(EVENT_COUNT);
    publish(&mut publisher, view_id, 1, &mut publish_latencies).await?;
    let redeliveries = wait_for_redelivery(&context_one, prefix, &durable).await?;
    let view_during_failure = projector.load(view_id).await?;
    let checkpoint_during_failure = checkpoint(&pg_pool, &table, view_id).await?;
    let ack_floor_during_failure = consumer_ack_floor(&context_one, prefix, &durable).await?;
    if view_during_failure.is_some()
        || checkpoint_during_failure.is_some()
        || ack_floor_during_failure != 0
    {
        return Err("failed projection committed partial state or acknowledgement".into());
    }

    sqlx::query(&format!(
        "ALTER TABLE {table} DROP CONSTRAINT reject_first_sequence"
    ))
    .execute(&pg_pool)
    .await?;
    wait_for_checkpoint(&pg_pool, &table, view_id, 1).await?;
    for ordinal in 2..=EVENT_COUNT as u64 {
        publish(&mut publisher, view_id, ordinal, &mut publish_latencies).await?;
    }
    let view = wait_for_view(&projector, view_id, EVENT_COUNT).await?;
    wait_for_ack_floor(&context_one, prefix, &durable, EVENT_COUNT as u64).await?;

    let mut consumer = context_one
        .get_stream(prefix)
        .await?
        .get_consumer::<ConsumerConfig>(&durable)
        .await
        .map_err(|error| error.to_string())?;
    let info = consumer.info().await?;
    let active_consumers = context_one
        .get_stream(prefix)
        .await?
        .consumer_names()
        .count()
        .await;
    let final_checkpoint = checkpoint(&pg_pool, &table, view_id)
        .await?
        .expect("checkpoint should exist");
    let checksum = view.ordinals.iter().sum::<u64>();
    let order_is_strict = view.ordinals.iter().copied().eq(1..=EVENT_COUNT as u64)
        && view
            .stream_sequences
            .windows(2)
            .all(|pair| pair[0] < pair[1]);
    let duration = started.elapsed();
    let attempts = APPLY_ATTEMPTS.load(Ordering::SeqCst);
    let max_overlap = MAX_APPLIES.load(Ordering::SeqCst);

    stop_views([first, second]).await;
    context_one.delete_stream(prefix).await?;
    sqlx::query(&format!("DROP TABLE {table}"))
        .execute(&pg_pool)
        .await?;

    println!(
        "fixture_events={EVENT_COUNT} clients=2 active_consumers={active_consumers} max_ack_pending={} max_projector_overlap={max_overlap} redeliveries_before_recovery={redeliveries} apply_attempts={attempts} committed_effects={} order={} checksum={checksum} checkpoint={final_checkpoint} expected_checkpoint={} errors_expected={} errors_unexpected=0 timeouts=0 publish_p50_us={} publish_p95_us={} publish_p99_us={} projection_p50_us={} projection_p95_us={} projection_p99_us={} duration_ms={} consumer_creation_rate_per_s={:.3} cleanup=PASS",
        info.config.max_ack_pending,
        view.ordinals.len(),
        if order_is_strict { "STRICT" } else { "VIOLATED" },
        view.stream_sequences.last().copied().unwrap_or_default(),
        attempts.saturating_sub(view.ordinals.len()),
        percentile_us(&publish_latencies, 50),
        percentile_us(&publish_latencies, 95),
        percentile_us(&publish_latencies, 99),
        percentile_us_values(&view.projection_latencies_us, 50),
        percentile_us_values(&view.projection_latencies_us, 95),
        percentile_us_values(&view.projection_latencies_us, 99),
        duration.as_millis(),
        active_consumers as f64 / duration.as_secs_f64(),
    );

    assert_eq!(active_consumers, 1);
    assert_eq!(info.config.max_ack_pending, 1);
    assert_eq!(info.num_ack_pending, 0);
    assert_eq!(info.num_pending, 0);
    assert_eq!(max_overlap, 1);
    assert!(redeliveries >= 1);
    assert!(attempts > EVENT_COUNT);
    assert_eq!(view.ordinals.len(), EVENT_COUNT);
    assert!(order_is_strict);
    assert_eq!(checksum, 5050);
    assert_eq!(final_checkpoint as u64, EVENT_COUNT as u64);
    Ok(())
}

async fn publish(
    store: &mut NatsStore,
    id: Uuid,
    ordinal: u64,
    latencies: &mut Vec<Duration>,
) -> esrc::error::Result<()> {
    let started = Instant::now();
    store
        .publish_to_automation(
            id,
            CombinedEvent {
                ordinal,
                sent_at_us: unix_time_us(),
            },
        )
        .await?;
    latencies.push(started.elapsed());
    Ok(())
}

fn start_view(
    store: NatsStore,
    projector: PgViewProjector<CombinedView>,
    durable: String,
) -> JoinHandle<esrc::error::Result<()>> {
    tokio::spawn(async move { store.start_view_automation(projector, &durable).await })
}

async fn stop_views<const N: usize>(handles: [JoinHandle<esrc::error::Result<()>>; N]) {
    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
    }
}

async fn connect_nats() -> Result<async_nats::jetstream::Context, Box<dyn std::error::Error>> {
    let urls = env::var("NATS_URL")?
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let client = async_nats::ConnectOptions::with_user_and_password(
        env::var("NATS_USER")?,
        env::var("NATS_PASSWORD")?,
    )
    .connect(urls)
    .await?;
    Ok(async_nats::jetstream::new(client))
}

async fn wait_for_consumer(
    context: &async_nats::jetstream::Context,
    stream_name: &str,
    durable: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            if let Ok(stream) = context.get_stream(stream_name).await
                && stream.get_consumer::<ConsumerConfig>(durable).await.is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}

async fn wait_for_waiting_pulls(
    context: &async_nats::jetstream::Context,
    stream_name: &str,
    durable: &str,
    expected: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            let mut consumer = context
                .get_stream(stream_name)
                .await?
                .get_consumer::<ConsumerConfig>(durable)
                .await
                .map_err(|error| error.to_string())?;
            if consumer.info().await?.num_waiting >= expected {
                return Result::<(), Box<dyn std::error::Error>>::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

async fn wait_for_redelivery(
    context: &async_nats::jetstream::Context,
    stream_name: &str,
    durable: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            let mut consumer = context
                .get_stream(stream_name)
                .await?
                .get_consumer::<ConsumerConfig>(durable)
                .await
                .map_err(|error| error.to_string())?;
            let redelivered = consumer.info().await?.num_redelivered;
            if redelivered >= 1 {
                return Result::<usize, Box<dyn std::error::Error>>::Ok(redelivered);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

async fn consumer_ack_floor(
    context: &async_nats::jetstream::Context,
    stream_name: &str,
    durable: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut consumer = context
        .get_stream(stream_name)
        .await?
        .get_consumer::<ConsumerConfig>(durable)
        .await
        .map_err(|error| error.to_string())?;
    Ok(consumer.info().await?.ack_floor.consumer_sequence)
}

async fn wait_for_ack_floor(
    context: &async_nats::jetstream::Context,
    stream_name: &str,
    durable: &str,
    expected: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            let mut consumer = context
                .get_stream(stream_name)
                .await?
                .get_consumer::<ConsumerConfig>(durable)
                .await
                .map_err(|error| error.to_string())?;
            let info = consumer.info().await?;
            if info.ack_floor.consumer_sequence >= expected && info.num_ack_pending == 0 {
                return Result::<(), Box<dyn std::error::Error>>::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

async fn checkpoint(
    pool: &sqlx::PgPool,
    table: &str,
    id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<i64>>(&format!(
        "SELECT stream_sequence FROM {table} WHERE view_id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
}

async fn wait_for_checkpoint(
    pool: &sqlx::PgPool,
    table: &str,
    id: Uuid,
    expected: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            if checkpoint(pool, table, id).await? == Some(expected) {
                return Result::<(), sqlx::Error>::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

async fn wait_for_view(
    projector: &PgViewProjector<CombinedView>,
    id: Uuid,
    expected: usize,
) -> Result<CombinedView, Box<dyn std::error::Error>> {
    Ok(tokio::time::timeout(OPERATION_TIMEOUT, async {
        loop {
            if let Some(view) = projector.load(id).await?
                && view.ordinals.len() == expected
            {
                return Result::<CombinedView, esrc_ext::postgres::PgViewProjectorError>::Ok(view);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??)
}

fn unix_time_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_micros() as u64
}

fn percentile_us(samples: &[Duration], percentile: usize) -> u128 {
    let mut values = samples.to_vec();
    values.sort_unstable();
    let index = ((values.len() - 1) * percentile).div_ceil(100);
    values[index].as_micros()
}

fn percentile_us_values(samples: &[u64], percentile: usize) -> u64 {
    let mut values = samples.to_vec();
    values.sort_unstable();
    let index = ((values.len() - 1) * percentile).div_ceil(100);
    values[index]
}
