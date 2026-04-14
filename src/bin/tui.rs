//! Terminal dashboard: all symbols — NBBO, spread, last trade, latency — with tick-to-tick delta colors.
//!
//! Run: `cargo run --features tui --bin easy-rs-tui`
//!
//! Env: same as `easy-rs` (`APCA_API_KEY_ID`, `APCA_API_SECRET_KEY`, `EASY_RS_SYMBOLS`, …).
//! Optional: `EASY_RS_FEED_CAP` — capacity of the lock-free ingest queue (default `65536`).
//! When the queue is full, new events are dropped (drop-newest) and the count is shown in the header.

use chrono::Local;
use crossbeam_queue::ArrayQueue;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use easy_rs::SymbolRegistry;
use easy_rs::cold_path::FeedMsg;
use easy_rs::cold_path::time_anchor;
use easy_rs::cold_path::{AlpacaFeedConfig, AlpacaFeedSource, AlpacaQuoteSink, run_alpaca_quotes};
use easy_rs::viz::{DashboardPipeline, NbboRow};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

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

#[inline]
fn color_delta_f64(prev: f64, cur: f64, had_prev: bool, cur_ok: bool) -> Color {
    if !cur_ok {
        return Color::DarkGray;
    }
    if !had_prev {
        return Color::White;
    }
    if cur > prev {
        Color::Green
    } else if cur < prev {
        Color::Red
    } else {
        Color::White
    }
}

#[inline]
fn color_delta_u64(prev: u64, cur: u64, had_prev: bool, cur_ok: bool) -> Color {
    if !cur_ok {
        return Color::DarkGray;
    }
    if !had_prev {
        return Color::White;
    }
    if cur > prev {
        Color::Green
    } else if cur < prev {
        Color::Red
    } else {
        Color::White
    }
}

#[inline]
fn color_delta_f32(prev: f32, cur: f32, had_prev: bool, cur_ok: bool) -> Color {
    if !cur_ok {
        return Color::DarkGray;
    }
    if !had_prev {
        return Color::White;
    }
    let d = f64::from(cur) - f64::from(prev);
    if d > 0.001 {
        Color::Green
    } else if d < -0.001 {
        Color::Red
    } else {
        Color::White
    }
}

struct TuiApp {
    symbol_labels: Vec<String>,
    pipeline: Arc<DashboardPipeline>,
    shutdown: Arc<AtomicBool>,
    /// Previous frame (for green/red deltas).
    prev_rows: Vec<NbboRow>,
}

impl TuiApp {
    fn new(
        symbol_labels: Vec<String>,
        pipeline: Arc<DashboardPipeline>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        let n = symbol_labels.len();
        Self {
            symbol_labels,
            pipeline,
            shutdown,
            prev_rows: vec![NbboRow::default(); n],
        }
    }

    fn draw(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let snap = self.pipeline.snapshot.load();
        let dropped = self.pipeline.feed_dropped.load(Ordering::Relaxed);
        let clock = Local::now().format("%H:%M:%S%.3f").to_string();
        let header = format!(
            " easy-rs │ {clock} │ symbols: {} │ feed drops: {dropped} │ q quit ",
            self.symbol_labels.len()
        );

        let prev = &self.prev_rows;
        let mut rows: Vec<Row> = vec![Row::new(vec![
            Cell::from("SYMBOL")
                .style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("BID").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("B SZ").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("ASK").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("A SZ").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("MID").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("SPRD").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("SPR%").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("LAST").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("L SZ").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
            Cell::from("LAT ms").style(Style::default().fg(Color::Gray).add_modifier(Modifier::BOLD)),
        ])];

        for (i, sym) in self.symbol_labels.iter().enumerate() {
            let cur = snap.symbols.get(i).cloned().unwrap_or_default();
            let p = prev.get(i).cloned().unwrap_or_default();

            let mid = if cur.valid {
                (cur.bid + cur.ask) / 2.0
            } else {
                f64::NAN
            };
            let mid_ok = cur.valid && mid.is_finite();
            let mid_prev = if p.valid {
                (p.bid + p.ask) / 2.0
            } else {
                f64::NAN
            };
            let mid_prev_ok = p.valid && mid_prev.is_finite();

            let spr_pct_ok = cur.valid && cur.spread_pct.is_finite();
            let spr_pct_prev_ok = p.valid && p.spread_pct.is_finite();

            let last_ok = cur.last_sz > 0;
            let last_prev_ok = p.last_sz > 0;

            let lat_ok = cur.valid;
            let lat_prev_ok = p.valid;

            let dash = "—";
            let bid_s = if cur.valid {
                format!("{:>10.4}", cur.bid)
            } else {
                dash.to_string()
            };
            let ask_s = if cur.valid {
                format!("{:>10.4}", cur.ask)
            } else {
                dash.to_string()
            };
            let mid_s = if mid_ok {
                format!("{:>10.4}", mid)
            } else {
                dash.to_string()
            };
            let spr_s = if cur.valid {
                format!("{:>9.4}", cur.spread_abs)
            } else {
                dash.to_string()
            };
            let pct_s = if spr_pct_ok {
                format!("{:>7.4}%", cur.spread_pct)
            } else {
                dash.to_string()
            };
            let last_s = if last_ok {
                format!("{:>10.4}", cur.last_px)
            } else {
                dash.to_string()
            };
            let lat_s = if lat_ok {
                format!("{:>8.2}", cur.lat_ms)
            } else {
                dash.to_string()
            };

            rows.push(Row::new(vec![
                Cell::from(sym.as_str()),
                Cell::from(bid_s).style(Style::default().fg(color_delta_f64(
                    p.bid, cur.bid, p.valid, cur.valid,
                ))),
                Cell::from(if cur.valid {
                    format!("{:>6}", cur.bid_sz)
                } else {
                    dash.to_string()
                })
                .style(Style::default().fg(color_delta_u64(
                    p.bid_sz,
                    cur.bid_sz,
                    p.valid,
                    cur.valid,
                ))),
                Cell::from(ask_s).style(Style::default().fg(color_delta_f64(
                    p.ask, cur.ask, p.valid, cur.valid,
                ))),
                Cell::from(if cur.valid {
                    format!("{:>6}", cur.ask_sz)
                } else {
                    dash.to_string()
                })
                .style(Style::default().fg(color_delta_u64(
                    p.ask_sz,
                    cur.ask_sz,
                    p.valid,
                    cur.valid,
                ))),
                Cell::from(mid_s).style(Style::default().fg(color_delta_f64(
                    mid_prev,
                    mid,
                    mid_prev_ok,
                    mid_ok,
                ))),
                Cell::from(spr_s).style(Style::default().fg(color_delta_f64(
                    p.spread_abs,
                    cur.spread_abs,
                    p.valid,
                    cur.valid,
                ))),
                Cell::from(pct_s).style(Style::default().fg(color_delta_f64(
                    p.spread_pct,
                    cur.spread_pct,
                    spr_pct_prev_ok,
                    spr_pct_ok,
                ))),
                Cell::from(last_s).style(Style::default().fg(color_delta_f64(
                    p.last_px,
                    cur.last_px,
                    last_prev_ok,
                    last_ok,
                ))),
                Cell::from(if last_ok {
                    format!("{:>6}", cur.last_sz)
                } else {
                    dash.to_string()
                })
                .style(Style::default().fg(color_delta_u64(
                    p.last_sz,
                    cur.last_sz,
                    last_prev_ok,
                    last_ok,
                ))),
                Cell::from(lat_s).style(Style::default().fg(color_delta_f32(
                    p.lat_ms,
                    cur.lat_ms,
                    lat_prev_ok,
                    lat_ok,
                ))),
            ]));
        }

        let widths = [
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
        ];

        let table = Table::new(rows, widths).block(Block::bordered().title(" Live NBBO "));

        terminal.draw(|frame| {
            let areas = Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                .split(frame.area());
            frame.render_widget(
                Paragraph::new(header).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                areas[0],
            );
            frame.render_widget(table, areas[1]);
        })?;

        self.prev_rows = snap.symbols.clone();
        Ok(())
    }
}

fn run_tui(app: &mut TuiApp, terminal: &mut DefaultTerminal) -> io::Result<()> {
    loop {
        app.draw(terminal)?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        app.shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if app.shutdown.load(Ordering::Relaxed) {
            break;
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
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

            let subscribe_bars = std::env::var("EASY_RS_ALPACA_BARS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);

            let cfg = AlpacaFeedConfig {
                source: parse_feed_source(),
                symbols: feed_symbols,
                subscribe_bars,
            };

            let sink = AlpacaQuoteSink::dashboard(queue, Arc::clone(&feed_dropped));

            if let Err(e) = run_alpaca_quotes(cfg, feed_registry, sink, feed_shutdown).await {
                tracing::error!(?e, "Alpaca feed exited with error");
            }
        });
    });

    let mut terminal = ratatui::try_init()?;
    let mut app = TuiApp::new(symbol_labels, pipeline, Arc::clone(&shutdown));
    let run_result = run_tui(&mut app, &mut terminal);
    ratatui::try_restore()?;

    shutdown.store(true, Ordering::SeqCst);
    run_result
}
