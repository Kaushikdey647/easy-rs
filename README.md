# easy-rs

Rust service that streams **stock quotes and trades** from [Alpaca](https://alpaca.markets/) over WebSocket and normalizes them on a **cold path** (Tokio + JSON decode + tick scaling). **NBBO comes straight from the feed**—there is no separate local order book, since Alpaca quotes are already top-of-book.

An optional **`tui`** feature adds a **terminal dashboard** ([ratatui](https://github.com/ratatui/ratatui)): **NBBO bid and ask** over time and quote **latency** (local receive time − exchange timestamp). The UI reads a pre-aggregated **`DashboardSnapshot`** (downsampled time buckets) so the Tokio ingest path only parses and **lock-free enqueues**; a dedicated thread owns downsampling (and Lee–Ready state used internally for the same pipeline).

License: GPL-3.0-or-later (see `Cargo.toml`).

## What it does

- Subscribes to Alpaca **v2 real-time quotes and trades** for one or more symbols.
- Maps each quote to a compact [`QuoteEvent`](src/hot_path/quote_event.rs): integer prices at 8 decimal places, whole-share sizes, symbol id, **exchange timestamp** (`ts_ns`), **local receive time** (`local_rx_ns`) for display latency, and **`ingest_mono_ns`** (monotonic nanoseconds since process start) for pipeline timing.
- Fans out each quote through [`AlpacaQuoteSink`](src/cold_path/sink.rs): optional bounded **ring** (for advanced use), optional **last-NBBO buffer** (CLI shutdown log), and/or a lock-free **`ArrayQueue<FeedMsg>`** for the TUI (**drop-newest** when full — see `EASY_RS_FEED_CAP` below).
- **Trades** are forwarded for the aggregator (Lee–Ready / internal series); they are not written into any local book.
- **CLI** on **Ctrl+C**: stops the feed and logs the **last quoted NBBO** per symbol from the buffer.
- **TUI**: live bid/ask and latency charts in the terminal; **q** or **Esc** exits; **Ctrl+C** also stops the feed.

**Note:** `Cargo.toml` also lists Polars and `ta` for optional analytics; the default binaries use Alpaca + tracing + (optional) ratatui only.

## Requirements

- **Rust** with **Edition 2024** support (for example Rust **1.85** or newer). Check with `rustc --version`.
- An Alpaca account with API keys and appropriate **market data** access. The default feed is **IEX**; **SIP** needs entitlement. See [Alpaca market data](https://docs.alpaca.markets/docs/market-data).

## Configuration: environment variables

### Alpaca API (via [`apca::ApiInfo::from_env`](https://docs.rs/apca/latest/apca/struct.ApiInfo.html))

| Variable | Required | Purpose |
|----------|----------|---------|
| `APCA_API_KEY_ID` | Yes | API key ID |
| `APCA_API_SECRET_KEY` | Yes | API secret |
| `APCA_API_BASE_URL` | Optional | REST base URL (paper vs live) |
| `APCA_API_STREAM_URL` | Optional | Trading stream URL |

If you omit the URL variables, `apca` uses its defaults (use paper vs live keys consistent with your account).

On startup, binaries load **`.env`** from the **current working directory** if it exists ([`dotenvy`](https://crates.io/crates/dotenvy)). Variables already set in your shell are **not** overwritten. Copy [`.env.example`](.env.example) to `.env` for a template (`.env` is gitignored).

### Runtime tuning

| Variable | Default | Purpose |
|----------|---------|---------|
| `EASY_RS_SYMBOLS` | `SPY` | Comma-separated tickers (trimmed, uppercased). At least one symbol required. |
| `EASY_RS_ALPACA_FEED` | `iex` | `iex` or `sip` — stream source. |
| `EASY_RS_FEED_CAP` | `65536` | TUI only: capacity of the ingest `ArrayQueue`. When full, **new events are dropped** (drop-newest) so backlog age stays bounded; the header shows a running drop count. |

`EASY_RS_RING_CAP` is only relevant if you integrate a **quote ring** yourself via [`AlpacaQuoteSink::with_ring`](src/cold_path/sink.rs); the stock CLI and TUI do **not** use a ring by default.

### Logging and tracing

Uses [`tracing-subscriber`](https://docs.rs/tracing-subscriber) with `EnvFilter`. Example:

```bash
export RUST_LOG=info # default if unset
export RUST_LOG=easy_rs=debug,apca=info
```

[`easy_rs::init_tracing()`](src/lib.rs) also supports:

| Variable | Purpose |
|----------|---------|
| `EASY_RS_TRACING_SPANS=1` | Emit span **close** events on stderr (pair with `RUST_LOG=trace`). |

With the **`tracy`** Cargo feature and `TRACY=1`, a **Tracy** layer is registered (use the Tracy profiler).

## How to run (CLI)

```bash
export APCA_API_KEY_ID="your-key-id"
export APCA_API_SECRET_KEY="your-secret"

export EASY_RS_SYMBOLS="SPY,QQQ"   # optional
export EASY_RS_ALPACA_FEED=iex     # or sip

cargo run --release
```

Stop with **Ctrl+C**. Logs include the **last NBBO** per symbol (integer tick prices).

## How to run (terminal UI)

The **`easy-rs-tui`** binary requires the **`tui`** feature. Same **`APCA_*`** and **`EASY_RS_*`** as the CLI.

```bash
export APCA_API_KEY_ID="your-key-id"
export APCA_API_SECRET_KEY="your-secret"
export EASY_RS_SYMBOLS="SPY,AAPL"   # optional

cargo run --release --features tui --bin easy-rs-tui
```

Development build:

```bash
cargo run --features tui --bin easy-rs-tui
```

**Stop:** **q** or **Esc**, or **Ctrl+C** in the terminal.

**Layout:** header (symbol, drop count); upper chart — **bid** and **ask** vs time; lower chart — **latency (ms)** vs time. Time axis is **seconds relative to the earliest bucket** in the downsampled window (~120 s over 1024 buckets). Use **Tab** / **Shift+Tab** or **←** / **→** to change symbol.

**Tracy:**

```bash
cargo run --release --features "tui,tracy" --bin easy-rs-tui
```

## Architecture (short)

1. **Cold path** — `tokio`: WebSocket, subscribe to quotes + trades; on each quote, **`Instant::now()` first**, then parse; normalize to [`QuoteEvent`](src/hot_path/quote_event.rs); [`AlpacaQuoteSink::on_quote`](src/cold_path/sink.rs) / `on_trade_msg` push compact [`FeedMsg`](src/cold_path/feed_msg.rs) to the TUI queue (or ring / last-NBBO as configured).
2. **Aggregator** — dedicated `std::thread` pops batches, time-bucket downsamples quotes ([`viz::pipeline`](src/viz/pipeline.rs)), runs Lee–Ready on trades, updates the focused-symbol heatmap internally, publishes **`Arc<DashboardSnapshot>`** via [`arc_swap`](https://docs.rs/arc-swap).
3. **ratatui** — each frame: cheap load of the snapshot; **Chart** widgets for bid/ask and latency.
4. **No hot book** — quotes are not replayed into `orderbook-rs`; use Alpaca’s NBBO directly.
5. **Registry** — [`SymbolRegistry`](src/cold_path/symbols.rs): sorted, deduped symbols with stable ids.

## Using as a library

Wire [`run_alpaca_quotes`](src/cold_path/alpaca_feed.rs) with [`AlpacaFeedConfig`](src/cold_path/alpaca_feed.rs) and an [`AlpacaQuoteSink`](src/cold_path/sink.rs):

- `AlpacaQuoteSink::headless_last_nbbo(n)` — CLI-style last quote buffer.
- `AlpacaQuoteSink::dashboard(queue, dropped)` — push quotes/trades to a shared `ArrayQueue<FeedMsg>` only (no mutex on the Tokio thread); used by the TUI binary.
- `AlpacaQuoteSink::with_ring(ring, dropped)` — optional bounded queue if you still want a producer/consumer split.

Call **`easy_rs::init_tracing()`** once if you want the same subscriber as the binaries.

## Tests

```bash
cargo test
cargo test --all-features
```

## Troubleshooting

- **Auth / subscription errors** — Keys, paper vs live URLs, and feed tier (`sip` needs SIP access).
- **Latency looks wrong** — Uses wall clock vs exchange time; sync NTP.
