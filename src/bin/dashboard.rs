//! Desktop dashboard: Alpaca quotes/trades + `egui_plot` panels.
//!
//! Run: `cargo run --features dashboard --bin easy-rs-dashboard`
//!
//! Env: same as `easy-rs` (`APCA_API_KEY_ID`, `APCA_API_SECRET_KEY`, `EASY_RS_SYMBOLS`, …).
//! Optional: `EASY_RS_FEED_CAP` — capacity of the lock-free ingest queue (default `65536`).
//! When the queue is full, **new events are dropped** (drop-newest) and a counter is shown in the UI.
//! Optional: `EASY_RS_TRACING_SPANS=1`, `TRACY=1` with `--features tracy`.

use crossbeam_queue::ArrayQueue;
use easy_rs::cold_path::time_anchor;
use easy_rs::cold_path::FeedMsg;
use easy_rs::cold_path::{run_alpaca_quotes, AlpacaFeedConfig, AlpacaFeedSource, AlpacaQuoteSink};
use easy_rs::viz::app::{dashboard_style, DashboardApp};
use easy_rs::viz::DashboardPipeline;
use easy_rs::SymbolRegistry;
use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

fn parse_feed_source() -> AlpacaFeedSource {
    match std::env::var("EASY_RS_ALPACA_FEED")
        .unwrap_or_else(|_| "iex".into())
        .to_lowercase()
        .as_str()
    {
        "sip" => AlpacaFeedSource::Sip,
        _ => AlpacaFeedSource::Iex,
    }
}

fn symbols_from_env() -> Vec<String> {
    std::env::var("EASY_RS_SYMBOLS")
        .unwrap_or_else(|_| "SPY".into())
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn feed_queue_cap() -> usize {
    std::env::var("EASY_RS_FEED_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&c| c > 0)
        .unwrap_or(65_536)
}

fn main() -> eframe::Result<()> {
    let _ = dotenvy::dotenv();
    easy_rs::init_tracing();
    time_anchor::ensure_started();

    let symbols = symbols_from_env();
    if symbols.is_empty() {
        eprintln!("EASY_RS_SYMBOLS must list at least one symbol");
        std::process::exit(1);
    }

    let registry = SymbolRegistry::new(symbols);
    let symbol_labels: Vec<String> = registry.symbols_sorted().to_vec();

    let shutdown = Arc::new(AtomicBool::new(false));
    let feed_dropped = Arc::new(AtomicU64::new(0));
    let queue = Arc::new(ArrayQueue::<FeedMsg>::new(feed_queue_cap()));
    let pipeline = DashboardPipeline::new(registry.len(), Arc::clone(&feed_dropped));
    pipeline.spawn_aggregator(Arc::clone(&queue), Arc::clone(&shutdown));

    let feed_shutdown = Arc::clone(&shutdown);
    let feed_symbols = registry.symbols_sorted().to_vec();
    let feed_registry = registry;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let shutdown_signal = Arc::clone(&feed_shutdown);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown_signal.store(true, Ordering::SeqCst);
            });

            let cfg = AlpacaFeedConfig {
                source: parse_feed_source(),
                symbols: feed_symbols,
            };

            let sink = AlpacaQuoteSink::dashboard(queue, Arc::clone(&feed_dropped));

            if let Err(e) = run_alpaca_quotes(cfg, feed_registry, sink, feed_shutdown).await {
                tracing::error!(?e, "Alpaca feed exited with error");
            }
        });
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 920.0])
            .with_title("easy-rs · market dashboard"),
        ..Default::default()
    };

    let app = DashboardApp::new(symbol_labels, pipeline, Arc::clone(&shutdown));
    let _ = eframe::run_native(
        "easy-rs dashboard",
        options,
        Box::new(|cc| {
            dashboard_style(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );

    shutdown.store(true, Ordering::SeqCst);

    Ok(())
}
