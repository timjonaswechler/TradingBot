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

    let fast      = indicators::sma(candles, FAST);
    let slow      = indicators::sma(candles, SLOW);
    let fast_prev = indicators::sma(candles, FAST, 1);
    let slow_prev = indicators::sma(candles, SLOW, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return decision::hold().with_reason("warming up");
    }

    let crossed_up   = fast_prev <= slow_prev && fast > slow;
    let crossed_down = fast_prev >= slow_prev && fast < slow;
    let position = context.portfolio.position;

    if crossed_up && position == () {
        return decision::open_long(QUANTITY)
            .with_stop_loss(current.low * 0.98)
            .with_take_profit(current.close * 1.10)
            .with_reason("fast SMA crossed above slow SMA");
    }

    if crossed_down && position != () && position.side == "Long" {
        return decision::close_long().with_reason("fast SMA crossed below slow SMA");
    }

    decision::hold()
}
