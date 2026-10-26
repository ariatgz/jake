//! Scrapes run on their own OS thread with their own Tokio runtime, database
//! pool and HTTP client. The Discord gateway, slash commands and buttons live on
//! the main runtime, so a long scrape (hundreds of pages to download and parse,
//! thousands of rows to write) can no longer starve them of CPU time, worker
//! threads or database connections.
//!
//! The thread takes one job at a time. `Handle::run` queues a job and waits for
//! its summary; while one is running, further requests are refused with `Busy`.

use crate::db::Profile;
use crate::pipeline::{self, RunSummary};
use poise::serenity_prelude::{ChannelId, Http};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};

/// A scrape that runs longer than this is abandoned so the next one can start.
const DEFAULT_TIMEOUT_MINUTES: u64 = 25;

struct Job {
    profiles: Vec<Profile>,
    channel_override: Option<ChannelId>,
    reply: oneshot::Sender<RunSummary>,
}

/// Something is already scraping.
#[derive(Debug)]
pub struct Busy;

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a scrape is already running")
    }
}

impl std::error::Error for Busy {}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Job>,
    busy: Arc<AtomicBool>,
}

impl Handle {
    /// Runs the scrape -> store -> notify pipeline for `profiles` on the scraper
    /// thread and waits for the result. `channel_override` posts everything to
    /// one channel (used by `/scrape`).
    pub async fn run(&self, profiles: Vec<Profile>, channel_override: Option<ChannelId>) -> Result<RunSummary, Busy> {
        if self.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err(Busy);
        }
        let (reply, result) = oneshot::channel();
        if self.tx.send(Job { profiles, channel_override, reply }).is_err() {
            self.busy.store(false, Ordering::Release);
            let mut summary = RunSummary::default();
            summary.errors.push("the scraper thread has stopped; restart the bot".into());
            return Ok(summary);
        }
        Ok(result.await.unwrap_or_else(|_| {
            self.busy.store(false, Ordering::Release);
            let mut summary = RunSummary::default();
            summary.errors.push("the scraper thread stopped mid-run; restart the bot".into());
            summary
        }))
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).filter(|v| *v > 0).unwrap_or(default)
}

/// Starts the scraper thread. `lock` is shared with the "Show next" button so a
/// batch is never posted while a scrape is posting for the same profile.
pub fn spawn(http: Arc<Http>, database_url: String, user_agent: String, lock: Arc<Mutex<()>>) -> Handle {
    let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
    let busy = Arc::new(AtomicBool::new(false));
    let handle = Handle { tx, busy: busy.clone() };

    let threads = env_u64("SCRAPER_THREADS", 1) as usize;
    let timeout = Duration::from_secs(env_u64("SCRAPE_TIMEOUT_MINUTES", DEFAULT_TIMEOUT_MINUTES) * 60);

    std::thread::Builder::new()
        .name("scraper".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(threads)
                .thread_name("scraper-rt")
                .enable_all()
                .build()
                .expect("scraper runtime");

            runtime.block_on(async move {
                // Its own small pool: commands keep the main pool to themselves.
                let pool = match sqlx::postgres::PgPoolOptions::new().max_connections(4).connect_lazy(&database_url) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Scraper: invalid DATABASE_URL: {e}");
                        return;
                    }
                };
                let client = match crate::http_client(&user_agent) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Scraper: couldn't build HTTP client: {e}");
                        return;
                    }
                };
                println!("Scraper thread ready ({threads} worker thread{}, {} minute limit per run)", if threads == 1 { "" } else { "s" }, timeout.as_secs() / 60);

                while let Some(job) = rx.recv().await {
                    let _guard = lock.lock().await;
                    let started = std::time::Instant::now();
                    let (http, pool, client) = (http.clone(), pool.clone(), client.clone());
                    let Job { profiles, channel_override, reply } = job;
                    // On its own task so a panic inside a scraper fails this run, not the thread.
                    let task = tokio::spawn(async move { pipeline::run(&http, &pool, &client, &profiles, channel_override).await });
                    let summary = match tokio::time::timeout(timeout, task).await {
                        Ok(Ok(summary)) => summary,
                        Ok(Err(join_error)) => {
                            let mut summary = RunSummary::default();
                            summary.errors.push(format!("the scrape crashed: {join_error}"));
                            summary
                        }
                        Err(_) => {
                            let mut summary = RunSummary::default();
                            summary.errors.push(format!("the scrape was stopped after {} minutes", timeout.as_secs() / 60));
                            summary
                        }
                    };
                    println!("Scrape finished in {:.0?}", started.elapsed());
                    busy.store(false, Ordering::Release);
                    let _ = reply.send(summary);
                }
            });
        })
        .expect("spawn scraper thread");

    handle
}
