import urllib.request
import json
import time
import datetime

def test_yahoo_crypto(symbol, start_date='2024-01-01', end_date='2026-06-30'):
    start_ts = int(datetime.datetime.strptime(start_date, '%Y-%m-%d').timestamp())
    end_ts = int(datetime.datetime.strptime(end_date, '%Y-%m-%d').timestamp())
    
    url = f'https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?period1={start_ts}&period2={end_ts}&interval=1d'
    
    try:
        req = urllib.request.Request(url)
        req.add_header('User-Agent', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36')
        req.add_header('Accept', '*/*')
        req.add_header('Accept-Language', 'en-US,en;q=0.9')
        req.add_header('Referer', 'https://finance.yahoo.com')
        req.add_header('Origin', 'https://finance.yahoo.com')
        
        with urllib.request.urlopen(req, timeout=15) as response:
            raw = response.read().decode()
            data = json.loads(raw)
            
        result = data['chart']['result'][0]
        timestamps = result.get('timestamp', [])
        closes = result['indicators']['quote'][0].get('close', [])
        
        valid_data = [(ts, cl) for ts, cl in zip(timestamps, closes) if cl is not None]
        
        print(f'[OK] {symbol}: {len(valid_data)} data points')
        if valid_data:
            first_dt = datetime.datetime.utcfromtimestamp(valid_data[0][0]).strftime('%Y-%m-%d')
            last_dt = datetime.datetime.utcfromtimestamp(valid_data[-1][0]).strftime('%Y-%m-%d')
            print(f'      First: {first_dt} @ ${valid_data[0][1]:.2f}')
            print(f'      Last:  {last_dt} @ ${valid_data[-1][1]:.2f}')
        return True
        
    except Exception as e:
        print(f'[FAIL] {symbol}: {e}')
        return False

print('=== Testing Yahoo Finance API for Crypto History ===')
print()

symbols = [
    'BTC-USD',
    'ETH-USD',
    'SOL-USD',
    'BNB-USD',
    'XRP-USD',
]

results = {}
for sym in symbols:
    results[sym] = test_yahoo_crypto(sym)
    time.sleep(0.3)

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
