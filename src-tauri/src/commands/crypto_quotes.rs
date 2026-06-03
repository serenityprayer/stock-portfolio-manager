use crate::models::StockQuote;
use crate::services::http_client::general_client;
use chrono::Utc;
use reqwest::header;
use std::collections::HashMap;
use std::time::Duration;

/// Map user symbol (e.g. "BTC") to Gate.io currency pair (e.g. "BTC_USDT").
/// Supports common suffixes: BTC->BTC_USDT, ETH->ETH_USDT
/// If input already contains `_`, treat as already in Gate.io format.
fn to_gateio_pair(symbol: &str) -> String {
    let s = symbol.to_uppercase();
    if s.contains('_') {
        return s; // already in BASE_QUOTE format
    }
    // Common quote currencies to try
    // Most altcoins are paired with USDT on Gate.io
    format!("{}_USDT", s)
}

/// Gate.io API response item (simplified, only fields we need)
#[derive(Debug, serde::Deserialize)]
struct GateioTicker {
    currency_pair: String,
    last: String,
    change_percentage: String,
    high_24h: String,
    low_24h: String,
    base_volume: String,
}

/// Fetch a single ticker from Gate.io spot API.
/// Gate.io public API requires no API key and is accessible from China.
/// Endpoint: GET /api/v4/spot/tickers?currency_pair=BTC_USDT
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

    // Response is a JSON array: [ { ... } ]
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

    // Calculate previous_close from current_price and change_percent
    // change_percent = (current - previous) / previous * 100
    // => previous = current / (1 + change_percent / 100)
    let previous_close = if change_percent != 0.0 && current_price > 0.0 {
        current_price / (1.0 + change_percent / 100.0)
    } else {
        current_price
    };
    let change = current_price - previous_close;

    // Use currency_pair as name (e.g. "BTC_USDT")
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

/// Fetch quotes for multiple crypto symbols.
/// Gate.io doesn't support batch requests with multiple pairs in one call,
/// so we fetch them sequentially with a small delay to avoid rate limiting.
#[tauri::command]
pub async fn fetch_crypto_quotes(
    symbols: String, // comma-separated, e.g. "BTC,ETH"
) -> Result<HashMap<String, serde_json::Value>, String> {
    let symbols_vec: Vec<&str> = symbols.split(',').map(|s| s.trim()).collect();
    let mut results: HashMap<String, serde_json::Value> = HashMap::new();
    let mut errors: Vec<String> = Vec::new();

    for (i, symbol) in symbols_vec.iter().enumerate() {
        // Small delay between requests to be gentle on rate limits
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        match fetch_from_gateio(symbol).await {
            Ok(quote) => {
                eprintln!(
                    "[crypto_quotes] {} quote from Gate.io: price={}",
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
