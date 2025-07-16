use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use clap::Parser;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tracing::{Level, error, info};
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
    // using BTreeMap instead of HashMap to keep the stats sorted by pool name
    pools: BTreeMap<String, PoolStats>,
}

#[derive(Clone)]
struct AppState {
    report_tx: mpsc::Sender<Report>,
    stats_rx: watch::Receiver<String>,
}

async fn post_report(
    State(state): State<AppState>,
    Json(report): Json<Report>,
) -> impl IntoResponse {
    if state.report_tx.send(report).await.is_err() {
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

async fn data_actor(
    mut report_rx: mpsc::Receiver<Report>,
    stats_tx: watch::Sender<String>,
    expiration_secs: u64,
) {
    let mut pools_data: HashMap<String, VecDeque<Report>> = HashMap::new();
    let mut calculation_interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            // Branch 1: A new report is received from a web handler.
            Some(report) = report_rx.recv() => {
                pools_data.entry(report.pool.clone()).or_default().push_back(report);
            }

            // Branch 2: The 1-second timer ticks, triggering a stats recalculation.
            _ = calculation_interval.tick() => {
                let now_ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let expiration_ts = now_ts.saturating_sub(expiration_secs);

                let pools = pools_data.iter_mut()
                    .map(|(pool_name, deque)| {
                        // Step 1: Prune old reports from the front of the deque.
                        while let Some(report) = deque.front() {
                            if report.timestamp < expiration_ts {
                                deque.pop_front();
                            } else {
                                break;
                            }
                        }

                        // Step 2: Calculate all required values in a single pass using fold.
                        let (total_hashrate, total_temp, unique_workers) = deque.iter().fold(
                            // The initial state of our accumulator: (hash, temp, worker_set)
                            (0.0, 0.0, HashSet::new()),
                            // The closure to update the accumulator for each report
                            |(h_acc, t_acc, mut workers_set), report| {
                                workers_set.insert(&report.worker_id);
                                (h_acc + report.hashrate, t_acc + report.temperature, workers_set)
                            },
                        );

                        // Step 3: Create the final stats struct for this pool.
                        let pool_stats = if !deque.is_empty() {
                            PoolStats {
                                workers: unique_workers.len(),
                                avg_hashrate: total_hashrate / deque.len() as f64,
                                avg_temp: total_temp / deque.len() as f64,
                            }
                        } else {
                            // If there are no reports, return a default state with 0 workers and 0.0 averages.
                            PoolStats::default()
                        };

                        (pool_name.clone(), pool_stats)
                    })
                    .collect::<BTreeMap<_, _>>();

                // Step 4: Assemble the final stats object and publish it.
                let current_stats = AllStats { pools };

                if let Ok(json) = serde_json::to_string(&current_stats) {
                    info!(stats = %json, "Publishing new stats");
                    // Send the new stats to all subscribed `get_stats` handlers.
                    stats_tx.send(json).ok();
                }
            }

            // Branch 3: The report channel has closed, so the actor should shut down.
            else => {
                info!("Report channel closed. Data actor shutting down.");
                break;
            }
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

    let (report_tx, report_rx) = mpsc::channel::<Report>(1024);
    let (stats_tx, stats_rx) = watch::channel(serde_json::to_string(&AllStats::default()).unwrap());

    info!("Spawning data actor...");
    tokio::spawn(data_actor(report_rx, stats_tx, cli.expiration_secs));

    let app_state = AppState {
        report_tx,
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
