import urllib.request
import json
import time

def test_coingecko_crypto(coin_id, days=180):
    """
    Test CoinGecko API for crypto history.
    coin_id: 'bitcoin', 'ethereum', 'solana', etc.
    days: number of days of history (max 365 for free tier)
    """
    url = f'https://api.coingecko.com/api/v3/coins/{coin_id}/market_chart?vs_currency=usd&days={days}&interval=daily'
    
    try:
        req = urllib.request.Request(url)
        req.add_header('User-Agent', 'Mozilla/5.0')
        
        with urllib.request.urlopen(req, timeout=15) as response:
            data = json.loads(response.read().decode())
            
        prices = data.get('prices', [])
        
        if not prices:
            print(f'[FAIL] {coin_id}: No data returned')
            return False
        
        # prices format: [[timestamp_ms, price], ...]
        first_ts = prices[0][0] / 1000
        last_ts = prices[-1][0] / 1000
        import datetime
        first_date = datetime.datetime.utcfromtimestamp(first_ts).strftime('%Y-%m-%d')
        last_date = datetime.datetime.utcfromtimestamp(last_ts).strftime('%Y-%m-%d')
        
        print(f'[OK] {coin_id}: {len(prices)} data points')
        print(f'      First: {first_date} @ ${prices[0][1]:.2f}')
        print(f'      Last:  {last_date} @ ${prices[-1][1]:.2f}')
        return True
        
    except Exception as e:
        print(f'[FAIL] {coin_id}: {e}')
        return False

print('=== Testing CoinGecko API for Crypto History ===')
print('(Free tier: max 365 days, rate limit ~10-50 req/min)')
print()

coin_ids = [
    'bitcoin',
    'ethereum',
    'solana',
    'binancecoin',
    'ripple',
    'cardano',
    'dogecoin',
]

results = {}
for cid in coin_ids:
    results[cid] = test_coingecko_crypto(cid, days=365)
    time.sleep(0.2)

print()
print('=== Summary ===')
ok_count = 0
for cid, ok in results.items():
    status = 'OK' if ok else 'FAIL'
    print(f'  [{status}] {cid}')
    if ok:
        ok_count += 1

print()
print(f'Result: {ok_count}/{len(coin_ids)} coins passed')
print()

if ok_count == len(coin_ids):
    print('✓ CoinGecko API is fully available for crypto history!')
    print('  Recommendation: Use CoinGecko as primary source for crypto historical data.')
else:
    print('Some coins failed. Check API rate limit or network.')
