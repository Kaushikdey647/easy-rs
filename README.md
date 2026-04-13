# easy-rs

Rust service that streams **stock quotes** from [Alpaca](https://alpaca.markets/) over WebSocket, normalizes them on a **cold path** (Tokio + I/O), hands them to a **hot path** (single consumer thread + lock-free queue), and maintains a **NBBO mirror** in [`orderbook-rs`](https://crates.io/crates/orderbook-rs). The design targets predictable local processing latency: the hot path avoids per-tick logging and uses a bounded queue so work stays bounded when the producer outruns the consumer.

License: GPL-3.0-or-later (see `Cargo.toml`).

## What it does

- Subscribes to Alpaca **v2 real-time quotes** for one or more symbols.
- Maps each quote to a compact [`QuoteEvent`](src/hot_path/quote_event.rs) (integer prices at 8 decimal places, whole-share sizes, symbol id).
- Pushes events through a **bounded** `crossbeam_queue::ArrayQueue`. If the queue is full, **new events are dropped** (counter tracked; see logs on shutdown).
- Applies NBBO updates into one [`DefaultOrderBook`](https://docs.rs/orderbook-rs) per symbol using **synthetic** bid/ask limit orders (stable IDs per symbol).
- On **Ctrl+C** (SIGINT), stops cleanly, joins the consumer, logs how many events were dropped, and prints a **best bid / best ask** snapshot per symbol.

**Note:** `Cargo.toml` also pulls in Polars, Plotters, and `ta` for potential analytics or visualization work. The current `easy-rs` library and binary **do not use them yet**; the running stack is Alpaca + ring + order book + tracing.

## Requirements

- **Rust** with **Edition 2024** support (for example Rust **1.85** or newer). Check with `rustc --version`.
- An Alpaca account with API keys and appropriate **market data** access. The default feed in this project is **IEX**; **SIP** requires Alpaca SIP entitlement (paid tier). See [Alpaca market data](https://docs.alpaca.markets/docs/market-data).

## Configuration: environment variables

### Alpaca API (via [`apca::ApiInfo::from_env`](https://docs.rs/apca/latest/apca/struct.ApiInfo.html))

| Variable | Required | Purpose |
|----------|----------|---------|
| `APCA_API_KEY_ID` | Yes | API key ID |
| `APCA_API_SECRET_KEY` | Yes | API secret |
| `APCA_API_BASE_URL` | Optional | REST base URL (paper vs live) |
| `APCA_API_STREAM_URL` | Optional | Trading stream URL |

If you omit the URL variables, `apca` uses its defaults (use paper vs live keys consistent with your account).

On startup, the binary loads a **`.env`** file from the **current working directory** if it exists ([`dotenvy`](https://crates.io/crates/dotenvy)). Variables already set in your shell are **not** overwritten. Copy [`.env.example`](.env.example) to `.env` for a template (`.env` is gitignored).

### Runtime tuning (`main`)

| Variable | Default | Purpose |
|----------|---------|---------|
| `EASY_RS_SYMBOLS` | `SPY` | Comma-separated tickers (trimmed, uppercased). At least one symbol required. |
| `EASY_RS_ALPACA_FEED` | `iex` | `iex` or `sip` — selects Alpaca quote stream source. |
| `EASY_RS_RING_CAP` | `4096` | Capacity of the quote ring; when full, newest quotes may be dropped. |

### Logging

Uses [`tracing-subscriber`](https://docs.rs/tracing-subscriber) with `EnvFilter`. Set log level like any `tracing` app, for example:

```bash
export RUST_LOG=info # default if unset
export RUST_LOG=easy_rs=debug,apca=info
```

## How to run

From the repository root:

```bash
export APCA_API_KEY_ID="your-key-id"
export APCA_API_SECRET_KEY="your-secret"

# Optional: symbols and feed
export EASY_RS_SYMBOLS="SPY,QQQ"
export EASY_RS_ALPACA_FEED=iex    # or sip if your account has SIP

# Optional: larger ring if you see many dropped events under load
export EASY_RS_RING_CAP=8192

cargo run --release
```

Stop with **Ctrl+C**. Watch for log lines that include `ring_dropped_events` (pressure on the ring) and `book top` (last NBBO snapshot).

For lowest local processing overhead, prefer `--release` (see release profile in `Cargo.toml`: LTO, single codegen unit).

## Architecture (short)

1. **Cold path** — `tokio` task: WebSocket connect, subscribe, JSON decode, convert Alpaca `Num` → integer ticks ([`PRICE_TICK_SCALE`](src/cold_path/ticks.rs) = `10^8`), enqueue [`QuoteEvent`](src/hot_path/quote_event.rs).
2. **Ring** — lock-free bounded queue; **drop-newest** on overflow ([`try_push_drop_newest`](src/hot_path/ring.rs)).
3. **Hot path** — dedicated thread: drain queue, update synthetic orders so the book’s top of book matches the stream ([`run_consumer`](src/hot_path/applier.rs)).
4. **Registry** — symbols are sorted and deduped; ids are stable for the session ([`SymbolRegistry`](src/cold_path/symbols.rs), [`BookRegistry`](src/orderbook_store.rs)).

Relevant crate docs in code: [`lib.rs`](src/lib.rs) (latency model sketch).

## Using as a library

Add the crate to your workspace (path or git dependency), enable a Tokio runtime in your binary, then wire:

- `SymbolRegistry`, `BookRegistry`, `new_quote_ring`, `run_consumer` (spawn the consumer thread), and `run_alpaca_quotes` with [`AlpacaFeedConfig`](src/cold_path/alpaca_feed.rs) / [`AlpacaFeedSource`](src/cold_path/alpaca_feed.rs).

The stock binary in [`src/main.rs`](src/main.rs) is the reference integration (shutdown flag, Ctrl+C handler, final snapshot).

## Tests

```bash
cargo test
```

Current tests cover symbol registry and decimal-to-tick parsing.

## Troubleshooting

- **Auth / subscription errors** — Verify keys, paper vs live URLs, and that your plan includes the feed you set (`EASY_RS_ALPACA_FEED=sip` needs SIP access).
- **High `ring_dropped_events`** — Increase `EASY_RS_RING_CAP`, reduce symbol count, or profile whether the hot path or machine is the bottleneck.
- **Empty or stale book** — Quotes must arrive and be enqueued; check `RUST_LOG=debug` for stream and handler warnings.
