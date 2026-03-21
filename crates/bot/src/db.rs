use anyhow::Result;
use chrono::DateTime;
use rusqlite::{params, Connection};

use crate::market_data::Candle;
use crate::paper_trading::engine::{Position, Trade, TradeSide};

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
                symbol           VARCHAR NOT NULL,
                side             VARCHAR NOT NULL,
                quantity         BIGINT  NOT NULL,
                price_cents      BIGINT  NOT NULL,
                timestamp        BIGINT  NOT NULL,
                pnl_cents        BIGINT  NOT NULL,
                commission_cents BIGINT  NOT NULL
            );

            CREATE TABLE IF NOT EXISTS positions (
                symbol          VARCHAR PRIMARY KEY,
                quantity        BIGINT  NOT NULL,
                avg_cost_cents  BIGINT  NOT NULL,
                entry_timestamp BIGINT  NOT NULL
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

    pub fn save_cash(&self, cash: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO state (key, value) VALUES ('cash', ?)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![cash],
        )?;
        Ok(())
    }

    pub fn load_positions(&self) -> Result<Vec<Position>> {
        let mut stmt = self
            .conn
            .prepare("SELECT symbol, quantity, avg_cost_cents, entry_timestamp FROM positions")?;
        let positions = stmt
            .query_map([], |row| {
                let ts: i64 = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    ts,
                ))
            })?
            .filter_map(|r| r.ok())
            .filter_map(|(symbol, quantity, avg_cost_cents, ts)| {
                Some(Position {
                    symbol: symbol.clone(),
                    quantity,
                    avg_cost_cents,
                    entry_timestamp: DateTime::from_timestamp(ts, 0)?,
                })
            })
            .collect();
        Ok(positions)
    }

    pub fn save_positions(&self, positions: &std::collections::HashMap<String, Position>) -> Result<()> {
        self.conn.execute("DELETE FROM positions", [])?;
        for p in positions.values() {
            self.conn.execute(
                "INSERT INTO positions (symbol, quantity, avg_cost_cents, entry_timestamp) VALUES (?, ?, ?, ?)",
                params![p.symbol, p.quantity, p.avg_cost_cents, p.entry_timestamp.timestamp()],
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
                 (symbol, side, quantity, price_cents, timestamp, pnl_cents, commission_cents)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                trade.symbol,
                side,
                trade.quantity,
                trade.price_cents,
                trade.timestamp.timestamp(),
                trade.pnl_cents,
                trade.commission_cents,
            ],
        )?;
        Ok(())
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
