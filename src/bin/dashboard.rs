//! Desktop dashboard: Alpaca quotes/trades + `egui_plot` panels.
//!
//! Run: `cargo run --features dashboard --bin easy-rs-dashboard`
//!
//! Env: same as `easy-rs` (`APCA_API_KEY_ID`, `APCA_API_SECRET_KEY`, `EASY_RS_SYMBOLS`, …).
//! Optional: `EASY_RS_TRACING_SPANS=1`, `TRACY=1` with `--features tracy`.

use easy_rs::cold_path::{run_alpaca_quotes, AlpacaFeedConfig, AlpacaFeedSource};
use easy_rs::hot_path::ring::new_quote_ring;
use easy_rs::hot_path::run_consumer;
use easy_rs::viz::app::DashboardApp;
use easy_rs::viz::VizStore;
use easy_rs::{BookRegistry, SymbolRegistry};
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

fn ring_capacity() -> usize {
    std::env::var("EASY_RS_RING_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096)
}

fn main() -> eframe::Result<()> {
    let _ = dotenvy::dotenv();
    easy_rs::init_tracing();

    let symbols = symbols_from_env();
    if symbols.is_empty() {
        eprintln!("EASY_RS_SYMBOLS must list at least one symbol");
        std::process::exit(1);
    }

    let registry = SymbolRegistry::new(symbols);
    let symbol_labels: Vec<String> = registry.symbols_sorted().to_vec();
    let book_registry = BookRegistry::from_sorted_symbols(registry.symbols_sorted());
    let consumer_books: Vec<_> = book_registry.books().to_vec();

    let ring = new_quote_ring(ring_capacity());
    let shutdown = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicU64::new(0));
    let viz = VizStore::new(registry.len());

    let hot_ring = Arc::clone(&ring);
    let hot_shutdown = Arc::clone(&shutdown);
    let hot_handle = std::thread::spawn(move || {
        run_consumer(hot_ring, consumer_books, hot_shutdown);
    });

    let feed_shutdown = Arc::clone(&shutdown);
    let feed_ring = Arc::clone(&ring);
    let feed_dropped = Arc::clone(&dropped);
    let feed_viz = Arc::clone(&viz);
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

            if let Err(e) = run_alpaca_quotes(
                cfg,
                feed_registry,
                feed_ring,
                feed_dropped,
                feed_shutdown,
                Some(feed_viz),
            )
            .await
            {
                tracing::error!(?e, "Alpaca feed exited with error");
            }
        });
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 900.0])
            .with_title("easy-rs market dashboard"),
        ..Default::default()
    };

    let app = DashboardApp::new(symbol_labels, viz, Arc::clone(&shutdown));
    let _ = eframe::run_native(
        "easy-rs dashboard",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    );

    shutdown.store(true, Ordering::SeqCst);
    let _ = hot_handle.join();

    Ok(())
}
