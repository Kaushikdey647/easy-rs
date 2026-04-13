//! Cold-side registry of `Arc<DefaultOrderBook>` for reads (HTTP / metrics) without touching HotPath logic.

use orderbook_rs::DefaultOrderBook;
use std::sync::Arc;

/// One shared `Arc` per symbol index (same order as [`crate::cold_path::symbols::SymbolRegistry`]).
#[derive(Clone)]
pub struct BookRegistry {
    books: Arc<Vec<Arc<DefaultOrderBook>>>,
}

impl BookRegistry {
    /// Build empty order books for each symbol string (sorted lexicographically to match the registry).
    pub fn from_sorted_symbols(symbols: &[String]) -> Self {
        let books: Vec<_> = symbols
            .iter()
            .map(|s| Arc::new(DefaultOrderBook::new(s.as_str())))
            .collect();
        Self {
            books: Arc::new(books),
        }
    }

    #[inline]
    pub fn books(&self) -> &[Arc<DefaultOrderBook>] {
        self.books.as_slice()
    }

    #[inline]
    pub fn book_by_id(&self, symbol_id: u16) -> Option<&Arc<DefaultOrderBook>> {
        self.books.get(symbol_id as usize)
    }
}
