// name: "sma_cross"
// Typed Runtime SMA crossover strategy.
// Opens a long position when the fast SMA crosses above the slow SMA and closes
// it when the fast SMA crosses back below the slow SMA.

const FAST = 50;
const SLOW = 200;
const QUANTITY = 1.0;

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1d"))
        .with_minimum_warmup(SLOW + 1)
}

fn on_tick(market, context) {
    let candles = market.candles();
    let current = market.candle();

    let fast      = ta::sma(candles, FAST);
    let slow      = ta::sma(candles, SLOW);
    let fast_prev = ta::sma(candles, FAST, 1);
    let slow_prev = ta::sma(candles, SLOW, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return decision::hold().with_reason("warming up");
    }

    let crossed_up   = ta::cross_over(fast_prev, slow_prev, fast, slow);
    let crossed_down = ta::cross_under(fast_prev, slow_prev, fast, slow);

    if crossed_up && context.portfolio.is_flat() {
        return decision::open_long(QUANTITY)
            .with_stop_loss(current.low * 0.98)
            .with_take_profit(current.close * 1.10)
            .with_reason("fast SMA crossed above slow SMA");
    }

    if crossed_down && context.portfolio.is_long() {
        return decision::close_long().with_reason("fast SMA crossed below slow SMA");
    }

    decision::hold()
}
