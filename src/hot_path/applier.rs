//! Single-writer NBBO applier: stable synthetic order IDs per symbol.

use super::quote_event::QuoteEvent;
use super::ring::QuoteRing;
use crossbeam_queue::ArrayQueue;
use orderbook_rs::{DefaultOrderBook, OrderBookError};
use pricelevel::{Id, OrderUpdate, Price, Quantity, Side, TimeInForce};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct SymbolSynth {
    bid_id: Id,
    ask_id: Id,
    placed: bool,
}

impl SymbolSynth {
    fn new() -> Self {
        Self {
            bid_id: Id::new(),
            ask_id: Id::new(),
            placed: false,
        }
    }

    fn apply(&mut self, book: &DefaultOrderBook, ev: QuoteEvent) -> Result<(), orderbook_rs::OrderBookError> {
        if !self.placed {
            book.add_limit_order(
                self.bid_id,
                ev.bid_px_ticks as u128,
                ev.bid_sz,
                Side::Buy,
                TimeInForce::Gtc,
                None,
            )?;
            book.add_limit_order(
                self.ask_id,
                ev.ask_px_ticks as u128,
                ev.ask_sz,
                Side::Sell,
                TimeInForce::Gtc,
                None,
            )?;
            self.placed = true;
            return Ok(());
        }

        Self::update_price_qty(book, self.bid_id, ev.bid_px_ticks, ev.bid_sz)?;
        Self::update_price_qty(book, self.ask_id, ev.ask_px_ticks, ev.ask_sz)?;
        Ok(())
    }

    fn update_price_qty(
        book: &DefaultOrderBook,
        order_id: Id,
        px_ticks: u64,
        qty: u64,
    ) -> Result<(), OrderBookError> {
        let upd = OrderUpdate::UpdatePriceAndQuantity {
            order_id,
            new_price: Price::new(px_ticks as u128),
            new_quantity: Quantity::new(qty),
        };
        match book.update_order(upd) {
            Ok(_) => Ok(()),
            Err(OrderBookError::InvalidOperation { .. }) => {
                let qonly = OrderUpdate::UpdateQuantity {
                    order_id,
                    new_quantity: Quantity::new(qty),
                };
                book.update_order(qonly).map(|_| ())
            }
            Err(e) => Err(e),
        }
    }
}

/// Drain the ring until empty, then spin briefly. Exits when `shutdown` is true **and** queue is empty.
pub fn run_consumer(queue: QuoteRing, books: Vec<Arc<DefaultOrderBook>>, shutdown: Arc<AtomicBool>) {
    let mut synth: Vec<Option<SymbolSynth>> = (0..books.len()).map(|_| Some(SymbolSynth::new())).collect();

    loop {
        let drained = drain_batch(&queue, &books, &mut synth);
        if shutdown.load(Ordering::Acquire) && queue.is_empty() && !drained {
            break;
        }
        if !drained {
            std::thread::yield_now();
        }
    }
}

fn drain_batch(
    queue: &Arc<ArrayQueue<QuoteEvent>>,
    books: &[Arc<DefaultOrderBook>],
    synth: &mut [Option<SymbolSynth>],
) -> bool {
    let mut any = false;
    while let Some(ev) = queue.pop() {
        any = true;
        let idx = ev.symbol_id as usize;
        if idx >= books.len() {
            continue;
        }
        if let Some(st) = synth.get_mut(idx).and_then(|s| s.as_mut()) {
            if let Err(e) = st.apply(&books[idx], ev) {
                tracing::error!(symbol_id = ev.symbol_id, ?e, "book apply error");
            }
        }
    }
    any
}
