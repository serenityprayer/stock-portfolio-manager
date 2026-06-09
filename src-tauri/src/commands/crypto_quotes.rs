use crate::db::Database;
use crate::models::StockQuote;
use crate::services::http_client::general_client;
use chrono::Utc;
use reqwest::header;
use std::collections::HashMap;
use std::time::Duration;
use tauri::State;

// ── Crypto quotes (Gate.io → Binance fallback) ───────────────────────────

/// Try Gate.io first, fallback to Binance if Gate.io fails.
async fn fetch_crypto_single(symbol: &str) -> Result<StockQuote, String> {
    match fetch_from_gateio(symbol).await {
        Ok(q) => return Ok(q),
        Err(e) => {
            eprintln!("[crypto_quotes] Gate.io failed for {}: {}, trying Binance...", symbol, e);
        }
    }
    fetch_from_binance(symbol).await
}

/// Gate.io API response item
#[derive(Debug, serde::Deserialize)]
struct GateioTicker {
    currency_pair: String,
    last: String,
    change_percentage: String,
    high_24h: String,
    low_24h: String,
    base_volume: String,
}

/// Map user symbol (e.g. "BTC") to Gate.io currency pair (e.g. "BTC_USDT").
fn to_gateio_pair(symbol: &str) -> String {
    let s = symbol.to_uppercase();
    if s.contains('_') {
        return s;
    }
    format!("{}_USDT", s)
}

/// Map user symbol (e.g. "HOOD") to Binance symbol (e.g. "HOODUSDT").
fn to_binance_symbol(symbol: &str) -> String {
    let s = symbol.to_uppercase();
    if s.ends_with("USDT") {
        return s;
    }
    format!("{}USDT", s)
}

async fn fetch_from_gateio(symbol: &str) -> Result<StockQuote, String> {
    let pair = to_gateio_pair(symbol);
    let url = format!(
        "https://api.gateio.ws/api/v4/spot/tickers?currency_pair={}",
        pair
    );

    let client = general_client();
    let resp = client
        .get(&url)
        .header(header::ACCEPT, "application/json")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Gate.io request failed for {}: {}", symbol, e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read Gate.io body failed for {}: {}", symbol, e))?;

    if !status.is_success() {
        return Err(format!(
            "Gate.io HTTP {} for {}: {}",
            status,
            symbol,
            &text[..text.len().min(300)]
        ));
    }

    let tickers: Vec<GateioTicker> = serde_json::from_str(&text)
        .map_err(|e| {
            format!(
                "Parse Gate.io JSON failed for {}: {} | body: {}",
                symbol,
                e,
                &text[..text.len().min(300)]
            )
        })?;

    if tickers.is_empty() {
        return Err(format!(
            "No data from Gate.io for {} (pair: {})",
            symbol, pair
        ));
    }

    let t = &tickers[0];

    let current_price = t.last.parse::<f64>().unwrap_or(0.0);
    let change_percent = t.change_percentage.parse::<f64>().unwrap_or(0.0);
    let high = t.high_24h.parse::<f64>().unwrap_or(current_price);
    let low = t.low_24h.parse::<f64>().unwrap_or(current_price);
    let volume = t.base_volume.parse::<f64>().unwrap_or(0.0) as u64;

    let previous_close = if change_percent != 0.0 && current_price > 0.0 {
        current_price / (1.0 + change_percent / 100.0)
    } else {
        current_price
    };
    let change = current_price - previous_close;

    let name = t
        .currency_pair
        .split('_')
        .next()
        .unwrap_or(symbol)
        .to_string();

    Ok(StockQuote {
        symbol: symbol.to_string(),
        name,
        market: "CRYPTO".to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high,
        low,
        volume,
        updated_at: Utc::now().to_rfc3339(),
    })
}

/// Binance 24hr ticker response
#[derive(Debug, serde::Deserialize)]
struct BinanceTicker {
    symbol: String,
    lastPrice: String,
    priceChangePercent: String,
    highPrice: String,
    lowPrice: String,
    volume: String,
}

async fn fetch_from_binance(symbol: &str) -> Result<StockQuote, String> {
    let binance_symbol = to_binance_symbol(symbol);
    let url = format!(
        "https://api.binance.com/api/v3/ticker/24hr?symbol={}",
        binance_symbol
    );

    let client = general_client();
    let resp = client
        .get(&url)
        .header(header::ACCEPT, "application/json")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Binance request failed for {}: {}", symbol, e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Read Binance body failed for {}: {}", symbol, e))?;

    if !status.is_success() {
        return Err(format!(
            "Binance HTTP {} for {}: {}",
            status,
            symbol,
            &text[..text.len().min(300)]
        ));
    }

    let t: BinanceTicker = serde_json::from_str(&text)
        .map_err(|e| {
            format!(
                "Parse Binance JSON failed for {}: {} | body: {}",
                symbol,
                e,
                &text[..text.len().min(300)]
            )
        })?;

    let current_price = t.lastPrice.parse::<f64>().unwrap_or(0.0);
    let change_percent = t.priceChangePercent.parse::<f64>().unwrap_or(0.0);
    let high = t.highPrice.parse::<f64>().unwrap_or(current_price);
    let low = t.lowPrice.parse::<f64>().unwrap_or(current_price);
    let volume = t.volume.parse::<f64>().unwrap_or(0.0) as u64;

    let previous_close = if change_percent != 0.0 && current_price > 0.0 {
        current_price / (1.0 + change_percent / 100.0)
    } else {
        current_price
    };
    let change = current_price - previous_close;

    Ok(StockQuote {
        symbol: symbol.to_string(),
        name: binance_symbol.clone(),
        market: "CRYPTO".to_string(),
        current_price,
        previous_close,
        change,
        change_percent,
        high,
        low,
        volume,
        updated_at: Utc::now().to_rfc3339(),
    })
}

#[tauri::command]
pub async fn fetch_crypto_quotes(
    symbols: String,
) -> Result<HashMap<String, serde_json::Value>, String> {
    let symbols_vec: Vec<&str> = symbols.split(',').map(|s| s.trim()).collect();
    let mut results: HashMap<String, serde_json::Value> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    for (i, symbol) in symbols_vec.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        match fetch_crypto_single(symbol).await {
            Ok(quote) => {
                eprintln!(
                    "[crypto_quotes] {} quote: price={}",
                    symbol, quote.current_price
                );
                results.insert(
                    symbol.to_string(),
                    serde_json::json!({
                        "price": quote.current_price,
                        "change": quote.change,
                        "changePercent": quote.change_percent,
                        "high": quote.high,
                        "low": quote.low,
                        "volume": quote.volume,
                        "name": quote.name,
                    }),
                );
            }
            Err(e) => {
                let err_msg = format!("获取 {} 行情失败: {}", symbol, e);
                eprintln!("{}", err_msg);
                errors.push(err_msg);
            }
        }
    }

    if results.is_empty() && !errors.is_empty() {
        return Err(format!("所有币种行情获取失败:\n{}", errors.join("\n")));
    }

    if !errors.is_empty() {
        eprintln!(
            "[crypto_quotes] {} errors (partial failure): {:?}",
            errors.len(),
            errors
        );
    }

    Ok(results)
}

// ── TradFi quotes (复用持仓管理行情逻辑) ────────────────────────────────
//
// 复用 quote_service 中已有的 fetch_us_quote_with_provider / fetch_hk_quote_with_provider，
// 与「持仓管理」使用完全相同的 provider 配置（默认 East Money，国内可访问）。

/// 获取单个 TradFi 标的行情，从数据库读取 provider 配置。
async fn fetch_tradfi_single(symbol: &str, provider: &str) -> Result<StockQuote, String> {
    crate::services::quote_service::fetch_us_quote_with_provider(symbol, provider).await
}

/// 获取 TradFi 标的（股票/ETF）行情。
/// 从数据库读取行情 provider 配置，与「持仓管理」使用相同的 provider。
#[tauri::command]
pub async fn fetch_tradfi_quotes(
    db: tauri::State<'_, Database>,
    symbols: String,
) -> Result<HashMap<String, serde_json::Value>, String> {
    // 从数据库读取 us_provider 配置
    let us_provider = {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT us_provider FROM quote_provider_config LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "xueqiu".to_string())
    };

    let symbols_vec: Vec<&str> = symbols.split(',').map(|s| s.trim()).collect();
    let mut results: HashMap<String, serde_json::Value> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    for (i, symbol) in symbols_vec.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        match fetch_tradfi_single(symbol, &us_provider).await {
            Ok(quote) => {
                eprintln!(
                    "[tradfi_quotes] {} quote: price={}",
                    symbol, quote.current_price
                );
                results.insert(
                    symbol.to_string(),
                    serde_json::json!({
                        "price": quote.current_price,
                        "change": quote.change,
                        "changePercent": quote.change_percent,
                        "high": quote.high,
                        "low": quote.low,
                        "volume": quote.volume,
                        "name": quote.name,
                    }),
                );
            }
            Err(e) => {
                let err_msg = format!("获取 {} 行情失败: {}", symbol, e);
                eprintln!("{}", err_msg);
                errors.push(err_msg);
            }
        }
    }

    if results.is_empty() && !errors.is_empty() {
        return Err(format!("所有 TradFi 行情获取失败:\n{}", errors.join("\n")));
    }

    if !errors.is_empty() {
        eprintln!(
            "[tradfi_quotes] {} errors (partial failure): {:?}",
            errors.len(),
            errors
        );
    }

    Ok(results)
}
