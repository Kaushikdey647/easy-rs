//! Terminal dashboard: NBBO bid/ask and quote latency (ratatui).
//!
//! Run: `cargo run --features tui --bin easy-rs-tui`
//!
//! Env: same as `easy-rs` (`APCA_API_KEY_ID`, `APCA_API_SECRET_KEY`, `EASY_RS_SYMBOLS`, …).
//! Optional: `EASY_RS_FEED_CAP` — capacity of the lock-free ingest queue (default `65536`).
//! When the queue is full, new events are dropped (drop-newest) and the count is shown in the header.

use crossbeam_queue::ArrayQueue;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use easy_rs::SymbolRegistry;
use easy_rs::cold_path::FeedMsg;
use easy_rs::cold_path::time_anchor;
use easy_rs::cold_path::{AlpacaFeedConfig, AlpacaFeedSource, AlpacaQuoteSink, run_alpaca_quotes};
use easy_rs::viz::DashboardPipeline;
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, LegendPosition, Paragraph};
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

struct TuiApp {
    symbol_labels: Vec<String>,
    pipeline: Arc<DashboardPipeline>,
    shutdown: Arc<AtomicBool>,
    /// Focused symbol index in `symbol_labels`.
    selected: usize,
    bid_pts: Vec<(f64, f64)>,
    ask_pts: Vec<(f64, f64)>,
    lat_pts: Vec<(f64, f64)>,
}

impl TuiApp {
    fn new(
        symbol_labels: Vec<String>,
        pipeline: Arc<DashboardPipeline>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        pipeline.heatmap_focus_symbol.store(0, Ordering::Relaxed);
        Self {
            symbol_labels,
            pipeline,
            shutdown,
            selected: 0,
            bid_pts: Vec::new(),
            ask_pts: Vec::new(),
            lat_pts: Vec::new(),
        }
    }

    fn cycle_symbol(&mut self, delta: isize) {
        let n = self.symbol_labels.len();
        if n == 0 {
            return;
        }
        let i = self.selected as isize + delta;
        let i = i.rem_euclid(n as isize) as usize;
        self.selected = i;
        self.pipeline
            .heatmap_focus_symbol
            .store(u16::try_from(i).unwrap_or(0), Ordering::Relaxed);
    }

    fn draw(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        terminal.draw(|frame| {
            let snap = self.pipeline.snapshot.load();
            let sym = snap
                .symbols
                .get(self.selected)
                .cloned()
                .unwrap_or_default();

            self.bid_pts.clear();
            self.ask_pts.clear();
            self.lat_pts.clear();
            for i in 0..sym.t_rel.len() {
                let t = sym.t_rel[i];
                if i < sym.bid.len() {
                    self.bid_pts.push((t, sym.bid[i]));
                }
                if i < sym.ask.len() {
                    self.ask_pts.push((t, sym.ask[i]));
                }
                if i < sym.lat_ms.len() {
                    self.lat_pts.push((t, f64::from(sym.lat_ms[i])));
                }
            }

            let dropped = self.pipeline.feed_dropped.load(Ordering::Relaxed);
            let sym_name = self
                .symbol_labels
                .get(self.selected)
                .map(String::as_str)
                .unwrap_or("—");

            let header = format!(
                " easy-rs │ {sym_name} │ feed drops (queue full): {dropped} │ Tab/←/→ symbol │ q quit "
            );
            let areas = Layout::vertical([
                Constraint::Length(1),
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(frame.area());

            frame.render_widget(
                Paragraph::new(header).style(Style::default().bg(Color::DarkGray).fg(Color::White)),
                areas[0],
            );

            if self.bid_pts.is_empty() && self.ask_pts.is_empty() {
                let empty = Paragraph::new("Waiting for quotes…".dark_gray())
                    .block(Block::bordered().title(" Bid / Ask (NBBO) ").border_style(Style::default().fg(Color::DarkGray)));
                frame.render_widget(empty, areas[1]);
            } else {
                let (x_min, x_max) = axis_range_x(&self.bid_pts, &self.ask_pts);
                let (y_min, y_max) = axis_range_y_price(&self.bid_pts, &self.ask_pts);
                let x_labels = three_labels(x_min, x_max, |v| format!("{v:.1}"));
                let y_labels = three_labels(y_min, y_max, |v| format!("{v:.4}"));

                let spread_chart = Chart::new(vec![
                    Dataset::default()
                        .name("bid")
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(Color::Green))
                        .data(&self.bid_pts),
                    Dataset::default()
                        .name("ask")
                        .marker(Marker::Braille)
                        .graph_type(GraphType::Line)
                        .style(Style::default().fg(Color::Red))
                        .data(&self.ask_pts),
                ])
                .block(Block::bordered().title(" Bid / Ask (NBBO) "))
                .x_axis(
                    Axis::default()
                        .title("t − t0 (s)".fg(Color::Gray))
                        .style(Style::default().fg(Color::DarkGray))
                        .bounds([x_min, x_max])
                        .labels(x_labels),
                )
                .y_axis(
                    Axis::default()
                        .title("px".fg(Color::Gray))
                        .style(Style::default().fg(Color::DarkGray))
                        .bounds([y_min, y_max])
                        .labels(y_labels),
                )
                .legend_position(Some(LegendPosition::TopRight));
                frame.render_widget(spread_chart, areas[1]);
            }

            if self.lat_pts.is_empty() {
                let empty = Paragraph::new("Waiting for quotes…".dark_gray()).block(
                    Block::bordered()
                        .title(" Latency (local_rx − exchange ts) ")
                        .border_style(Style::default().fg(Color::DarkGray)),
                );
                frame.render_widget(empty, areas[2]);
            } else {
                let (x_min, x_max) = axis_range_x_lat(&self.lat_pts);
                let (y_min, y_max) = axis_range_y_scalar(&self.lat_pts);
                let x_labels = three_labels(x_min, x_max, |v| format!("{v:.1}"));
                let y_labels = three_labels(y_min, y_max, |v| format!("{v:.2}"));

                let lat_chart = Chart::new(vec![Dataset::default()
                    .name("ms")
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(Color::Magenta))
                    .data(&self.lat_pts)])
                .block(Block::bordered().title(" Latency (ms) "))
                .x_axis(
                    Axis::default()
                        .title("t − t0 (s)".fg(Color::Gray))
                        .style(Style::default().fg(Color::DarkGray))
                        .bounds([x_min, x_max])
                        .labels(x_labels),
                )
                .y_axis(
                    Axis::default()
                        .title("ms".fg(Color::Gray))
                        .style(Style::default().fg(Color::DarkGray))
                        .bounds([y_min, y_max])
                        .labels(y_labels),
                )
                .legend_position(Some(LegendPosition::TopRight));
                frame.render_widget(lat_chart, areas[2]);
            }
        })?;
        Ok(())
    }
}

fn axis_range_x(bid: &[(f64, f64)], ask: &[(f64, f64)]) -> (f64, f64) {
    let mut xs: Vec<f64> = bid
        .iter()
        .map(|p| p.0)
        .chain(ask.iter().map(|p| p.0))
        .collect();
    if xs.is_empty() {
        return (0.0, 1.0);
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo = xs[0];
    let hi = *xs.last().unwrap();
    pad_range(lo, hi, 0.02)
}

fn axis_range_x_lat(lat: &[(f64, f64)]) -> (f64, f64) {
    if lat.is_empty() {
        return (0.0, 1.0);
    }
    let lo = lat.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let hi = lat.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    pad_range(lo, hi, 0.02)
}

fn axis_range_y_price(bid: &[(f64, f64)], ask: &[(f64, f64)]) -> (f64, f64) {
    let ys: Vec<f64> = bid
        .iter()
        .map(|p| p.1)
        .chain(ask.iter().map(|p| p.1))
        .collect();
    if ys.is_empty() {
        return (0.0, 1.0);
    }
    let lo = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    pad_range(lo, hi, 0.001)
}

fn axis_range_y_scalar(pts: &[(f64, f64)]) -> (f64, f64) {
    if pts.is_empty() {
        return (0.0, 1.0);
    }
    let lo = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let hi = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    pad_range(lo, hi, 0.08)
}

fn pad_range(lo: f64, hi: f64, frac: f64) -> (f64, f64) {
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if (hi - lo).abs() < 1e-12 {
        return (lo - 0.5, hi + 0.5);
    }
    let pad = (hi - lo) * frac;
    (lo - pad, hi + pad)
}

fn three_labels<F: Fn(f64) -> String>(
    lo: f64,
    hi: f64,
    fmt: F,
) -> [ratatui::text::Line<'static>; 3] {
    let mid = (lo + hi) / 2.0;
    [fmt(lo).into(), fmt(mid).into(), fmt(hi).into()]
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
                    KeyCode::Tab => app.cycle_symbol(1),
                    KeyCode::BackTab => app.cycle_symbol(-1),
                    KeyCode::Right => app.cycle_symbol(1),
                    KeyCode::Left => app.cycle_symbol(-1),
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

    let mut terminal = ratatui::try_init()?;
    let mut app = TuiApp::new(symbol_labels, pipeline, Arc::clone(&shutdown));
    let run_result = run_tui(&mut app, &mut terminal);
    ratatui::try_restore()?;

    shutdown.store(true, Ordering::SeqCst);
    run_result
}
