import urllib.request
import json
import time
import datetime

def test_binance_crypto(symbol='BTCUSDT', limit=365):
    """
    Test Binance Public API for crypto history.
    symbol: 'BTCUSDT', 'ETHUSDT', 'SOLUSDT', etc.
    limit: number of daily candles (max 1000)
    """
    url = f'https://api.binance.com/api/v3/klines?symbol={symbol}&interval=1d&limit={limit}'
    
    try:
        req = urllib.request.Request(url)
        req.add_header('User-Agent', 'Mozilla/5.0')
        
        with urllib.request.urlopen(req, timeout=15) as response:
            data = json.loads(response.read().decode())
            
        if not data or not isinstance(data, list):
            print(f'[FAIL] {symbol}: No data returned')
            return False
        
        # Binance kline format:
        # [timestamp, open, high, low, close, volume, close_time, ...]
        valid_data = [(d[0] // 1000, float(d[4])) for d in data if d[4]]
        
        if not valid_data:
            print(f'[FAIL] {symbol}: No valid price data')
            return False
        
        first_dt = datetime.datetime.utcfromtimestamp(valid_data[0][0]).strftime('%Y-%m-%d')
        last_dt = datetime.datetime.utcfromtimestamp(valid_data[-1][0]).strftime('%Y-%m-%d')
        
        print(f'[OK] {symbol}: {len(valid_data)} data points')
        print(f'      First: {first_dt} @ ${valid_data[0][1]:.2f}')
        print(f'      Last:  {last_dt} @ ${valid_data[-1][1]:.2f}')
        return True
        
    except Exception as e:
        print(f'[FAIL] {symbol}: {e}')
        return False

print('=== Testing Binance Public API for Crypto History ===')
print('(No authentication required, free tier: 1000 requests/min)')
print()

# Binance symbols (USDT-margin contracts)
symbols = [
    'BTCUSDT',   # Bitcoin
    'ETHUSDT',   # Ethereum
    'SOLUSDT',   # Solana
    'BNBUSDT',   # Binance Coin
    'XRPUSDT',   # Ripple
    'ADAUSDT',   # Cardano
    'DOGEUSDT',  # Dogecoin
]

results = {}
for sym in symbols:
    results[sym] = test_binance_crypto(sym, limit=365)
    time.sleep(0.1)

print()
print('=== Summary ===')
ok_count = 0
for sym, ok in results.items():
    status = 'OK' if ok else 'FAIL'
    print(f'  [{status}] {sym}')
    if ok:
        ok_count += 1

print()
print(f'Result: {ok_count}/{len(symbols)} symbols passed')
print()

if ok_count == len(symbols):
    print('✓ Binance Public API is fully available for crypto history!')
    print('  Recommendation: Use Binance as PRIMARY source for crypto historical data.')
    print('  Symbol mapping: BTC → BTCUSDT, ETH → ETHUSDT, etc.')
elif ok_count > 0:
    print(f'⚠ Partial success ({ok_count}/{len(symbols)}). Check failed symbols.')
else:
    print('✗ All symbols failed. Check network connection.')
