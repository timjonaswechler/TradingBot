// STUB for paper_trading::engine — replace with full implementation when merged

#[derive(Debug, Clone, PartialEq)]
pub enum TradeSide {
    Buy,
    Sell,
    Short,
    Cover,
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub side: TradeSide,
    pub quantity: i64,
    pub price_cents: i64,
    pub entry_price_cents: i64,
    pub timestamp: i64,
    pub pnl_cents: i64,
    pub commission_cents: i64,
}
