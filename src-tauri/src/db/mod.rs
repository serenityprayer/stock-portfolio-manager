use rusqlite::{params, Connection, Result};
use std::sync::Mutex;

pub mod crypto_contract;
pub mod crypto_spot;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                description TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS categories (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                color TEXT NOT NULL,
                icon TEXT NOT NULL,
                is_system INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS holdings (
                id TEXT PRIMARY KEY NOT NULL,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                category_id TEXT REFERENCES categories(id) ON DELETE SET NULL,
                shares REAL NOT NULL DEFAULT 0,
                avg_cost REAL NOT NULL DEFAULT 0,
                currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS transactions (
                id TEXT PRIMARY KEY NOT NULL,
                holding_id TEXT REFERENCES holdings(id) ON DELETE SET NULL,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL', 'OPEN', 'PAY')),
                shares REAL NOT NULL,
                price REAL NOT NULL,
                total_amount REAL NOT NULL,
                commission REAL NOT NULL DEFAULT 0,
                currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
                traded_at TEXT NOT NULL,
                notes TEXT,
                created_at TEXT NOT NULL
            );
        ")?;

        // Seed system categories (fixed UUIDs — INSERT OR IGNORE prevents re-insertion even after rename)
        let categories = [
            ("a0000001-0000-0000-0000-000000000001", "现金类", "#22C55E", "💵", 1, 1),
            ("a0000001-0000-0000-0000-000000000002", "分红股", "#3B82F6", "💰", 1, 2),
            ("a0000001-0000-0000-0000-000000000003", "成长股", "#F97316", "🚀", 1, 3),
            ("a0000001-0000-0000-0000-000000000004", "套利",   "#8B5CF6", "🔄", 1, 4),
        ];

        let now = chrono::Utc::now().to_rfc3339();
        for (id, name, color, icon, is_system, sort_order) in &categories {
            conn.execute(
                "INSERT OR IGNORE INTO categories (id, name, color, icon, is_system, sort_order, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, name, color, icon, is_system, sort_order, now],
            )?;
        }

        // 清理重复系统类别（旧版随机UUID + 改名后重入导致的重复）
        // 策略：将旧随机ID的系统类别合并到固定UUID上，删掉多余条目
        let system_fixups = [
            ("a0000001-0000-0000-0000-000000000001", 1), // 现金类
            ("a0000001-0000-0000-0000-000000000002", 2), // 分红股
            ("a0000001-0000-0000-0000-000000000003", 3), // 成长股
            ("a0000001-0000-0000-0000-000000000004", 4), // 套利
        ];
        for (fixed_id, sort_order) in &system_fixups {
            let existing: Option<String> = conn.query_row(
                "SELECT id FROM categories WHERE id = ?1",
                rusqlite::params![fixed_id],
                |row| row.get(0),
            ).ok();
            if existing.is_none() {
                // 固定ID不存在，找一个同sort_order的非固定ID系统类别来"收编"
                if let Ok(old_id) = conn.query_row(
                    "SELECT id FROM categories WHERE sort_order = ?1 AND is_system = 1 AND id != ?2 LIMIT 1",
                    rusqlite::params![sort_order, fixed_id],
                    |row| row.get::<_, String>(0),
                ) {
                    conn.execute("UPDATE holdings SET category_id = ?1 WHERE category_id = ?2",
                        rusqlite::params![fixed_id, &old_id])?;
                    conn.execute("UPDATE categories SET id = ?1 WHERE id = ?2",
                        rusqlite::params![fixed_id, &old_id])?;
                }
            } else {
                // 固定ID已存在，删掉同sort_order的其他系统类别（迁移其持仓）
                let extras: Vec<String> = conn.prepare(
                    "SELECT id FROM categories WHERE sort_order = ?1 AND is_system = 1 AND id != ?2"
                )?.query_map(rusqlite::params![sort_order, fixed_id], |row| row.get(0))?
                 .collect::<Result<Vec<_>, _>>()?;
                for extra_id in &extras {
                    conn.execute("UPDATE holdings SET category_id = ?1 WHERE category_id = ?2",
                        rusqlite::params![fixed_id, extra_id])?;
                    conn.execute("DELETE FROM categories WHERE id = ?1",
                        rusqlite::params![extra_id])?;
                }
            }
        }

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS daily_portfolio_values (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL UNIQUE,
                total_cost REAL NOT NULL DEFAULT 0,
                total_value REAL NOT NULL DEFAULT 0,
                us_cost REAL NOT NULL DEFAULT 0,
                us_value REAL NOT NULL DEFAULT 0,
                cn_cost REAL NOT NULL DEFAULT 0,
                cn_value REAL NOT NULL DEFAULT 0,
                hk_cost REAL NOT NULL DEFAULT 0,
                hk_value REAL NOT NULL DEFAULT 0,
                exchange_rates TEXT NOT NULL DEFAULT '{}',
                daily_pnl REAL NOT NULL DEFAULT 0,
                cumulative_pnl REAL NOT NULL DEFAULT 0
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS daily_holding_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL,
                account_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                category_name TEXT,
                shares REAL NOT NULL DEFAULT 0,
                avg_cost REAL NOT NULL DEFAULT 0,
                close_price REAL NOT NULL DEFAULT 0,
                market_value REAL NOT NULL DEFAULT 0
            );
        ")?;

        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_daily_holding_snapshots_date
            ON daily_holding_snapshots(date);
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS benchmark_daily_prices (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                symbol TEXT NOT NULL,
                date TEXT NOT NULL,
                close_price REAL NOT NULL DEFAULT 0,
                change_percent REAL NOT NULL DEFAULT 0,
                UNIQUE(symbol, date)
            );
        ")?;

        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_benchmark_daily_prices_symbol_date
            ON benchmark_daily_prices(symbol, date);
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS quarterly_snapshots (
                id TEXT PRIMARY KEY NOT NULL,
                quarter TEXT NOT NULL UNIQUE,
                snapshot_date TEXT NOT NULL,
                total_value REAL NOT NULL DEFAULT 0,
                total_cost REAL NOT NULL DEFAULT 0,
                total_pnl REAL NOT NULL DEFAULT 0,
                us_value REAL NOT NULL DEFAULT 0,
                us_cost REAL NOT NULL DEFAULT 0,
                cn_value REAL NOT NULL DEFAULT 0,
                cn_cost REAL NOT NULL DEFAULT 0,
                hk_value REAL NOT NULL DEFAULT 0,
                hk_cost REAL NOT NULL DEFAULT 0,
                exchange_rates TEXT NOT NULL DEFAULT '{}',
                overall_notes TEXT,
                created_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS quarterly_holding_snapshots (
                id TEXT PRIMARY KEY NOT NULL,
                quarterly_snapshot_id TEXT NOT NULL REFERENCES quarterly_snapshots(id) ON DELETE CASCADE,
                account_id TEXT NOT NULL,
                account_name TEXT NOT NULL DEFAULT '',
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                category_name TEXT NOT NULL DEFAULT '未分类',
                category_color TEXT NOT NULL DEFAULT '#8B8B8B',
                shares REAL NOT NULL DEFAULT 0,
                avg_cost REAL NOT NULL DEFAULT 0,
                close_price REAL NOT NULL DEFAULT 0,
                market_value REAL NOT NULL DEFAULT 0,
                cost_value REAL NOT NULL DEFAULT 0,
                pnl REAL NOT NULL DEFAULT 0,
                pnl_percent REAL NOT NULL DEFAULT 0,
                weight REAL NOT NULL DEFAULT 0,
                notes TEXT
            );
        ")?;

        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_snapshot_id
            ON quarterly_holding_snapshots(quarterly_snapshot_id);
        ")?;

        conn.execute_batch("
            CREATE INDEX IF NOT EXISTS idx_quarterly_holding_snapshots_symbol
            ON quarterly_holding_snapshots(symbol);
        ")?;

        // Add decision_quality column if not exists (migration)
        let _ = conn.execute_batch("
            ALTER TABLE quarterly_holding_snapshots ADD COLUMN decision_quality TEXT;
        ");

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS price_alerts (
                id TEXT PRIMARY KEY NOT NULL,
                holding_id TEXT,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                alert_type TEXT NOT NULL CHECK(alert_type IN ('PRICE_ABOVE', 'PRICE_BELOW', 'CHANGE_ABOVE', 'CHANGE_BELOW', 'PNL_ABOVE', 'PNL_BELOW')),
                threshold REAL NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 1,
                is_triggered INTEGER NOT NULL DEFAULT 0,
                triggered_at TEXT,
                created_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS quote_provider_config (
                id INTEGER PRIMARY KEY DEFAULT 1,
                us_provider TEXT NOT NULL DEFAULT 'xueqiu',
                hk_provider TEXT NOT NULL DEFAULT 'xueqiu',
                cn_provider TEXT NOT NULL DEFAULT 'xueqiu',
                updated_at TEXT NOT NULL DEFAULT ''
            );
        ")?;

        // Add xueqiu_cookie column if not exists (migration)
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN xueqiu_cookie TEXT;
        ");

        // Add xueqiu_u column if not exists (migration)
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN xueqiu_u TEXT;
        ");
        // Crypto spot table
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS crypto_spot (
                id          TEXT PRIMARY KEY NOT NULL,
                symbol      TEXT NOT NULL,
                name        TEXT,
                buy_price   REAL NOT NULL,
                shares      REAL NOT NULL,
                fee         REAL DEFAULT 0,
                exchange    TEXT DEFAULT 'Binance',
                notes       TEXT,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
        ")?;

        // Crypto contract table
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS crypto_contract (
                id              TEXT PRIMARY KEY NOT NULL,
                symbol          TEXT NOT NULL,
                name            TEXT,
                asset_type      TEXT NOT NULL DEFAULT 'crypto' CHECK(asset_type IN ('crypto', 'tradfi')),
                position_type   TEXT NOT NULL CHECK(position_type IN ('long', 'short')),
                open_price      REAL NOT NULL,
                shares          REAL NOT NULL,
                leverage        REAL NOT NULL DEFAULT 1,
                margin          REAL NOT NULL,
                fee             REAL DEFAULT 0,
                exchange        TEXT DEFAULT 'Binance',
                notes           TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );
        ")?;

        // Migrate: add asset_type column if not exists
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract ADD COLUMN asset_type TEXT NOT NULL DEFAULT 'crypto' CHECK(asset_type IN ('crypto', 'tradfi'));
        ");

        // Migrate: rename table from crypto_contract_close_history to crypto_contract_history
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_close_history RENAME TO crypto_contract_history;
        ");

        // Crypto contract history table (nullable columns for add/close actions)
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS crypto_contract_history (
                id              TEXT PRIMARY KEY NOT NULL,
                contract_id     TEXT NOT NULL,
                symbol          TEXT NOT NULL,
                name            TEXT,
                asset_type      TEXT NOT NULL DEFAULT 'crypto',
                position_type   TEXT NOT NULL,
                action_type     TEXT NOT NULL DEFAULT 'close',
                open_price      REAL NOT NULL,
                close_price     REAL,
                close_shares    REAL,
                add_price       REAL,
                add_shares     REAL,
                new_avg_price   REAL,
                new_total_shares REAL,
                leverage        REAL NOT NULL DEFAULT 1,
                open_fee        REAL DEFAULT 0,
                close_fee       REAL DEFAULT 0,
                realized_pnl    REAL,
                return_rate     REAL,
                closed_at       TEXT NOT NULL,
                notes           TEXT
            );
        ")?;

        // Migrate: add fields to crypto_contract_history for 成交历史
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN action_type TEXT NOT NULL DEFAULT 'close';
        ");
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN add_price REAL;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN add_shares REAL;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN new_avg_price REAL;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN new_total_shares REAL;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE crypto_contract_history ADD COLUMN return_rate REAL;
        ");

        // 补算历史平仓记录的 return_rate（之前创建的记录该字段为 NULL）
        {
            let mut stmt = conn.prepare(
                "SELECT id, open_price, close_shares, leverage, open_fee, close_fee, realized_pnl
                 FROM crypto_contract_history
                 WHERE action_type = 'close' AND return_rate IS NULL"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                ))
            })?;
            for row in rows {
                let (id, open_price, close_shares, leverage, open_fee, close_fee, realized_pnl) =
                    row.map_err(|e| rusqlite::Error::from(e))?;
                let margin = open_price * close_shares / leverage + open_fee + close_fee;
                let return_rate = if margin.abs() > 1e-8 {
                    realized_pnl / margin
                } else {
                    0.0
                };
                conn.execute(
                    "UPDATE crypto_contract_history SET return_rate = ?1 WHERE id = ?2",
                    params![return_rate, id],
                )?;
            }
        }

        // 补填 open 类型历史记录的 close_shares（开仓数量）
        // 旧记录创建时未写入，从合约表回填
        conn.execute_batch("
            UPDATE crypto_contract_history
            SET close_shares = (
                SELECT shares FROM crypto_contract
                WHERE crypto_contract.id = crypto_contract_history.contract_id
            )
            WHERE action_type = 'open' AND close_shares IS NULL;
        ")?;

        // 清理残留备份表（如有）
        let _ = conn.execute_batch("
            DROP TABLE IF EXISTS _crypto_contract_history_bk;
            DROP TABLE IF EXISTS _crypto_contract_close_history_bk;
        ");

        // 清理错误的 historical 快照数据
        // backfill_snapshots 旧版 bug：unwind 不处理 OPEN 交易类型，
        // 导致开仓前的日期也被写入了快照（用当前持仓股数 × 历史价格）
        // 修复：删除每个 symbol 在其首笔交易日期之前的所有快照
        conn.execute_batch("
            DELETE FROM daily_holding_snapshots
            WHERE ROWID IN (
                SELECT s.ROWID
                FROM daily_holding_snapshots s
                INNER JOIN (
                    SELECT account_id, symbol, MIN(DATE(traded_at)) as first_trade_date
                    FROM transactions
                    GROUP BY account_id, symbol
                ) first_tx ON s.account_id = first_tx.account_id
                    AND s.symbol = first_tx.symbol
                WHERE s.date < first_tx.first_trade_date
            );
        ")?;

        // 清理 daily_portfolio_values 中的错误历史数据
        // 同上：backfill 旧版 bug 导致建仓前的日期也写入了组合市值
        conn.execute(
            "DELETE FROM daily_portfolio_values
             WHERE date < (SELECT MIN(DATE(traded_at)) FROM transactions)",
            [],
        )?;

        // 升级 crypto_contract_history 表结构（仅首次执行）
        // 条件：表存在 且 close_price 列为 NOT NULL（需要改为可空）
        {
            let need_rebuild = conn
                .prepare(
                    "SELECT COUNT(*) FROM pragma_table_info('crypto_contract_history')
                     WHERE name IN ('close_price','close_shares','realized_pnl')
                       AND \"notnull\" = 1"
                )
                .map(|mut s| {
                    s.query_row([], |r| r.get::<_, i32>(0)).unwrap_or(0) > 0
                })
                .unwrap_or(false);

            if need_rebuild {
                // 1. 备份旧表
                let _ = conn.execute_batch(
                    "ALTER TABLE crypto_contract_history RENAME TO _crypto_contract_history_bk;"
                );

                // 2. 新建表（所有字段可空）
                conn.execute_batch("
                    CREATE TABLE crypto_contract_history (
                        id              TEXT PRIMARY KEY NOT NULL,
                        contract_id     TEXT NOT NULL,
                        symbol          TEXT NOT NULL,
                        name            TEXT,
                        asset_type      TEXT NOT NULL DEFAULT 'crypto',
                        position_type   TEXT NOT NULL,
                        action_type     TEXT NOT NULL DEFAULT 'close',
                        open_price      REAL NOT NULL,
                        close_price     REAL,
                        close_shares    REAL,
                        add_price       REAL,
                        add_shares     REAL,
                        new_avg_price   REAL,
                        new_total_shares REAL,
                        leverage        REAL NOT NULL DEFAULT 1,
                        open_fee        REAL DEFAULT 0,
                        close_fee       REAL DEFAULT 0,
                        realized_pnl    REAL,
                        return_rate     REAL,
                        closed_at       TEXT NOT NULL,
                        notes           TEXT
                    );
                ")?;

                // 3. 迁移旧数据
                let has_return_rate = conn.prepare(
                    "SELECT COUNT(*) FROM pragma_table_info('_crypto_contract_history_bk') WHERE name='return_rate'"
                ).map(|mut s| {
                    s.query_row([], |r| r.get::<_, i32>(0)).unwrap_or(0) > 0
                }).unwrap_or(false);

                if has_return_rate {
                    conn.execute_batch("
                        INSERT INTO crypto_contract_history
                        SELECT id, contract_id, symbol, name, asset_type, position_type,
                               'close' as action_type, open_price, close_price, close_shares,
                               NULL, NULL, NULL, NULL,
                               leverage, open_fee, close_fee, realized_pnl,
                               return_rate, closed_at, notes
                        FROM _crypto_contract_history_bk;
                    ")?;
                } else {
                    conn.execute_batch("
                        INSERT INTO crypto_contract_history
                        SELECT id, contract_id, symbol, name, asset_type, position_type,
                               'close' as action_type, open_price, close_price, close_shares,
                               NULL, NULL, NULL, NULL,
                               leverage, open_fee, close_fee, realized_pnl,
                               NULL, closed_at, notes
                        FROM _crypto_contract_history_bk;
                    ")?;
                }

                // 4. 删掉备份表
                let _ = conn.execute_batch("DROP TABLE IF EXISTS _crypto_contract_history_bk;");
            }
        }

        // NOTE: xueqiu_cookie (xq_a_token) and xueqiu_u (user ID) are
        // different values – do NOT copy one into the other.  Users who
        // previously only had xueqiu_cookie set will need to enter their
        // u value separately via the settings UI;

        // Add per-market cost adjustment setting columns (migration).
        // CN defaults to 1 (true) because A-share investors traditionally adjust
        // cost basis on every transaction. US and HK default to 0 (false) because
        // those markets realise gains on SELL and dividends are taxed separately.
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN cn_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 1;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN us_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0;
        ");
        let _ = conn.execute_batch("
            ALTER TABLE quote_provider_config ADD COLUMN hk_adjust_sell_pay_cost INTEGER NOT NULL DEFAULT 0;
        ");

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS ai_config (
                id INTEGER PRIMARY KEY DEFAULT 1,
                provider TEXT NOT NULL DEFAULT 'openai',
                api_key TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT 'gpt-4',
                base_url TEXT,
                system_prompt TEXT NOT NULL DEFAULT '你是一位专业的投资顾问，帮助用户分析股票投资组合。',
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS cached_quotes (
                symbol TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL,
                current_price REAL NOT NULL DEFAULT 0,
                previous_close REAL NOT NULL DEFAULT 0,
                change REAL NOT NULL DEFAULT 0,
                change_percent REAL NOT NULL DEFAULT 0,
                high REAL NOT NULL DEFAULT 0,
                low REAL NOT NULL DEFAULT 0,
                volume INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS cached_exchange_rates (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                usd_cny REAL NOT NULL,
                usd_hkd REAL NOT NULL,
                cny_hkd REAL NOT NULL,
                updated_at TEXT NOT NULL
            );
        ")?;

        migrate_transactions_check_constraint(&conn)?;

        // Convert synthetic BUY records to OPEN type so they are correctly
        // treated as zero-cash-impact position entries everywhere.
        //
        // These migrations are idempotent (UPDATE with 0 rows is not an error)
        // and failures are tolerated – if they don't apply, the frontend filter
        // below provides a fallback by explicitly excluding 'backfill:initial'.
        //
        // 1. Records created by the backfill_open_transactions tool:
        //    these always carry notes = 'backfill:initial'.
        let _ = conn.execute_batch("
            UPDATE transactions
            SET transaction_type = 'OPEN'
            WHERE transaction_type = 'BUY'
              AND notes = 'backfill:initial'
              AND symbol NOT LIKE '$CASH-%';
        ");

        // 2. Records created by create_holding (initial position entries):
        //    identified by notes IS NULL, commission = 0, and the transaction's
        //    traded_at matching its parent holding's created_at exactly (because
        //    create_holding sets both to `now` in the same operation).
        let _ = conn.execute_batch("
            UPDATE transactions
            SET transaction_type = 'OPEN'
            WHERE transaction_type = 'BUY'
              AND notes IS NULL
              AND commission = 0.0
              AND symbol NOT LIKE '$CASH-%'
              AND holding_id IS NOT NULL
              AND traded_at = (
                  SELECT h.created_at FROM holdings h WHERE h.id = holding_id
              );
        ");

        Ok(())
    }
}

fn migrate_transactions_check_constraint(conn: &Connection) -> Result<()> {
    // Check if the transactions table already has 'PAY' in its CHECK constraint.
    // If not, recreate the table with the updated constraint.
    let schema: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='transactions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if schema.contains("'PAY'") {
        return Ok(());
    }

    conn.execute_batch("
        PRAGMA foreign_keys = OFF;

        CREATE TABLE transactions_new (
            id TEXT PRIMARY KEY NOT NULL,
            holding_id TEXT REFERENCES holdings(id) ON DELETE SET NULL,
            account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            symbol TEXT NOT NULL,
            name TEXT NOT NULL,
            market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
            transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL', 'OPEN', 'PAY')),
            shares REAL NOT NULL,
            price REAL NOT NULL,
            total_amount REAL NOT NULL,
            commission REAL NOT NULL DEFAULT 0,
            currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
            traded_at TEXT NOT NULL,
            notes TEXT,
            created_at TEXT NOT NULL
        );

        INSERT INTO transactions_new SELECT * FROM transactions;
        DROP TABLE transactions;
        ALTER TABLE transactions_new RENAME TO transactions;

        PRAGMA foreign_keys = ON;
    ")?;

    Ok(())
}

#[cfg(test)]
mod tests;
