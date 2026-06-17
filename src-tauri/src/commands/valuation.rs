use crate::db::Database;
use crate::models::valuation::PortfolioValuation;
use crate::services::quote_service::QuoteCache;
use crate::services::valuation_service;

/// Get portfolio valuation metrics (weighted PE, dividend yield) with
/// optional market/account/category filtering.
#[tauri::command(rename_all = "camelCase")]
pub fn get_portfolio_valuation(
    db: tauri::State<'_, Database>,
    quote_cache: tauri::State<'_, QuoteCache>,
    market: Option<String>,
    account_id: Option<String>,
    category_id: Option<String>,
) -> Result<PortfolioValuation, String> {
    let holdings = {
        let mut sql = String::from(
            "SELECT h.id, h.account_id, a.name as account_name, h.symbol, h.name,
                    h.market, COALESCE(c.name, '未分类') as category_name,
                    COALESCE(c.color, '#8c8c8c') as category_color,
                    h.shares, h.avg_cost, h.currency
             FROM holdings h
             LEFT JOIN accounts a ON h.account_id = a.id
             LEFT JOIN categories c ON h.category_id = c.id
             WHERE h.shares > 0",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref m) = market {
            sql.push_str(&format!(" AND h.market = ?{}", params.len() + 1));
            params.push(Box::new(m.clone()));
        }
        if let Some(ref aid) = account_id {
            sql.push_str(&format!(" AND h.account_id = ?{}", params.len() + 1));
            params.push(Box::new(aid.clone()));
        }
        if let Some(ref cid) = category_id {
            sql.push_str(&format!(" AND h.category_id = ?{}", params.len() + 1));
            params.push(Box::new(cid.clone()));
        }

        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                // SELECT order: id(0), account_id(1), account_name(2),
                //                symbol(3), name(4), market(5),
                //                category_name(6), category_color(7),
                //                shares(8), avg_cost(9), currency(10)
                let symbol: String = row.get(3)?;
                let name: String = row.get(4)?;
                let market: String = row.get(5)?;
                let category_name: String = row.get(6)?;
                let category_color: String = row.get(7)?;
                let shares: f64 = row.get(8)?;
                let avg_cost: f64 = row.get(9)?;
                let currency: String = row.get(10)?;

                // Lookup quote for current price
                let current_price = quote_cache.get(&symbol).map(|q| q.current_price).unwrap_or(avg_cost);
                let market_value = shares * current_price;

                Ok(crate::models::HoldingDetail {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    account_name: String::new(), // not used for valuation
                    symbol,
                    name,
                    market,
                    category_name,
                    category_color,
                    shares,
                    avg_cost,
                    current_price,
                    market_value,
                    cost_value: shares * avg_cost,
                    pnl: market_value - shares * avg_cost,
                    pnl_percent: if avg_cost > 0.0 {
                        (market_value - shares * avg_cost) / (shares * avg_cost) * 100.0
                    } else {
                        0.0
                    },
                    daily_pnl: 0.0,
                    currency,
                    market_value_usd: market_value, // simplified
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    // Build a quote map from cache
    let quote_map: std::collections::HashMap<String, crate::models::StockQuote> = {
        let mut map = std::collections::HashMap::new();
        for h in &holdings {
            if let Some(q) = quote_cache.get(&h.symbol) {
                map.insert(h.symbol.clone(), q);
            }
        }
        map
    };

    Ok(valuation_service::compute_valuation(&holdings, &quote_map))
}
