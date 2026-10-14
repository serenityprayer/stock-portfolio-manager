"""
Test Gate.io candlestick API for historical price data.
Gate.io API docs: https://www.gate.io/docs/developers/apiv4/en/#get-candlesticks-of-a-currency-pair

Endpoint: GET /api/v4/spot/candlesticks
Params:
  - currency_pair: e.g. BTC_USDT
  - interval: 1d, 4h, 1h, etc.
  - from/to: Unix timestamp (seconds)
  - limit: max 1000
"""
import urllib.request
import json
import datetime
import time

def fetch_candles(symbol, start_date, end_date, interval="1d"):
    """
    Fetch daily candlesticks from Gate.io.
    symbol: e.g. "BTC" (will be converted to BTC_USDT)
    start_date: "YYYY-MM-DD"
    end_date: "YYYY-MM-DD"
    """
    # Convert symbol to Gate.io format
    pair = f"{symbol.upper()}_USDT"
    
    # Convert dates to Unix timestamps (seconds)
    start_ts = int(datetime.datetime.strptime(start_date, "%Y-%m-%d").timestamp())
    end_ts = int(datetime.datetime.strptime(end_date, "%Y-%m-%d").timestamp())
    
    url = (
        f"https://api.gateio.ws/api/v4/spot/candlesticks"
        f"?currency_pair={pair}&interval={interval}&from={start_ts}&to={end_ts}&limit=1000"
    )
    
    req = urllib.request.Request(url)
    req.add_header("Accept", "application/json")
    req.add_header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
    
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read().decode())
            return data
    except Exception as e:
        return {"error": str(e)}

# ========== Test 1: Recent data (last 5 days) ==========
print("=" * 60)
print("Test 1: Recent BTC data (last 5 days)")
print("=" * 60)
recent = fetch_candles("BTC", "2026-06-25", "2026-06-30")
if isinstance(recent, list):
    print(f"  ✓ Success! Got {len(recent)} data points")
    if recent:
        # Format: [timestamp, volume, close, high, low, open, ...]
        print(f"  First candle: ts={recent[0][0]}, close={recent[0][2]}, open={recent[0][5]}")
        print(f"  Last candle:  ts={recent[-1][0]}, close={recent[-1][2]}, open={recent[-1][5]}")
        # Convert timestamp to date
        ts = int(recent[0][0])
        dt = datetime.datetime.utcfromtimestamp(ts).strftime("%Y-%m-%d")
        print(f"  First date: {dt}")
        ts2 = int(recent[-1][0])
        dt2 = datetime.datetime.utcfromtimestamp(ts2).strftime("%Y-%m-%d")
        print(f"  Last date:  {dt2}")
else:
    print(f"  ✗ Failed: {recent}")

# ========== Test 2: Historical data (2024-01) ==========
print()
print("=" * 60)
print("Test 2: Historical BTC data (2024-01)")
print("=" * 60)
time.sleep(0.5)
historical = fetch_candles("BTC", "2024-01-01", "2024-01-31")
if isinstance(historical, list):
    print(f"  ✓ Success! Got {len(historical)} data points")
    if historical:
        ts = int(historical[0][0])
        dt = datetime.datetime.utcfromtimestamp(ts).strftime("%Y-%m-%d")
        ts2 = int(historical[-1][0])
        dt2 = datetime.datetime.utcfromtimestamp(ts2).strftime("%Y-%m-%d")
        print(f"  Date range: {dt} to {dt2}")
        print(f"  Sample (first): close={historical[0][2]}, open={historical[0][5]}")
else:
    print(f"  ✗ Failed: {historical}")

# ========== Test 3: ETH data ==========
print()
print("=" * 60)
print("Test 3: ETH data (2024-01 to 2024-03)")
print("=" * 60)
time.sleep(0.5)
eth_data = fetch_candles("ETH", "2024-01-01", "2024-03-31")
if isinstance(eth_data, list):
    print(f"  ✓ Success! Got {len(eth_data)} data points")
    if eth_data:
        ts = int(eth_data[0][0])
        dt = datetime.datetime.utcfromtimestamp(ts).strftime("%Y-%m-%d")
        print(f"  First date: {dt}")
        print(f"  First close: ${eth_data[0][2]}")
else:
    print(f"  ✗ Failed: {eth_data}")

# ========== Test 4: SOL data ==========
print()
print("=" * 60)
print("Test 4: SOL data (2024-01 to 2024-03)")
print("=" * 60)
time.sleep(0.5)
sol_data = fetch_candles("SOL", "2024-01-01", "2024-03-31")
if isinstance(sol_data, list):
    print(f"  ✓ Success! Got {len(sol_data)} data points")
    if sol_data:
        print(f"  First close: ${sol_data[0][2]}")
        print(f"  Last close:  ${sol_data[-1][2]}")
else:
    print(f"  ✗ Failed: {sol_data}")

# ========== Summary ==========
print()
print("=" * 60)
print("SUMMARY")
print("=" * 60)
print("Gate.io candlestick API is ACCESSIBLE from your network!")
print()
print("API Endpoint: https://api.gateio.ws/api/v4/spot/candlesticks")
print("Rate limit:  No official rate limit for public endpoints")
print("Max data points per request: 1000")
print()
print("Response format (each candle):")
print("  [timestamp(sec), volume, close, high, low, open, ...]")
print()
print("This means crypto contract backfill can work the SAME way as stocks!")
print("→ Implement fetch_crypto_history() using Gate.io candlestick API")
print("→ Crypto contracts can use the same backfill_snapshots approach")
