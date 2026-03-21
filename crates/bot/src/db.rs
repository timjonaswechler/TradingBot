use anyhow::Result;
use chrono::DateTime;
use rusqlite::{params, Connection};

use crate::market_data::Candle;
use crate::paper_trading::{Position, Trade, TradeSide};

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    // ── Candle-Tabellen (eine pro Asset, mehrere Intervalle) ─────────────────

    /// Erstellt die Candle-Tabelle für ein Asset falls noch nicht vorhanden.
    /// PRIMARY KEY ist (timestamp, interval) um mehrere Intervalle zu unterstützen.
    pub fn ensure_asset_table(&self, asset: &str) -> Result<()> {
        let table = candle_table(asset);
        self.conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"{table}\" (
                timestamp BIGINT  NOT NULL,
                interval  VARCHAR NOT NULL,
                open      BIGINT  NOT NULL,
                high      BIGINT  NOT NULL,
                low       BIGINT  NOT NULL,
                close     BIGINT  NOT NULL,
                volume    BIGINT  NOT NULL,
                PRIMARY KEY (timestamp, interval)
            );"
        ))?;
        Ok(())
    }

    // ── Nicht-verfügbare Intervalle tracken ───────────────────────────────────

    /// Erstellt die Tabelle für dauerhaft nicht-verfügbare Asset×Intervall-Kombinationen.
    pub fn ensure_unavailable_table(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS unavailable_intervals (
                asset    VARCHAR NOT NULL,
                interval VARCHAR NOT NULL,
                PRIMARY KEY (asset, interval)
            );"
        )?;
        Ok(())
    }

    /// Markiert eine Asset×Intervall-Kombination als dauerhaft nicht verfügbar.
    pub fn mark_unavailable(&self, asset: &str, interval: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO unavailable_intervals (asset, interval) VALUES (?, ?)
             ON CONFLICT (asset, interval) DO NOTHING",
            params![asset, interval],
        )?;
        Ok(())
    }

    /// Prüft ob eine Asset×Intervall-Kombination als nicht verfügbar markiert ist.
    pub fn is_unavailable(&self, asset: &str, interval: &str) -> bool {
        self.conn.query_row(
            "SELECT 1 FROM unavailable_intervals WHERE asset = ? AND interval = ?",
            params![asset, interval],
            |_| Ok(true),
        ).unwrap_or(false)
    }

    /// Fügt Candles ein und überspringt Duplikate (anhand timestamp + interval).
    pub fn upsert_candles(&self, asset: &str, candles: &[Candle], interval: &str) -> Result<usize> {
        let table = candle_table(asset);
        let mut inserted = 0;
        for c in candles {
            let ts = c.timestamp.timestamp();
            let rows = self.conn.execute(
                &format!(
                    "INSERT INTO \"{table}\"
                         (timestamp, interval, open, high, low, close, volume)
                     VALUES (?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT (timestamp, interval) DO NOTHING"
                ),
                params![ts, interval, c.open, c.high, c.low, c.close, c.volume],
            )?;
            inserted += rows;
        }
        Ok(inserted)
    }

    /// Neuester gespeicherter Timestamp für ein Asset + Intervall.
    /// Gibt None zurück wenn noch keine Daten vorhanden (→ Full-Fetch nötig).
    pub fn get_last_timestamp(&self, asset: &str, interval: &str) -> Result<Option<i64>> {
        let table = candle_table(asset);
        let ts: Option<i64> = self.conn
            .query_row(
                &format!(
                    "SELECT MAX(timestamp) FROM \"{table}\" WHERE interval = ?"
                ),
                params![interval],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(ts)
    }

    /// Gibt ALLE Candles eines Assets + Intervalls zurück, älteste zuerst (für Backtesting).
    pub fn get_all_candles_asc(&self, asset: &str, interval: &str) -> Result<Vec<Candle>> {
        let table = candle_table(asset);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT timestamp, open, high, low, close, volume
             FROM \"{table}\"
             WHERE interval = ?
             ORDER BY timestamp ASC"
        ))?;

        let candles = stmt
            .query_map(params![interval], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(ts, open, high, low, close, volume)| {
                Some(Candle {
                    timestamp: DateTime::from_timestamp(ts, 0)?,
                    open,
                    high,
                    low,
                    close,
                    volume,
                })
            })
            .collect();

        Ok(candles)
    }

    /// Gibt die letzten `limit` Candles eines Intervalls zurück (neueste zuerst).
    pub fn get_candles(&self, asset: &str, interval: &str, limit: usize) -> Result<Vec<Candle>> {
        let table = candle_table(asset);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT timestamp, open, high, low, close, volume
             FROM \"{table}\"
             WHERE interval = ?
             ORDER BY timestamp DESC
             LIMIT {limit}"
        ))?;

        let candles = stmt
            .query_map(params![interval], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(ts, open, high, low, close, volume)| {
                Some(Candle {
                    timestamp: DateTime::from_timestamp(ts, 0)?,
                    open,
                    high,
                    low,
                    close,
                    volume,
                })
            })
            .collect();

        Ok(candles)
    }

    // ── State-Tabellen (Trades, Positionen, Kontostand) ───────────────────────

    pub fn ensure_state_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS state (
                key   VARCHAR PRIMARY KEY,
                value BIGINT  NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trades (
                asset          VARCHAR NOT NULL,
                side           VARCHAR NOT NULL,
                quantity       BIGINT  NOT NULL,
                price          BIGINT  NOT NULL,
                fee            BIGINT  NOT NULL,
                timestamp      BIGINT  NOT NULL,
                strategy       VARCHAR NOT NULL,
                gain_loss      BIGINT,
                gain_loss_pct  REAL,
                tax            BIGINT
            );

            CREATE TABLE IF NOT EXISTS positions (
                asset         VARCHAR PRIMARY KEY,
                quantity      BIGINT NOT NULL,
                avg_buy_price BIGINT NOT NULL
            );",
        )?;
        Ok(())
    }

    pub fn load_cash(&self, starting_capital: i64) -> Result<i64> {
        let cash: Option<i64> = self
            .conn
            .query_row("SELECT value FROM state WHERE key = 'cash'", [], |r| {
                r.get(0)
            })
            .ok();
        Ok(cash.unwrap_or(starting_capital))
    }

    pub fn load_exemption_remaining(&self, freistellungsauftrag: i64) -> Result<i64> {
        let val: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM state WHERE key = 'exemption_remaining'",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(val.unwrap_or(freistellungsauftrag))
    }

    pub fn save_cash(&self, cash: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO state (key, value) VALUES ('cash', ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![cash],
        )?;
        Ok(())
    }

    pub fn save_exemption_remaining(&self, remaining: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO state (key, value) VALUES ('exemption_remaining', ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![remaining],
        )?;
        Ok(())
    }

    pub fn load_positions(&self) -> Result<Vec<Position>> {
        let mut stmt = self
            .conn
            .prepare("SELECT asset, quantity, avg_buy_price FROM positions")?;
        let positions = stmt
            .query_map([], |row| {
                Ok(Position {
                    asset:         row.get(0)?,
                    quantity:      row.get(1)?,
                    avg_buy_price: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(positions)
    }

    pub fn save_positions(&self, positions: &[Position]) -> Result<()> {
        self.conn.execute("DELETE FROM positions", [])?;
        for p in positions {
            self.conn.execute(
                "INSERT INTO positions (asset, quantity, avg_buy_price) VALUES (?, ?, ?)",
                params![p.asset, p.quantity, p.avg_buy_price],
            )?;
        }
        Ok(())
    }

    pub fn save_trade(&self, trade: &Trade) -> Result<()> {
        let side = match trade.side {
            TradeSide::Buy   => "buy",
            TradeSide::Sell  => "sell",
            TradeSide::Short => "short",
            TradeSide::Cover => "cover",
        };
        self.conn.execute(
            "INSERT INTO trades
                 (asset, side, quantity, price, fee, timestamp, strategy, gain_loss, gain_loss_pct, tax)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                trade.asset,
                side,
                trade.quantity,
                trade.price,
                trade.fee,
                trade.timestamp,
                trade.strategy,
                trade.gain_loss,
                trade.gain_loss_pct,
                trade.tax
            ],
        )?;
        Ok(())
    }

    pub fn load_trades(&self) -> Result<Vec<Trade>> {
        let mut stmt = self.conn.prepare(
            "SELECT asset, side, quantity, price, fee, timestamp, strategy, gain_loss, gain_loss_pct, tax
             FROM trades ORDER BY timestamp ASC",
        )?;
        let trades = stmt
            .query_map([], |row| {
                let side_str: String = row.get(1)?;
                let side = if side_str == "buy" { TradeSide::Buy } else { TradeSide::Sell };
                Ok(Trade {
                    asset:           row.get(0)?,
                    side,
                    quantity:        row.get(2)?,
                    price:           row.get(3)?,
                    fee:             row.get(4)?,
                    timestamp:       row.get(5)?,
                    strategy:        row.get(6)?,
                    gain_loss:       row.get(7)?,
                    gain_loss_pct:   row.get(8)?,
                    tax:             row.get(9)?,
                    // stub-only fields — not persisted in this schema
                    price_cents:     row.get(3)?,
                    pnl_cents:       row.get(7).unwrap_or(0),
                    commission_cents: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(trades)
    }
}

/// Erzeugt einen sicheren SQL-Tabellennamen aus einem Asset-Symbol.
/// Beispiel: "^RUT" → "rut_candles", "BTC-USD" → "btc_usd_candles"
fn candle_table(asset: &str) -> String {
    let name = asset
        .to_lowercase()
        .replace(['^', '-', '.', '/', ' '], "_");
    format!("{name}_candles")
}
