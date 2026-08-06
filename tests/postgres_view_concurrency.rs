use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use esrc::event::event_model::view::Changed;
use esrc::prelude::*;
use esrc::version::{DeserializeVersion, SerializeVersion};
use esrc_ext::postgres::PgViewProjector;
use serde::{Deserialize, Serialize};
use serde_json::Deserializer;
use tokio::sync::Barrier;
use uuid::Uuid;

const CONCURRENT_DELIVERIES: usize = 8;
static APPLY_CALLS: AtomicUsize = AtomicUsize::new(0);
static CURRENT_APPLIES: AtomicUsize = AtomicUsize::new(0);
static MAX_APPLIES: AtomicUsize = AtomicUsize::new(0);
static REPLAY_STARTED: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, DeserializeVersion, Event, Serialize, SerializeVersion)]
enum CounterEvent {
    Added(u64),
    Ignored,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CounterView {
    total: u64,
}

impl View for CounterView {
    type EventGroup = CounterEvent;

    fn apply<'de, E>(&mut self, context: Context<'de, E, Self::EventGroup>) -> Changed
    where
        E: Envelope,
    {
        APPLY_CALLS.fetch_add(1, Ordering::SeqCst);
        let current = CURRENT_APPLIES.fetch_add(1, Ordering::SeqCst) + 1;
        MAX_APPLIES.fetch_max(current, Ordering::SeqCst);
        if u64::from(Context::sequence(&context)) <= 2 {
            REPLAY_STARTED.fetch_add(1, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(100));
        let changed = match *context {
            CounterEvent::Added(value) => {
                self.total += value;
                true
            },
            CounterEvent::Ignored => false,
        };
        CURRENT_APPLIES.fetch_sub(1, Ordering::SeqCst);
        changed
    }
}

struct SyntheticEnvelope {
    id: Uuid,
    sequence: Sequence,
    payload: Vec<u8>,
}

impl SyntheticEnvelope {
    fn new(id: Uuid, sequence: u64, event: &CounterEvent) -> Self {
        Self {
            id,
            sequence: sequence.into(),
            payload: serde_json::to_vec(event).expect("synthetic event should serialize"),
        }
    }
}

impl Envelope for SyntheticEnvelope {
    fn id(&self) -> Uuid {
        self.id
    }

    fn sequence(&self) -> Sequence {
        self.sequence
    }

    fn timestamp(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }

    fn name(&self) -> &str {
        CounterEvent::name()
    }

    fn get_metadata(&self, _key: &str) -> Option<&str> {
        None
    }

    fn deserialize<E>(&self) -> esrc::error::Result<E>
    where
        E: DeserializeVersion + Event,
    {
        if E::name() != self.name() {
            return Err(esrc::Error::Invalid);
        }
        let mut deserializer = Deserializer::from_slice(&self.payload);
        E::deserialize_version(&mut deserializer, CounterEvent::version())
            .map_err(|error| esrc::Error::Format(error.into()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn setup_migrates_an_existing_payload_table_without_losing_rows(
) -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::PgPool::connect(&env::var("DATABASE_URL")?).await?;
    let table = format!("milestone0018_migration_{}", Uuid::now_v7().simple());
    let id = Uuid::now_v7();
    sqlx::query(&format!(
        "CREATE TABLE {table}(view_id uuid NOT NULL PRIMARY KEY, payload jsonb NOT NULL)"
    ))
    .execute(&pool)
    .await?;
    sqlx::query(&format!(
        "INSERT INTO {table} (view_id, payload) VALUES ($1, $2)"
    ))
    .bind(id)
    .bind(serde_json::to_value(CounterView { total: 7 })?)
    .execute(&pool)
    .await?;

    let projector = PgViewProjector::<CounterView>::new(table.clone(), pool.clone());
    projector.clone().setup().await?;
    let view = projector
        .load(id)
        .await?
        .expect("legacy view should remain");
    let committed_checkpoint = checkpoint(&pool, &table, id).await?;
    let nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns WHERE table_name = $1 AND column_name = 'stream_sequence'",
    )
    .bind(&table)
    .fetch_one(&pool)
    .await?;
    let separate_checkpoint_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(format!("{table}_esrc_checkpoint"))
            .fetch_one(&pool)
            .await?;

    println!(
        "legacy_payload_total={} checkpoint_is_null={} stream_sequence_nullable={nullable}",
        view.total,
        committed_checkpoint.is_none()
    );
    cleanup(&pool, &table).await?;
    assert_eq!(view.total, 7);
    assert_eq!(committed_checkpoint, None);
    assert_eq!(nullable, "YES");
    assert_eq!(separate_checkpoint_table, None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn unchanged_first_event_stores_default_payload_and_checkpoint(
) -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, mut projector) = setup_projector().await?;
    let id = Uuid::now_v7();
    let envelope = SyntheticEnvelope::new(id, 1, &CounterEvent::Ignored);
    let context = Context::try_with_envelope(&envelope)?;
    projector.project(context).await?;

    let view = projector
        .load(id)
        .await?
        .expect("checkpoint row should exist");
    let committed_checkpoint = checkpoint(&pool, &table, id).await?;
    println!(
        "unchanged_event_default_total={} checkpoint={}",
        view.total,
        committed_checkpoint.unwrap_or_default()
    );
    cleanup(&pool, &table).await?;
    assert_eq!(view.total, 0);
    assert_eq!(committed_checkpoint, Some(1));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn concurrent_redelivery_commits_one_view_effect() -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, projector) = setup_projector().await?;
    let id = Uuid::now_v7();
    let start = Arc::new(Barrier::new(CONCURRENT_DELIVERIES));

    let mut tasks = Vec::with_capacity(CONCURRENT_DELIVERIES);
    for _ in 0..CONCURRENT_DELIVERIES {
        let start = start.clone();
        let mut projector = projector.clone();
        tasks.push(tokio::spawn(async move {
            let envelope = SyntheticEnvelope::new(id, 1, &CounterEvent::Added(1));
            let context =
                Context::try_with_envelope(&envelope).expect("synthetic envelope should decode");
            start.wait().await;
            let started = Instant::now();
            let result = projector.project(context).await;
            (result, started.elapsed())
        }));
    }

    let mut latencies = Vec::with_capacity(CONCURRENT_DELIVERIES);
    for task in tasks {
        let (result, elapsed) = task.await?;
        result?;
        latencies.push(elapsed);
    }

    let view = projector.load(id).await?.expect("view should exist");
    let apply_calls = APPLY_CALLS.load(Ordering::SeqCst);
    println!(
        "fixture_deliveries={CONCURRENT_DELIVERIES} stream_sequence=1 apply_calls={apply_calls} committed_total={} expected_apply_calls=1 errors=0 timeouts=0 latency_p50_us={} latency_p95_us={} latency_p99_us={}",
        view.total,
        percentile_us(&latencies, 50),
        percentile_us(&latencies, 95),
        percentile_us(&latencies, 99)
    );

    cleanup(&pool, &table).await?;

    assert_eq!(apply_calls, 1, "redelivery repeated View::apply");
    assert_eq!(
        view.total, 1,
        "redelivery changed committed view more than once"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn distinct_view_ids_share_one_database_critical_section(
) -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, projector) = setup_projector().await?;
    let start = Arc::new(Barrier::new(CONCURRENT_DELIVERIES));
    let mut tasks = Vec::with_capacity(CONCURRENT_DELIVERIES);

    for ordinal in 0..CONCURRENT_DELIVERIES {
        let start = start.clone();
        let mut projector = projector.clone();
        tasks.push(tokio::spawn(async move {
            let envelope =
                SyntheticEnvelope::new(Uuid::now_v7(), ordinal as u64 + 1, &CounterEvent::Added(1));
            let context =
                Context::try_with_envelope(&envelope).expect("synthetic envelope should decode");
            start.wait().await;
            let started = Instant::now();
            let result = projector.project(context).await;
            (result, started.elapsed())
        }));
    }
    let mut latencies = Vec::with_capacity(CONCURRENT_DELIVERIES);
    for task in tasks {
        let (result, elapsed) = task.await?;
        result?;
        latencies.push(elapsed);
    }

    let maximum = MAX_APPLIES.load(Ordering::SeqCst);
    println!(
        "fixture_deliveries={CONCURRENT_DELIVERIES} distinct_view_ids={CONCURRENT_DELIVERIES} apply_calls={} max_critical_section_overlap={maximum} errors=0 timeouts=0 latency_p50_us={} latency_p95_us={} latency_p99_us={}",
        APPLY_CALLS.load(Ordering::SeqCst),
        percentile_us(&latencies, 50),
        percentile_us(&latencies, 95),
        percentile_us(&latencies, 99)
    );
    cleanup(&pool, &table).await?;
    assert_eq!(maximum, 1, "view mutations overlapped across view IDs");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn sequential_events_advance_checkpoint_and_delete_cleans_row(
) -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, mut projector) = setup_projector().await?;
    let id = Uuid::now_v7();
    project_event(&mut projector, id, 1, 2).await?;
    project_event(&mut projector, id, 2, 3).await?;
    project_event(&mut projector, id, 2, 100).await?;

    let view = projector.load(id).await?.expect("view should exist");
    let committed_checkpoint = checkpoint(&pool, &table, id).await?;
    assert_eq!(view.total, 5);
    assert_eq!(committed_checkpoint, Some(2));
    assert_eq!(APPLY_CALLS.load(Ordering::SeqCst), 2);

    projector.delete_one(id).await?;
    assert!(projector.load(id).await?.is_none());
    assert_eq!(checkpoint(&pool, &table, id).await?, None);
    project_event(&mut projector, Uuid::now_v7(), 3, 1).await?;
    project_event(&mut projector, Uuid::now_v7(), 4, 1).await?;
    projector.delete().await?;
    let view_rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&pool)
        .await?;
    println!("sequential_effects=2 duplicate_sequence_skipped=1 checkpoint=2 delete_one_rows=0 delete_all_rows={view_rows}");
    assert_eq!(view_rows, 0);
    cleanup(&pool, &table).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn replay_and_live_projection_do_not_interleave() -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, projector) = setup_projector().await?;
    let id = Uuid::now_v7();
    let first = Box::leak(Box::new(SyntheticEnvelope::new(
        id,
        1,
        &CounterEvent::Added(1),
    )));
    let second = Box::leak(Box::new(SyntheticEnvelope::new(
        id,
        2,
        &CounterEvent::Added(1),
    )));
    let events = vec![
        Context::try_with_envelope(first)?,
        Context::try_with_envelope(second)?,
    ];
    let replay_projector = projector.clone();
    let replay = tokio::spawn(async move { replay_projector.replay(id, events).await });

    tokio::time::timeout(Duration::from_secs(5), async {
        while REPLAY_STARTED.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let mut live_projector = projector.clone();
    let live = tokio::spawn(async move { project_event(&mut live_projector, id, 3, 1).await });
    replay.await??;
    live.await??;

    let view = projector.load(id).await?.expect("view should exist");
    let checkpoint = checkpoint(&pool, &table, id).await?;
    println!(
        "replay_events=2 live_events=1 final_total={} checkpoint={} max_critical_section_overlap={}",
        view.total,
        checkpoint.unwrap_or_default(),
        MAX_APPLIES.load(Ordering::SeqCst)
    );
    cleanup(&pool, &table).await?;
    assert_eq!(view.total, 3);
    assert_eq!(checkpoint, Some(3));
    assert_eq!(MAX_APPLIES.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn checkpoint_failure_rolls_back_view_change() -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, mut projector) = setup_projector().await?;
    sqlx::query(&format!(
        "ALTER TABLE {table} ADD CONSTRAINT reject_sequence_five CHECK (stream_sequence < 5)"
    ))
    .execute(&pool)
    .await?;
    let id = Uuid::now_v7();

    let result = project_event(&mut projector, id, 5, 1).await;
    let view = projector.load(id).await?;
    let checkpoint = checkpoint(&pool, &table, id).await?;
    println!(
        "injected_checkpoint_failure={} view_rows={} checkpoint_rows={}",
        result.is_err(),
        usize::from(view.is_some()),
        usize::from(checkpoint.is_some())
    );
    cleanup(&pool, &table).await?;
    assert!(result.is_err());
    assert!(view.is_none(), "view write escaped the failed transaction");
    assert_eq!(checkpoint, None);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DATABASE_URL for an isolated PostgreSQL database"]
async fn replay_rejects_out_of_order_input_and_preserves_empty_replay_semantics(
) -> Result<(), Box<dyn std::error::Error>> {
    reset_probes();
    let (pool, table, projector) = setup_projector().await?;
    let id = Uuid::now_v7();
    let second = SyntheticEnvelope::new(id, 2, &CounterEvent::Added(1));
    let first = SyntheticEnvelope::new(id, 1, &CounterEvent::Added(1));
    let out_of_order = vec![
        Context::try_with_envelope(&second)?,
        Context::try_with_envelope(&first)?,
    ];

    let result = projector.replay(id, out_of_order).await;
    assert!(matches!(
        result,
        Err(esrc_ext::postgres::PgViewProjectorError::ReplayOutOfOrder)
    ));
    assert!(projector.load(id).await?.is_none());

    projector
        .replay::<SyntheticEnvelope>(id, Vec::new())
        .await?;
    let view = projector
        .load(id)
        .await?
        .expect("empty replay should preserve the prior default-row behavior");
    let committed_checkpoint = checkpoint(&pool, &table, id).await?;
    println!(
        "out_of_order_rejected=true empty_replay_default_total={} empty_replay_checkpoint_rows={}",
        view.total,
        usize::from(committed_checkpoint.is_some())
    );
    cleanup(&pool, &table).await?;
    assert_eq!(view.total, 0);
    assert_eq!(committed_checkpoint, None);
    Ok(())
}

fn reset_probes() {
    APPLY_CALLS.store(0, Ordering::SeqCst);
    CURRENT_APPLIES.store(0, Ordering::SeqCst);
    MAX_APPLIES.store(0, Ordering::SeqCst);
    REPLAY_STARTED.store(0, Ordering::SeqCst);
}

async fn setup_projector(
) -> Result<(sqlx::PgPool, String, PgViewProjector<CounterView>), Box<dyn std::error::Error>> {
    let pool = sqlx::PgPool::connect(&env::var("DATABASE_URL")?).await?;
    let table = format!("milestone0018_{}", Uuid::now_v7().simple());
    let projector = PgViewProjector::<CounterView>::new(table.clone(), pool.clone());
    projector.clone().setup().await?;
    Ok((pool, table, projector))
}

async fn project_event(
    projector: &mut PgViewProjector<CounterView>,
    id: Uuid,
    sequence: u64,
    value: u64,
) -> Result<(), esrc_ext::postgres::PgViewProjectorError> {
    let envelope = SyntheticEnvelope::new(id, sequence, &CounterEvent::Added(value));
    let context = Context::try_with_envelope(&envelope).expect("synthetic envelope should decode");
    projector.project(context).await
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

async fn cleanup(pool: &sqlx::PgPool, table: &str) -> Result<(), sqlx::Error> {
    sqlx::query(&format!("DROP TABLE {table}"))
        .execute(pool)
        .await?;
    Ok(())
}

fn percentile_us(samples: &[Duration], percentile: usize) -> u128 {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index].as_micros()
}
