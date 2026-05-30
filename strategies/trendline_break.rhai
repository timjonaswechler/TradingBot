// name: "trendline_break"
// Typed Runtime trendline breakout strategy.
// Uses anchored pivot-derived resistance exposed through `market`.

const PIVOT_LEFT = 5;
const PIVOT_RIGHT = 5;
const PIVOT_BUFFER = 6;
const TOLERANCE = 0.002;
const MIN_TOUCHES = 3;
const MAX_LINES = 1;
const QUANTITY = 1.0;

fn anchored_config() {
    anchored_config::new()
        .with_detector(
            pivot_detector::new("p")
                .with_left_bars(PIVOT_LEFT)
                .with_right_bars(PIVOT_RIGHT)
        )
        .with_evaluator(
            anchored::trendline("resistance", "p")
                .with_side(pivot_side::high())
                .with_pivot_buffer(PIVOT_BUFFER)
                .with_tolerance(TOLERANCE)
                .with_min_touches(MIN_TOUCHES)
                .with_max_lines(MAX_LINES)
        )
}

fn on_tick(market, context) {
    let c = market.candle();
    let current_bar = market.candles().len() - 1;
    let lines = market.anchored("resistance");
    let has_lines = type_of(lines) == "array" && lines.len() > 0;
    let position = context.portfolio.position;

    if position != () {
        return decision::hold();
    }

    if has_lines {
        let line = lines[0];
        let y = line.y_at(current_bar);
        if c.close > y {
            return decision::open_long(QUANTITY).with_reason("break above resistance");
        }
    }

    decision::hold()
}
