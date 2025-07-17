use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use clap::Parser;
use futures::future;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, mpsc, oneshot, watch};
use tracing::{Level, error, info, warn};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value_t = 300)]
    expiration_secs: u64,
}

static STATS_RESPONSE_HEADERS: Lazy<HeaderMap> = Lazy::new(|| {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    headers.insert(
        header::CACHE_CONTROL,
        "no-cache, no-store, must-revalidate".parse().unwrap(),
    );
    headers
});

#[derive(Debug, Deserialize, Clone)]
pub struct Report {
    worker_id: String,
    pool: String,
    hashrate: f64,
    temperature: f64,
    timestamp: u64,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct PoolStats {
    workers: usize,
    avg_hashrate: f64,
    avg_temp: f64,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct AllStats {
    pools: BTreeMap<String, PoolStats>,
}

#[derive(Debug)]
enum PoolActorCommand {
    AddReport(Report),
    CalculateStats(oneshot::Sender<PoolStats>),
}

type ActorRegistry = RwLock<HashMap<String, mpsc::Sender<PoolActorCommand>>>;

#[derive(Clone)]
struct AppState {
    actor_registry: Arc<ActorRegistry>,
    stats_rx: watch::Receiver<String>,
    expiration_secs: u64,
}

async fn post_report(
    State(state): State<AppState>,
    Json(report): Json<Report>,
) -> impl IntoResponse {
    let mut registry = state.actor_registry.write().await;

    let actor_tx = registry.entry(report.pool.clone()).or_insert_with(|| {
        info!("Spawning new actor for pool: {}", report.pool);
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(pool_actor(rx, state.expiration_secs));
        tx
    });

    if actor_tx
        .send(PoolActorCommand::AddReport(report))
        .await
        .is_err()
    {
        error!("Report channel is closed. This is a critical internal error.");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    StatusCode::OK
}

async fn get_stats(State(state): State<AppState>) -> impl IntoResponse {
    (
        STATS_RESPONSE_HEADERS.clone(),
        state.stats_rx.borrow().clone(),
    )
}

/// An actor that manages the data and computes stats for a single pool.
async fn pool_actor(mut command_rx: mpsc::Receiver<PoolActorCommand>, expiration_secs: u64) {
    let mut reports: VecDeque<Report> = VecDeque::new();

    while let Some(command) = command_rx.recv().await {
        match command {
            PoolActorCommand::AddReport(report) => {
                reports.push_back(report);
            }
            PoolActorCommand::CalculateStats(reply_tx) => {
                // Step 1: Prune old reports based on the current time.
                let now_ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let expiration_ts = now_ts.saturating_sub(expiration_secs);
                reports.retain(|r| r.timestamp >= expiration_ts);

                // Step 2: Perform the calculation, just like the old single actor did.
                let (total_hashrate, total_temp, unique_workers) =
                    reports
                        .iter()
                        .fold((0.0, 0.0, HashSet::new()), |(h, t, mut w), r| {
                            w.insert(&r.worker_id);
                            (h + r.hashrate, t + r.temperature, w)
                        });

                let pool_stats = if !reports.is_empty() {
                    PoolStats {
                        workers: unique_workers.len(),
                        avg_hashrate: total_hashrate / reports.len() as f64,
                        avg_temp: total_temp / reports.len() as f64,
                    }
                } else {
                    PoolStats::default()
                };

                // Step 3: Send the small, final PoolStats struct back.
                reply_tx.send(pool_stats).ok();
            }
        }
    }
    info!("Pool actor shutting down as its channel was closed.");
}

/// A lightweight actor that orchestrates the stats collection from a RwLock<HashMap>.
async fn stats_aggregator_actor(
    actor_registry: Arc<ActorRegistry>,
    stats_tx: watch::Sender<String>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        interval.tick().await;

        // Phase 1: Collect actor senders from the locked HashMap
        let registry_lock = actor_registry.read().await;

        // Check if the map is empty to avoid unnecessary work.
        if registry_lock.is_empty() {
            drop(registry_lock); // Release the lock before continuing.
            let empty_stats = AllStats::default();
            if let Ok(json) = serde_json::to_string(&empty_stats) {
                stats_tx.send(json).ok();
            }
            continue;
        }

        // Create a copy of the necessary data (pool names and senders)
        // so we can release the lock as quickly as possible.
        let actors_to_query: Vec<(String, mpsc::Sender<PoolActorCommand>)> = registry_lock
            .iter()
            .map(|(pool_name, sender)| (pool_name.clone(), sender.clone()))
            .collect();

        // Release the read lock. Now other tasks can access the registry.
        drop(registry_lock);

        // Phase 2: Asynchronously query all collected actors
        let mut reply_channels = vec![];
        let mut pool_names_for_replies = vec![];
        let mut dead_pools_to_remove = vec![];

        for (pool_name, actor_tx) in actors_to_query {
            let (reply_tx, reply_rx) = oneshot::channel();
            let command = PoolActorCommand::CalculateStats(reply_tx);

            if actor_tx.send(command).await.is_err() {
                // The actor's channel is closed. Mark it for removal later.
                dead_pools_to_remove.push(pool_name);
            } else {
                reply_channels.push(reply_rx);
                pool_names_for_replies.push(pool_name);
            }
        }

        // Phase 2a: Clean up dead actors. This requires a write lock.
        if !dead_pools_to_remove.is_empty() {
            let mut write_lock = actor_registry.write().await;
            for pool_name in dead_pools_to_remove {
                warn!("Removing dead actor for pool: {}", &pool_name);
                write_lock.remove(&pool_name);
            }
        }

        // Phase 3: Wait for all live actors to reply
        let all_replies = future::join_all(reply_channels).await;
        let mut final_pools = BTreeMap::new();

        for (pool_name, reply_result) in pool_names_for_replies.into_iter().zip(all_replies) {
            // reply_result is a Result from the oneshot channel receive.
            if let Ok(pool_stats) = reply_result {
                final_pools.insert(pool_name, pool_stats);
            }
        }

        // Phase 4: Assemble and publish the final JSON
        let current_stats = AllStats { pools: final_pools };
        if let Ok(json) = serde_json::to_string(&current_stats) {
            stats_tx.send(json).ok(); // Errors are fine if no one is listening.
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();
    let actor_registry = Arc::new(RwLock::new(HashMap::new()));
    let (stats_tx, stats_rx) = watch::channel(serde_json::to_string(&AllStats::default()).unwrap());

    info!("Spawning stats aggregator actor...");
    tokio::spawn(stats_aggregator_actor(actor_registry.clone(), stats_tx));

    let app_state = AppState {
        actor_registry,
        stats_rx,
        expiration_secs: cli.expiration_secs,
    };

    let app = Router::new()
        .route("/report", post(post_report))
        .route("/stats", get(get_stats))
        .with_state(app_state);

    let addr = "127.0.0.1:3000";
    info!("Server listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
