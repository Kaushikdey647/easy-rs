//! Symbol → compact `symbol_id` mapping (ColdPath).

use std::collections::HashMap;

#[derive(Debug)]
pub struct SymbolRegistry {
    /// Lexicographically sorted symbols (matches Alpaca normalization expectations).
    symbols: Vec<String>,
    id_of: HashMap<String, u16>,
}

impl SymbolRegistry {
    /// Build a registry from raw symbols (deduped, sorted).
    pub fn new(mut symbols: Vec<String>) -> Self {
        symbols.sort();
        symbols.dedup();
        let id_of: HashMap<String, u16> = symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u16))
            .collect();
        Self { symbols, id_of }
    }

    #[inline]
    pub fn id(&self, symbol: &str) -> Option<u16> {
        self.id_of.get(symbol).copied()
    }

    #[inline]
    pub fn symbols_sorted(&self) -> &[String] {
        &self.symbols
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_and_ids() {
        let r = SymbolRegistry::new(vec!["ZZZ".into(), "AAA".into(), "AAA".into()]);
        assert_eq!(r.symbols_sorted(), &["AAA", "ZZZ"]);
        assert_eq!(r.id("AAA"), Some(0));
        assert_eq!(r.id("ZZZ"), Some(1));
    }
}
