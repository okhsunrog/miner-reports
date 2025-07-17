use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use clap::Parser;
use crossbeam_queue::SegQueue;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tracing::{Level, info};
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

type ReportQueue = SegQueue<Report>;

#[derive(Clone)]
struct AppState {
    report_queue: Arc<ReportQueue>,
    stats_rx: watch::Receiver<String>,
}

async fn post_report(
    State(state): State<AppState>,
    Json(report): Json<Report>,
) -> impl IntoResponse {
    // Trivial, lock-free, and incredibly fast.
    state.report_queue.push(report);
    StatusCode::OK
}

async fn get_stats(State(state): State<AppState>) -> impl IntoResponse {
    (
        STATS_RESPONSE_HEADERS.clone(),
        state.stats_rx.borrow().clone(),
    )
}

// The Rayon-powered Stats Aggregator
async fn stats_aggregator_actor(
    report_queue: Arc<ReportQueue>,
    stats_tx: watch::Sender<String>,
    expiration_secs: u64,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    // This is the aggregator's own persistent state.
    let mut pool_data: HashMap<String, VecDeque<Report>> = HashMap::new();

    loop {
        interval.tick().await;

        // Step 1: Drain the global queue
        let mut new_reports = Vec::with_capacity(report_queue.len());
        // SegQueue is fantastic for concurrent writes but terrible for parallel processing because you can't easily "split" it
        // so moving the data to Vec
        while let Some(report) = report_queue.pop() {
            new_reports.push(report);
        }

        // Step 2: Parallel Grouping with Rayon. Parrallel fold/reduce do the magic here!
        let new_data_by_pool: HashMap<String, Vec<Report>> = new_reports
            .into_par_iter() // parallel iterator
            .fold(
                HashMap::new, // each CPU core get's a small HashMap to fill :)
                |mut map: HashMap<String, Vec<Report>>, report| {
                    map.entry(report.pool.clone()).or_default().push(report);
                    map
                },
            )
            .reduce(HashMap::new, |mut map1, map2| {
                // single-theaded, collecting into one HashMap
                for (key, val) in map2 {
                    map1.entry(key).or_default().extend(val);
                }
                map1
            });

        // Step 3: Merge the results into persistent state (single-threaded)
        for (pool, reports) in new_data_by_pool {
            pool_data.entry(pool).or_default().extend(reports);
        }

        // Step 4: Prune and Calculate Stats in Parallel with Rayon, one thread per pool
        let now_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expiration_ts = now_ts.saturating_sub(expiration_secs);

        let pools: BTreeMap<String, PoolStats> = pool_data
            .par_iter_mut() // Use a parallel mutable iterator
            .map(|(pool_name, deque)| {
                // This closure runs in parallel for each pool.
                deque.retain(|r| r.timestamp >= expiration_ts);

                let (total_hashrate, total_temp, unique_workers) =
                    deque
                        .iter()
                        .fold((0.0, 0.0, HashSet::new()), |(h, t, mut w), r| {
                            w.insert(&r.worker_id);
                            (h + r.hashrate, t + r.temperature, w)
                        });

                let stats = if !deque.is_empty() {
                    PoolStats {
                        workers: unique_workers.len(),
                        avg_hashrate: total_hashrate / deque.len() as f64,
                        avg_temp: total_temp / deque.len() as f64,
                    }
                } else {
                    PoolStats::default()
                };

                (pool_name.clone(), stats)
            })
            .collect(); // Rayon's .collect() builds the BTreeMap in a parallel-friendly way.

        // Step 5: Clean up empty deques from the main state
        // This must be done in a separate, single-threaded step.
        pool_data.retain(|_, deque| !deque.is_empty());

        let current_stats = AllStats { pools };
        if let Ok(json) = serde_json::to_string(&current_stats) {
            stats_tx.send(json).ok();
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
    info!(config = ?cli, "Service starting with configuration");

    let report_queue = Arc::new(ReportQueue::new());
    let (stats_tx, stats_rx) = watch::channel(serde_json::to_string(&AllStats::default()).unwrap());

    info!("Spawning Rayon-powered stats aggregator actor...");
    tokio::spawn(stats_aggregator_actor(
        report_queue.clone(),
        stats_tx,
        cli.expiration_secs,
    ));

    let app_state = AppState {
        report_queue,
        stats_rx,
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
