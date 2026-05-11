// name: "sma_cross_pnl"
// SMA Crossover Strategy with Trailing Stop (via State API)
// BUY  when the fast SMA crosses above the slow SMA
// SELL when the fast SMA crosses below the slow SMA OR trailing stop hit

const FAST = 20;
const SLOW = 20;

fn on_tick(candles, context) {
    // Trailing stop via state API
    // if context.has_position() {
    //     let pnl = context.position.pnl();
    //     let peak = context.state_f("peak_pnl", 0.0);
    //     if pnl > peak { context.set_state_f("peak_pnl", pnl); }
    //     if pnl < 0.9 * peak {
    //         return #{ signal: "SELL", reason: "trailing stop" };
    //     }
    // }

    let fast      = indicators::ema(candles, FAST);
    let slow      = indicators::sma(candles, SLOW);
    let fast_prev = indicators::ema(candles, FAST, 1);
    let slow_prev = indicators::sma(candles, SLOW, 1);

    // Not enough data yet
    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return #{ signal: "HOLD", reason: "warming up" };
    }

    let crossed_up   = fast_prev <= slow_prev && fast > slow;
    let crossed_down = fast_prev >= slow_prev && fast < slow;

    if crossed_up && context.position == () {
        return #{
            signal:      "BUY",
            size:        0.5,
            reason:      `SMA${FAST} crossed above SMA${SLOW}`,
        };
    }

    if crossed_down && context.position != () {
        return #{
            signal: "SELL",
            reason: `SMA${FAST} crossed below SMA${SLOW}`,
        };
    }

    #{ signal: "HOLD" }
}
