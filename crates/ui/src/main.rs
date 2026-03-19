use bot::{
    config::Config,
    db::Db,
    market_data::Candle,
    paper_trading::{PaperTradingEngine, TradeSide},
    strategy::Signal,
};
use eframe::egui::{self, Color32, RichText, Stroke};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotPoints, Points, VLine};
use std::path::Path;

// ── Strategie-Parameter (editierbar in der UI) ────────────────────────────────

#[derive(Clone)]
struct StrategyParams {
    name: String,
    // SMA
    short_period: usize,
    long_period:  usize,
    // RSI
    rsi_period:     usize,
    rsi_oversold:   f64,
    rsi_overbought: f64,
    // MACD
    macd_fast:   usize,
    macd_slow:   usize,
    macd_signal: usize,
    // Bollinger
    bb_period: usize,
    bb_k:      f64,
}

impl StrategyParams {
    fn from_config(cfg: &Config) -> Self {
        let s = &cfg.strategy;
        Self {
            name:           s.name.clone(),
            short_period:   s.short_period,
            long_period:    s.long_period,
            rsi_period:     s.rsi_period.unwrap_or(14),
            rsi_oversold:   s.rsi_oversold.unwrap_or(30.0),
            rsi_overbought: s.rsi_overbought.unwrap_or(70.0),
            macd_fast:      s.macd_fast.unwrap_or(12),
            macd_slow:      s.macd_slow.unwrap_or(26),
            macd_signal:    s.macd_signal.unwrap_or(9),
            bb_period:      s.bb_period.unwrap_or(20),
            bb_k:           s.bb_k.unwrap_or(2.0),
        }
    }

    fn to_strategy(&self) -> anyhow::Result<Box<dyn bot::strategy::Strategy>> {
        use bot::strategy::{bollinger::BollingerBands, macd::Macd, rsi::Rsi,
                             sma_crossover::SmaCrossover};
        let s: Box<dyn bot::strategy::Strategy> = match self.name.as_str() {
            "sma_crossover" => Box::new(SmaCrossover {
                short_period: self.short_period,
                long_period:  self.long_period,
            }),
            "rsi" => Box::new(Rsi {
                period:     self.rsi_period,
                oversold:   self.rsi_oversold,
                overbought: self.rsi_overbought,
            }),
            "macd" => Box::new(Macd {
                fast_period:   self.macd_fast,
                slow_period:   self.macd_slow,
                signal_period: self.macd_signal,
            }),
            "bollinger" => Box::new(BollingerBands {
                period: self.bb_period,
                k:      self.bb_k,
            }),
            n => anyhow::bail!("Unbekannte Strategie: {n}"),
        };
        Ok(s)
    }
}

// ── Daten-Strukturen ──────────────────────────────────────────────────────────

#[derive(Default)]
struct PortfolioData {
    positions: Vec<(String, i64, f64)>,
}

#[derive(Default)]
struct SimTrade {
    timestamp: i64,
    side:      String,
    price:     f64,
    quantity:  i64,
    gain_loss: Option<f64>,
    fee:       f64,
    tax:       Option<f64>,
}

#[derive(Default)]
struct ChartData {
    price:      Vec<[f64; 2]>,
    indicator:  Vec<[f64; 2]>,
    indicator2: Vec<[f64; 2]>,
    indicator3: Vec<[f64; 2]>,
    buys:       Vec<[f64; 2]>,
    sells:      Vec<[f64; 2]>,
    current_signal: CurrentSignal,
    // Simulierte Trades mit echten G/L-Werten
    sim_trades:           Vec<SimTrade>,
    sim_total_gl:         f64,
    sim_win:              usize,
    sim_loss:             usize,
    sim_fees:             f64,
    sim_total_tax:        f64,
    // Engine-Endstand nach Simulation
    sim_cash:             f64,
    sim_total_value:      f64,
    sim_exemption_left:   f64,
}

#[derive(Default)]
enum CurrentSignal { Buy, Sell, #[default] Hold }

// ── App ───────────────────────────────────────────────────────────────────────

struct TradingApp {
    config:         Config,
    db_path:         String,
    selected_asset:  String,
    selected_interval: String,
    params:          StrategyParams,
    portfolio:      PortfolioData,
    chart:          ChartData,
    // Chart-Ansicht
    show_indicator:  bool,
    show_indicator2: bool,
    chart_start_ts:  Option<f64>, // Unix-Timestamp Startpunkt
    chart_start_str: String,      // Eingabefeld "YYYY-MM-DD"
    // UI-State
    status:          String,
    settings_dirty:  bool,        // Parameter wurden geändert, noch nicht angewendet
}

impl TradingApp {
    fn new(config: Config) -> Self {
        let db_path           = config.db.path.clone();
        let selected_asset    = config.assets.watchlist.first().cloned().unwrap_or_default();
        let selected_interval = config.data.primary_interval().to_string();
        let params            = StrategyParams::from_config(&config);
        let mut app = Self {
            config,
            db_path,
            selected_asset,
            selected_interval,
            params,
            portfolio:       PortfolioData::default(),
            chart:           ChartData::default(),
            show_indicator:  true,
            show_indicator2: true,
            chart_start_ts:  None,
            chart_start_str: String::new(),
            status:          String::new(),
            settings_dirty:  false,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        match self.load_all() {
            Ok(_)  => self.status = format!(
                "Aktualisiert  {}",
                chrono::Local::now().format("%H:%M:%S")
            ),
            Err(e) => self.status = format!("Fehler: {e}"),
        }
    }

    fn load_all(&mut self) -> anyhow::Result<()> {
        let db = Db::open(&self.db_path)?;
        self.load_portfolio()?;
        self.load_chart(&db)?;
        Ok(())
    }

    fn load_portfolio(&mut self) -> anyhow::Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;

        let mut stmt = conn
            .prepare("SELECT asset, quantity, avg_buy_price FROM positions ORDER BY asset")?;
        let positions: Vec<(String, i64, f64)> = stmt
            .query_map([], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)? as f64 / 100.0,
            )))?
            .filter_map(|r| r.ok())
            .collect();

        self.portfolio = PortfolioData { positions };
        Ok(())
    }

    fn load_chart(&mut self, db: &Db) -> anyhow::Result<()> {
        // Gesamter Datensatz für Strategie-Berechnung
        let candles = db.get_all_candles_asc(&self.selected_asset, &self.selected_interval)?;
        if candles.is_empty() {
            self.chart = ChartData::default();
            return Ok(());
        }

        // Startpunkt nur für die Chart-Anzeige (Strategie läuft immer auf allen Daten)
        let display_start = self.chart_start_ts.unwrap_or(f64::NEG_INFINITY);

        let strat = self.params.to_strategy()?;
        let h     = strat.required_history();

        // Preislinie + Indikatoren: nur ab Startpunkt anzeigen
        let price: Vec<[f64; 2]> = candles.iter()
            .filter(|c| c.timestamp.timestamp() as f64 >= display_start)
            .map(|c| [c.timestamp.timestamp() as f64, c.close as f64 / 100.0])
            .collect();

        // Indikatoren je nach Strategie (auf allen Daten berechnen, dann filtern)
        let (ind1_all, ind2_all, ind3_all) = compute_indicators(&candles, &self.params);
        let filter_pts = |pts: Vec<[f64; 2]>| -> Vec<[f64; 2]> {
            pts.into_iter().filter(|p| p[0] >= display_start).collect()
        };
        let (ind1, ind2, ind3) = (filter_pts(ind1_all), filter_pts(ind2_all), filter_pts(ind3_all));

        let mut buys  = vec![];
        let mut sells = vec![];
        let mut current_signal = CurrentSignal::Hold;
        let mut sim_trades: Vec<SimTrade> = vec![];

        let mut engine = PaperTradingEngine::new(
            self.config.paper_trading.starting_capital,
            self.config.tax.freistellungsauftrag,
            vec![],
            self.config.costs.clone(),
            self.config.tax.clone(),
            self.config.paper_trading.position_size_pct,
        );

        if candles.len() >= h {
            for t in (h - 1)..candles.len() {
                let window: Vec<_> =
                    candles[t + 1 - h..=t].iter().rev().cloned().collect();
                let ts       = candles[t].timestamp.timestamp();
                let price_f  = candles[t].close as f64 / 100.0;
                let sig      = strat.signal(&window);

                if t == candles.len() - 1 {
                    current_signal = match sig {
                        Signal::Buy  => CurrentSignal::Buy,
                        Signal::Sell => CurrentSignal::Sell,
                        Signal::Hold => CurrentSignal::Hold,
                    };
                }

                if let Ok(Some(trade)) = engine.execute(&sig, &self.selected_asset, candles[t].close, strat.name()) {
                    let is_buy = trade.side == TradeSide::Buy;
                    // Marker nur im sichtbaren Bereich (ab Startpunkt)
                    if ts as f64 >= display_start {
                        if is_buy { buys.push([ts as f64, price_f]); }
                        else      { sells.push([ts as f64, price_f]); }
                    }
                    sim_trades.push(SimTrade {
                        timestamp: ts,
                        side:      if is_buy { "buy".into() } else { "sell".into() },
                        price:     price_f,
                        quantity:  trade.quantity,
                        gain_loss: trade.gain_loss.map(|g| g as f64 / 100.0),
                        fee:       trade.fee as f64 / 100.0,
                        tax:       trade.tax.map(|t| t as f64 / 100.0),
                    });
                }
            }
        }

        // Aggregierte Statistiken
        let sim_total_gl:  f64 = sim_trades.iter().filter_map(|t| t.gain_loss).sum();
        let sim_fees:      f64 = sim_trades.iter().map(|t| t.fee).sum();
        let sim_total_tax: f64 = sim_trades.iter().filter_map(|t| t.tax).sum();
        let sim_win  = sim_trades.iter().filter(|t| t.gain_loss.unwrap_or(0.0) > 0.0).count();
        let sim_loss = sim_trades.iter().filter(|t| t.gain_loss.unwrap_or(0.0) < 0.0).count();

        // Endstand der Engine
        let last_price = candles.last().map(|c| c.close).unwrap_or(0);
        let pos_value: f64 = engine.positions.iter()
            .map(|p| p.quantity as f64 * last_price as f64 / 100.0)
            .sum();
        let sim_cash        = engine.cash as f64 / 100.0;
        let sim_total_value = sim_cash + pos_value;
        let sim_exemption_left = engine.exemption_remaining as f64 / 100.0;

        self.chart = ChartData {
            price, indicator: ind1, indicator2: ind2, indicator3: ind3,
            buys, sells, current_signal,
            sim_trades, sim_total_gl, sim_win, sim_loss, sim_fees, sim_total_tax,
            sim_cash, sim_total_value, sim_exemption_left,
        };
        Ok(())
    }

    fn apply_params(&mut self) {
        self.settings_dirty = false;
        if let Ok(db) = Db::open(&self.db_path) {
            match self.load_chart(&db) {
                Ok(_)  => self.status = "Parameter angewendet".into(),
                Err(e) => self.status = format!("Fehler: {e}"),
            }
        }
    }

    fn select_asset(&mut self, asset: String) {
        self.selected_asset = asset;
        if let Ok(db) = Db::open(&self.db_path) {
            let _ = self.load_chart(&db);
        }
    }

    fn set_chart_start(&mut self) {
        if self.chart_start_str.is_empty() {
            self.chart_start_ts = None;
        } else if let Ok(d) = chrono::NaiveDate::parse_from_str(&self.chart_start_str, "%Y-%m-%d") {
            use chrono::TimeZone;
            self.chart_start_ts = Some(chrono::Utc.from_utc_datetime(
                &d.and_hms_opt(0, 0, 0).unwrap()
            ).timestamp() as f64);
            self.status = format!("Startpunkt: {}", d);
        } else {
            self.status = "Ungültiges Datum (YYYY-MM-DD)".into();
            return;
        }
        if let Ok(db) = Db::open(&self.db_path) {
            let _ = self.load_chart(&db);
        }
    }
}

impl eframe::App for TradingApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        // ── Toolbar ───────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("TradingBot");
                ui.separator();

                // Asset-Auswahl
                ui.label("Asset:");
                let assets = self.config.assets.watchlist.clone();
                egui::ComboBox::from_id_salt("asset_select")
                    .selected_text(&self.selected_asset)
                    .show_ui(ui, |ui| {
                        for asset in assets {
                            if ui.selectable_label(self.selected_asset == asset, &asset).clicked() {
                                self.select_asset(asset);
                            }
                        }
                    });

                // Intervall-Auswahl
                ui.label("Intervall:");
                let intervals = self.config.data.intervals.clone();
                let cur_iv = self.selected_interval.clone();
                egui::ComboBox::from_id_salt("interval_select")
                    .selected_text(&cur_iv)
                    .show_ui(ui, |ui| {
                        for iv in &intervals {
                            if ui.selectable_label(cur_iv == *iv, iv).clicked() {
                                self.selected_interval = iv.clone();
                                if let Ok(db) = Db::open(&self.db_path) {
                                    let _ = self.load_chart(&db);
                                }
                            }
                        }
                    });

                ui.separator();

                // Strategie-Auswahl
                ui.label("Strategie:");
                let strategies = ["sma_crossover", "rsi", "macd", "bollinger"];
                let cur = self.params.name.clone();
                egui::ComboBox::from_id_salt("strat_select")
                    .selected_text(strategy_display_name(&cur))
                    .show_ui(ui, |ui| {
                        for s in strategies {
                            if ui.selectable_label(cur == s, strategy_display_name(s)).clicked() {
                                self.params.name = s.to_string();
                                self.settings_dirty = true;
                            }
                        }
                    });

                ui.separator();

                // Signal-Anzeige
                let (sig_label, sig_color) = match self.chart.current_signal {
                    CurrentSignal::Buy  => ("▲ BUY",  Color32::from_rgb(60, 210, 90)),
                    CurrentSignal::Sell => ("▼ SELL", Color32::from_rgb(220, 60, 60)),
                    CurrentSignal::Hold => ("● HOLD", Color32::from_rgb(160, 160, 160)),
                };
                ui.label(
                    RichText::new(sig_label).color(sig_color).strong().size(15.0)
                );

                ui.separator();

                if ui.button("⟳  Refresh").clicked() {
                    self.refresh();
                }

                ui.separator();

                // Portfolio-Stats
                let start    = self.config.paper_trading.starting_capital as f64 / 100.0;
                let gl       = self.chart.sim_total_value - start;
                let gl_color = if gl >= 0.0 { Color32::from_rgb(100, 220, 100) }
                               else          { Color32::from_rgb(220, 80, 80) };

                ui.label(RichText::new(format!("Cash: {:.2} €", self.chart.sim_cash)).strong());
                ui.label(RichText::new(format!("Gesamt: {:.2} €", self.chart.sim_total_value)).strong());
                ui.label(RichText::new(format!("G/L: {:+.2} €", gl)).color(gl_color).strong());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(&self.status).weak().small());
                });
            });
        });

        // ── Rechtes Panel ─────────────────────────────────────────────────────
        egui::SidePanel::right("sidebar")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {

                    // ── Strategie-Einstellungen ───────────────────────────────
                    ui.collapsing("⚙  Strategie-Parameter", |ui| {
                        let mut dirty = false;
                        egui::Grid::new("params").spacing([8.0, 6.0]).show(ui, |ui| {
                            match self.params.name.as_str() {
                                "sma_crossover" => {
                                    ui.label("Short SMA");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.short_period)
                                        .range(2..=200).speed(1)).changed();
                                    ui.end_row();
                                    ui.label("Long SMA");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.long_period)
                                        .range(2..=500).speed(1)).changed();
                                    ui.end_row();
                                }
                                "rsi" => {
                                    ui.label("Periode");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.rsi_period)
                                        .range(2..=100).speed(1)).changed();
                                    ui.end_row();
                                    ui.label("Oversold");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.rsi_oversold)
                                        .range(1.0..=49.0).speed(0.5)).changed();
                                    ui.end_row();
                                    ui.label("Overbought");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.rsi_overbought)
                                        .range(51.0..=99.0).speed(0.5)).changed();
                                    ui.end_row();
                                }
                                "macd" => {
                                    ui.label("Fast EMA");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.macd_fast)
                                        .range(2..=100).speed(1)).changed();
                                    ui.end_row();
                                    ui.label("Slow EMA");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.macd_slow)
                                        .range(2..=200).speed(1)).changed();
                                    ui.end_row();
                                    ui.label("Signal");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.macd_signal)
                                        .range(2..=50).speed(1)).changed();
                                    ui.end_row();
                                }
                                "bollinger" => {
                                    ui.label("Periode");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.bb_period)
                                        .range(2..=200).speed(1)).changed();
                                    ui.end_row();
                                    ui.label("Abw. (k)");
                                    dirty |= ui.add(egui::DragValue::new(&mut self.params.bb_k)
                                        .range(0.5..=5.0).speed(0.1)).changed();
                                    ui.end_row();
                                }
                                _ => {}
                            }
                        });

                        if dirty { self.settings_dirty = true; }

                        ui.add_space(4.0);
                        let btn = egui::Button::new(
                            if self.settings_dirty { "▶  Anwenden *" } else { "▶  Anwenden" }
                        );
                        if ui.add(btn).clicked() {
                            self.apply_params();
                        }

                        ui.add_space(4.0);
                        ui.separator();
                        ui.label(RichText::new("Chart Startpunkt").small().weak());
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.chart_start_str)
                                .hint_text("YYYY-MM-DD")
                                .desired_width(100.0));
                            if ui.small_button("✓").clicked() {
                                self.set_chart_start();
                            }
                            if ui.small_button("✕").clicked() {
                                self.chart_start_str.clear();
                                self.chart_start_ts = None;
                                if let Ok(db) = Db::open(&self.db_path) {
                                    let _ = self.load_chart(&db);
                                }
                            }
                        });
                        if let Some(ts) = self.chart_start_ts {
                            let d = chrono::DateTime::from_timestamp(ts as i64, 0)
                                .map(|d| d.format("%Y-%m-%d").to_string())
                                .unwrap_or_default();
                            ui.label(RichText::new(format!("ab {d}")).small().color(Color32::YELLOW));
                        }
                    });

                    ui.add_space(6.0);

                    // ── Simulations-Performance ───────────────────────────────
                    ui.collapsing("📈  Simulation Performance", |ui| {
                        let gl = self.chart.sim_total_gl;
                        let gl_color = if gl >= 0.0 { Color32::from_rgb(100, 220, 100) }
                                       else          { Color32::from_rgb(220, 80, 80) };
                        let total_closed = self.chart.sim_win + self.chart.sim_loss;
                        egui::Grid::new("sim_perf").spacing([10.0, 5.0]).show(ui, |ui| {
                            ui.label("Gesamt G/L:");
                            ui.label(RichText::new(format!("{:+.2} €", gl)).color(gl_color).strong());
                            ui.end_row();
                            ui.label("Gebühren:");
                            ui.label(format!("{:.2} €", self.chart.sim_fees));
                            ui.end_row();
                            ui.label("Gewinner:");
                            ui.label(RichText::new(format!(
                                "{}  ({:.0} %)", self.chart.sim_win,
                                if total_closed > 0 { self.chart.sim_win as f64 / total_closed as f64 * 100.0 } else { 0.0 }
                            )).color(Color32::from_rgb(100, 220, 100)));
                            ui.end_row();
                            ui.label("Verlierer:");
                            ui.label(RichText::new(format!(
                                "{}  ({:.0} %)", self.chart.sim_loss,
                                if total_closed > 0 { self.chart.sim_loss as f64 / total_closed as f64 * 100.0 } else { 0.0 }
                            )).color(Color32::from_rgb(220, 80, 80)));
                            ui.end_row();
                        });
                    });

                    ui.add_space(6.0);

                    // ── Positionen ────────────────────────────────────────────
                    ui.collapsing("📊  Positionen", |ui| {
                        if self.portfolio.positions.is_empty() {
                            ui.label(RichText::new("Keine offenen Positionen").weak());
                        } else {
                            egui::Grid::new("positions").striped(true).spacing([12.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label(RichText::new("Asset").strong());
                                    ui.label(RichText::new("Stück").strong());
                                    ui.label(RichText::new("Ø Kauf").strong());
                                    ui.end_row();
                                    for (asset, qty, price) in &self.portfolio.positions {
                                        ui.label(asset);
                                        ui.label(qty.to_string());
                                        ui.label(format!("{price:.2} €"));
                                        ui.end_row();
                                    }
                                });
                        }
                    });

                    ui.add_space(6.0);

                    // ── Simulierte Trades ─────────────────────────────────────
                    ui.collapsing(
                        format!("📋  Sim-Trades ({})", self.chart.sim_trades.len()),
                        |ui| {
                        if self.chart.sim_trades.is_empty() {
                            ui.label(RichText::new("Keine Trades im gewählten Zeitraum").weak());
                        } else {
                            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                egui::Grid::new("sim_trades").striped(true).spacing([8.0, 3.0])
                                    .show(ui, |ui| {
                                        ui.label(RichText::new("Datum").strong().small());
                                        ui.label(RichText::new("Seite").strong().small());
                                        ui.label(RichText::new("Preis").strong().small());
                                        ui.label(RichText::new("Stk").strong().small());
                                        ui.label(RichText::new("G/L").strong().small());
                                        ui.end_row();

                                        for t in self.chart.sim_trades.iter().rev() {
                                            let side_color = if t.side == "buy" {
                                                Color32::from_rgb(100, 210, 100)
                                            } else {
                                                Color32::from_rgb(210, 80, 80)
                                            };
                                            let date = chrono::DateTime::from_timestamp(t.timestamp, 0)
                                                .map(|d| d.format("%y-%m-%d").to_string())
                                                .unwrap_or_default();
                                            ui.label(RichText::new(date).small());
                                            ui.label(RichText::new(t.side.to_uppercase())
                                                .color(side_color).small());
                                            ui.label(RichText::new(format!("{:.2}", t.price)).small());
                                            ui.label(RichText::new(t.quantity.to_string()).small());
                                            match t.gain_loss {
                                                Some(g) => {
                                                    let c = if g >= 0.0 {
                                                        Color32::from_rgb(100, 210, 100)
                                                    } else {
                                                        Color32::from_rgb(210, 80, 80)
                                                    };
                                                    ui.label(RichText::new(
                                                        format!("{:+.2}", g)
                                                    ).color(c).small());
                                                }
                                                None => { ui.label(RichText::new("–").small()); }
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                        }
                    });

                    ui.add_space(6.0);

                    // ── Steuern ───────────────────────────────────────────────
                    ui.collapsing("🧾  Steuern", |ui| {
                        let total = self.config.tax.freistellungsauftrag as f64 / 100.0;
                        let used  = total - self.chart.sim_exemption_left;
                        let pct   = (used / total * 100.0).clamp(0.0, 100.0);

                        egui::Grid::new("tax").spacing([10.0, 4.0]).show(ui, |ui| {
                            ui.label("Freistellungsauftrag:");
                            ui.label(format!("{total:.2} €"));
                            ui.end_row();
                            ui.label("Verbraucht:");
                            ui.label(format!("{used:.2} € ({pct:.1} %)"));
                            ui.end_row();
                            ui.label("Verbleibend:");
                            ui.label(format!("{:.2} €", self.chart.sim_exemption_left));
                            ui.end_row();
                            ui.label("Steuern gezahlt:");
                            ui.label(format!("{:.2} €", self.chart.sim_total_tax));
                            ui.end_row();
                        });
                        ui.add(egui::ProgressBar::new(pct as f32 / 100.0)
                            .show_percentage()
                            .fill(if pct > 80.0 {
                                Color32::from_rgb(220, 80, 80)
                            } else {
                                Color32::from_rgb(80, 160, 220)
                            }));
                    });
                });
            });

        // ── Chart ─────────────────────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.chart.price.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Keine Daten – erst `cargo run -p bot` ausführen")
                        .weak().size(18.0));
                });
                return;
            }

            // Indikator-Labels je nach Strategie
            let (ind1_label, ind2_label, ind3_label) = indicator_labels(&self.params);

            // Indikator-Checkboxen in der Toolbar des Charts
            ui.horizontal(|ui| {
                if !self.chart.indicator.is_empty() {
                    ui.checkbox(&mut self.show_indicator,
                        RichText::new(&ind1_label).color(Color32::from_rgb(255, 175, 50)));
                }
                if !self.chart.indicator2.is_empty() {
                    ui.checkbox(&mut self.show_indicator2,
                        RichText::new(&ind2_label).color(Color32::from_rgb(255, 80, 80)));
                }
            });

            Plot::new("chart")
                .legend(Legend::default())
                .auto_bounds([true, true].into())
                .x_axis_formatter({
                    let iv = self.selected_interval.clone();
                    move |mark, _range| {
                        chrono::DateTime::from_timestamp(mark.value as i64, 0)
                            .map(|d| {
                                if iv.contains('m') || iv == "1h" || iv == "90m" {
                                    d.format("%m-%d %H:%M").to_string()
                                } else if iv == "1d" || iv == "5d" {
                                    d.format("%b %d '%y").to_string()
                                } else {
                                    d.format("%b %Y").to_string()
                                }
                            })
                            .unwrap_or_default()
                    }
                })
                .y_axis_formatter(|mark, _range| format!("{:.0} €", mark.value))
                .label_formatter(|name, value| {
                    let date = chrono::DateTime::from_timestamp(value.x as i64, 0)
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_default();
                    if name.is_empty() { format!("{date}\n{:.2} €", value.y) }
                    else               { format!("{name}\n{date}\n{:.2} €", value.y) }
                })
                .show(ui, |plot_ui| {
                    // Startpunkt-Markierung
                    if let Some(ts) = self.chart_start_ts {
                        plot_ui.vline(
                            VLine::new(ts)
                                .name("Startpunkt")
                                .color(Color32::from_rgba_unmultiplied(255, 255, 0, 80))
                                .width(1.5),
                        );
                    }

                    // Kurs
                    plot_ui.line(
                        Line::new(PlotPoints::new(self.chart.price.clone()))
                            .name("Kurs")
                            .color(Color32::from_rgb(80, 145, 255))
                            .stroke(Stroke::new(2.0, Color32::from_rgb(80, 145, 255))),
                    );

                    // Indikator 1
                    if self.show_indicator && !self.chart.indicator.is_empty() {
                        plot_ui.line(
                            Line::new(PlotPoints::new(self.chart.indicator.clone()))
                                .name(&ind1_label)
                                .color(Color32::from_rgb(255, 175, 50))
                                .stroke(Stroke::new(1.5, Color32::from_rgb(255, 175, 50))),
                        );
                    }

                    // Indikator 2 (Long SMA / BB upper)
                    if self.show_indicator2 && !self.chart.indicator2.is_empty() {
                        plot_ui.line(
                            Line::new(PlotPoints::new(self.chart.indicator2.clone()))
                                .name(&ind2_label)
                                .color(Color32::from_rgb(255, 80, 80))
                                .stroke(Stroke::new(1.5, Color32::from_rgb(255, 80, 80))),
                        );
                    }

                    // Indikator 3 (BB lower)
                    if self.show_indicator2 && !self.chart.indicator3.is_empty() {
                        plot_ui.line(
                            Line::new(PlotPoints::new(self.chart.indicator3.clone()))
                                .name(&ind3_label)
                                .color(Color32::from_rgb(255, 80, 80))
                                .style(egui_plot::LineStyle::dashed_loose())
                                .stroke(Stroke::new(1.0, Color32::from_rgb(255, 80, 80))),
                        );
                    }

                    // BUY
                    if !self.chart.buys.is_empty() {
                        plot_ui.points(
                            Points::new(PlotPoints::new(self.chart.buys.clone()))
                                .name("BUY")
                                .shape(MarkerShape::Up)
                                .radius(9.0)
                                .color(Color32::from_rgb(60, 210, 90))
                                .filled(true),
                        );
                    }

                    // SELL
                    if !self.chart.sells.is_empty() {
                        plot_ui.points(
                            Points::new(PlotPoints::new(self.chart.sells.clone()))
                                .name("SELL")
                                .shape(MarkerShape::Down)
                                .radius(9.0)
                                .color(Color32::from_rgb(220, 60, 60))
                                .filled(true),
                        );
                    }
                });
        });
    }
}

// ── Indikator-Berechnung ──────────────────────────────────────────────────────

fn compute_indicators(
    candles: &[Candle],
    params:  &StrategyParams,
) -> (Vec<[f64; 2]>, Vec<[f64; 2]>, Vec<[f64; 2]>) {
    match params.name.as_str() {
        "sma_crossover" => {
            let s = sma_points(candles, params.short_period);
            let l = sma_points(candles, params.long_period);
            (s, l, vec![])
        }
        "bollinger" => {
            let p = params.bb_period;
            let k = params.bb_k;
            if candles.len() < p { return (vec![], vec![], vec![]); }
            let mut mid = vec![];
            let mut upper = vec![];
            let mut lower = vec![];
            for (i, w) in candles.windows(p).enumerate() {
                let ts  = candles[i + p - 1].timestamp.timestamp() as f64;
                let mean = w.iter().map(|c| c.close as f64 / 100.0).sum::<f64>() / p as f64;
                let var  = w.iter().map(|c| {
                    let d = c.close as f64 / 100.0 - mean; d * d
                }).sum::<f64>() / p as f64;
                let std  = var.sqrt();
                mid.push([ts, mean]);
                upper.push([ts, mean + k * std]);
                lower.push([ts, mean - k * std]);
            }
            (mid, upper, lower)
        }
        // RSI und MACD haben keine direkt auf dem Preis-Chart darstellbaren Indikatoren
        // → nur SMA 20 als Orientierung anzeigen
        _ => {
            let s = sma_points(candles, 20.min(candles.len()));
            (s, vec![], vec![])
        }
    }
}

fn sma_points(candles: &[Candle], period: usize) -> Vec<[f64; 2]> {
    if candles.len() < period || period == 0 { return vec![]; }
    candles.windows(period).enumerate().map(|(i, w)| {
        let avg = w.iter().map(|c| c.close as f64).sum::<f64>() / period as f64 / 100.0;
        [candles[i + period - 1].timestamp.timestamp() as f64, avg]
    }).collect()
}

fn indicator_labels(params: &StrategyParams) -> (String, String, String) {
    match params.name.as_str() {
        "sma_crossover" => (
            format!("SMA {}", params.short_period),
            format!("SMA {}", params.long_period),
            String::new(),
        ),
        "bollinger" => (
            "BB Mitte".into(),
            "BB Oben".into(),
            "BB Unten".into(),
        ),
        _ => ("SMA 20".into(), String::new(), String::new()),
    }
}

fn strategy_display_name(name: &str) -> &str {
    match name {
        "sma_crossover" => "SMA Crossover",
        "rsi"           => "RSI",
        "macd"          => "MACD",
        "bollinger"     => "Bollinger Bands",
        _               => name,
    }
}

// ── Entry Point ───────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let config = Config::load(Path::new("config.toml"))?;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 820.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title("TradingBot"),
        ..Default::default()
    };

    eframe::run_native(
        "TradingBot",
        options,
        Box::new(|_cc| Ok(Box::new(TradingApp::new(config)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
