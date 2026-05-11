// name: "trendline_break"
// Trendline Breakout Strategy
//
// Uses anchored indicators:
//   - Pivot detector (left=5, right=5) feeds a 6-slot ring of pivot highs / lows.
//   - Trendline evaluators fit 3+-touch resistance and support lines on the ring.
//
// Entry:
//   BUY   when close breaks above the top resistance line.
//   SHORT when close breaks below the top support line.
// Exit:
//   Managed via stop_loss (opposite line) and take_profit (risk:reward 2:1).
//
// The anchored pipeline only recomputes on each new confirmed pivot, so
// engine cost is event-driven, not per-bar.

const PIVOT_LEFT     = 5;
const PIVOT_RIGHT    = 5;
const PIVOT_BUFFER   = 6;
const TOLERANCE      = 0.002;  // 0.2% touch tolerance
const MIN_TOUCHES    = 3;
const MAX_LINES      = 1;
const RR_RATIO       = 2.0;

fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot", left: PIVOT_LEFT, right: PIVOT_RIGHT },
        ],
        evaluators: [
            #{
                expose_as:    "resistance",
                kind:         "trendline",
                side:         "resistance",
                pivot_source: "p",
                pivot_buffer: PIVOT_BUFFER,
                tolerance:    TOLERANCE,
                min_touches:  MIN_TOUCHES,
                max_lines:    MAX_LINES,
            },
            #{
                expose_as:    "support",
                kind:         "trendline",
                side:         "support",
                pivot_source: "p",
                pivot_buffer: PIVOT_BUFFER,
                tolerance:    TOLERANCE,
                min_touches:  MIN_TOUCHES,
                max_lines:    MAX_LINES,
            },
        ],
    }
}

fn on_tick(candles, context) {
    let c   = candles[1];
    if c == () { return #{ signal: "HOLD" }; }

    let res = context.anchored("resistance");
    let sup = context.anchored("support");

    let has_res = type_of(res) == "array" && res.len() > 0;
    let has_sup = type_of(sup) == "array" && sup.len() > 0;

    // Exit logic: if in a position, close on opposite break.
    if context.position != () {
        if context.position.side == "Long" && has_sup && c.close < sup[0].y_at(c.bar) {
            return #{ signal: "SELL", reason: "support broken — long exit" };
        }
        if context.position.side == "Short" && has_res && c.close > res[0].y_at(c.bar) {
            return #{ signal: "COVER", reason: "resistance broken — short exit" };
        }
        return #{ signal: "HOLD" };
    }

    // Entry logic: breakout in either direction.
    if has_res {
        let r    = res[0];
        let r_y  = r.y_at(c.bar);
        if c.close > r_y && has_sup {
            let s_y  = sup[0].y_at(c.bar);
            let risk = c.close - s_y;
            if risk > 0.0 {
                let tp = c.close + risk * RR_RATIO;
                return #{
                    signal:      "BUY",
                    size:        0.5,
                    stop_loss:   s_y,
                    take_profit: tp,
                    reason:      "break above resistance",
                };
            }
        }
    }

    if has_sup {
        let s    = sup[0];
        let s_y  = s.y_at(c.bar);
        if c.close < s_y && has_res {
            let r_y  = res[0].y_at(c.bar);
            let risk = r_y - c.close;
            if risk > 0.0 {
                let tp = c.close - risk * RR_RATIO;
                return #{
                    signal:      "SHORT",
                    size:        0.5,
                    stop_loss:   r_y,
                    take_profit: tp,
                    reason:      "break below support",
                };
            }
        }
    }

    #{ signal: "HOLD" }
}
