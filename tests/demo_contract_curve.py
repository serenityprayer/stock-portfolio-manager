"""
合约收益曲线对比示例：
- 理想曲线（有历史价格）：平滑曲线
- 方案A（只用平仓记录）：阶跃式曲线
"""
import json
from datetime import datetime, timedelta

# ========== 模拟数据：3笔合约交易 ==========
# 交易1：BTC 5x 做多
# 2024-01-10 开仓，开仓价 $42,000，数量 0.5 BTC，保证金 $4,200
# 2024-03-15 平仓，平仓价 $68,000

# 交易2：ETH 3x 做空
# 2024-02-01 开仓，开仓价 $2,200，数量 10 ETH，保证金 $7,333
# 2024-04-20 平仓，平仓价 $1,800

# 交易3：SOL 2x 做多（未平仓）
# 2024-03-01 开仓，开仓价 $80，数量 100 SOL，保证金 $4,000
# 当前价 $140（未平仓）

trades = [
    {
        "id": "trade_001",
        "symbol": "BTC",
        "direction": "long",
        "open_date": "2024-01-10",
        "open_price": 42000,
        "shares": 0.5,
        "leverage": 5,
        "margin": 4200,  # 42000 * 0.5 / 5
        "close_date": "2024-03-15",
        "close_price": 68000,
        "realized_pnl": (68000 - 42000) * 0.5,  # +$13,000
    },
    {
        "id": "trade_002",
        "symbol": "ETH",
        "direction": "short",
        "open_date": "2024-02-01",
        "open_price": 2200,
        "shares": 10,
        "leverage": 3,
        "margin": 7333,  # 2200 * 10 / 3
        "close_date": "2024-04-20",
        "close_price": 1800,
        "realized_pnl": (2200 - 1800) * 10,  # +$4,000（做空盈利=开仓价-平仓价）
    },
    {
        "id": "trade_003",
        "symbol": "SOL",
        "direction": "long",
        "open_date": "2024-03-01",
        "open_price": 80,
        "shares": 100,
        "leverage": 2,
        "margin": 4000,  # 80 * 100 / 2
        "close_date": None,
        "close_price": None,
        "current_price": 140,
        "unrealized_pnl": (140 - 80) * 100,  # +$6,000
    },
]

# ========== 模拟历史价格（用于生成理想曲线）==========
import random
random.seed(42)

def gen_daily_prices(start_date, end_date, start_price, volatility=0.03):
    """生成模拟日线价格"""
    prices = {}
    current = start_date
    price = start_price
    while current <= end_date:
        # 简单随机游走
        change = random.gauss(0, volatility)
        price = price * (1 + change)
        prices[current.strftime("%Y-%m-%d")] = round(price, 2)
        current += timedelta(days=1)
    return prices

# 生成各标的模拟价格
btc_prices = gen_daily_prices(
    datetime(2024, 1, 10), datetime(2024, 3, 15), 42000, 0.03
)
eth_prices = gen_daily_prices(
    datetime(2024, 2, 1), datetime(2024, 4, 20), 2200, 0.04
)
sol_prices = gen_daily_prices(
    datetime(2024, 3, 1), datetime(2024, 6, 30), 80, 0.05
)
# 当前价格修正
sol_prices["2024-06-30"] = 140

# ========== 计算理想曲线（每日净值）==========
def calc_ideal_curve():
    """有历史价格的理想曲线：每日计算净值"""
    start = datetime(2024, 1, 10)
    end = datetime(2024, 6, 30)
    dates = []
    values = []
    
    current = start
    while current <= end:
        date_str = current.strftime("%Y-%m-%d")
        total_value = 0
        
        for t in trades:
            open_d = datetime.strptime(t["open_date"], "%Y-%m-%d")
            if current < open_d:
                continue  # 还未开仓
            
            if t["close_date"]:
                close_d = datetime.strptime(t["close_date"], "%Y-%m-%d")
                if current > close_d:
                    # 已平仓：净值 = 保证金 + 已实现盈亏（固定）
                    total_value += t["margin"] + t["realized_pnl"]
                    continue
            
            # 持仓中：需要当日价格
            if t["symbol"] == "BTC" and date_str in btc_prices:
                price = btc_prices[date_str]
            elif t["symbol"] == "ETH" and date_str in eth_prices:
                price = eth_prices[date_str]
            elif t["symbol"] == "SOL" and date_str in sol_prices:
                price = sol_prices[date_str]
            else:
                current += timedelta(days=1)
                continue
            
            direction = 1 if t["direction"] == "long" else -1
            pnl = (price - t["open_price"]) * direction * t["shares"]
            total_value += t["margin"] + pnl
        
        dates.append(date_str)
        values.append(round(total_value, 2))
        current += timedelta(days=1)
    
    return dates, values

# ========== 计算方案A曲线（只用平仓记录）==========
def calc_option_a_curve():
    """
    方案A：只用平仓记录
    - 开仓日：净值 = 保证金
    - 平仓日：净值 = 保证金 + 已实现盈亏
    - 未平仓：净值 = 保证金 + 当前未实现盈亏（最后一天）
    → 曲线是阶跃式的，中间是直线
    """
    events = []
    
    for t in trades:
        open_d = t["open_date"]
        # 开仓事件：净值增加保证金
        events.append({
            "date": open_d,
            "type": "open",
            "value_change": t["margin"],  # 投入保证金
            "symbol": t["symbol"],
        })
        
        if t["close_date"]:
            # 平仓事件：净值变化 = 已实现盈亏
            events.append({
                "date": t["close_date"],
                "type": "close",
                "value_change": t["margin"] + t["realized_pnl"],
                "symbol": t["symbol"],
            })
        else:
            # 未平仓：用当前价格估算
            events.append({
                "date": "2024-06-30",  # 当前日期
                "type": "current",
                "value_change": t["margin"] + t["unrealized_pnl"],
                "symbol": t["symbol"],
            })
    
    # 按日期排序
    events.sort(key=lambda x: x["date"])
    
    # 生成阶跃式曲线
    dates = []
    values = []
    running_value = 0
    prev_date = None
    
    for evt in events:
        if prev_date is None:
            prev_date = evt["date"]
            running_value = evt["value_change"]
            dates.append(evt["date"])
            values.append(round(running_value, 2))
        else:
            # 阶跃：从上一事件日期到当前事件日期，净值不变
            # （这就是问题所在：中间的价格波动完全看不到）
            running_value = evt["value_change"]
            dates.append(evt["date"])
            values.append(round(running_value, 2))
        prev_date = evt["date"]
    
    return dates, values, events

# ========== 生成数据 ==========
ideal_dates, ideal_values = calc_ideal_curve()
option_a_dates, option_a_values, option_a_events = calc_option_a_curve()

# ========== 输出 HTML 可视化 ==========
html = f"""
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>合约收益曲线对比：理想 vs 方案A</title>
  <script src="https://cdn.jsdelivr.net/npm/echarts@5/dist/echarts.min.js"></script>
  <style>
    body {{ font-family: sans-serif; padding: 20px; background: #f5f5f5; }}
    .chart-container {{ width: 100%; height: 450px; background: white; border-radius: 8px; padding: 16px; margin-bottom: 20px; }}
    .info {{ background: white; padding: 16px; border-radius: 8px; margin-bottom: 20px; }}
    .warning {{ background: #fff3cd; border-left: 4px solid #ffc107; padding: 12px; border-radius: 4px; margin-top: 12px; }}
    .verdict {{ background: #d4edda; border-left: 4px solid #28a745; padding: 12px; border-radius: 4px; margin-top: 12px; }}
  </style>
</head>
<body>
  <h1>📊 合约收益曲线对比示例</h1>
  
  <div class="info">
    <h3>模拟场景（3笔交易）</h3>
    <ul>
      <li>🟢 BTC 5x 做多：2024-01-10 开仓 @ $42,000 → 2024-03-15 平仓 @ $68,000（盈利 +$13,000）</li>
      <li>🔵 ETH 3x 做空：2024-02-01 开仓 @ $2,200 → 2024-04-20 平仓 @ $1,800（盈利 +$4,000）</li>
      <li>🟡 SOL 2x 做多：2024-03-01 开仓 @ $80 → 当前未平仓 @ $140（浮盈 +$6,000）</li>
    </ul>
  </div>

  <div class="chart-container">
    <h3>✅ 理想曲线（有历史价格数据）</h3>
    <div id="idealChart" style="width:100%;height:380px;"></div>
  </div>

  <div class="chart-container">
    <h3>⚠️ 方案A（只用平仓记录）</h3>
    <div id="optionAChart" style="width:100%;height:380px;"></div>
  </div>

  <div class="info">
    <h3>🔍 方案A 的问题分析</h3>
    <div class="warning">
      <b>问题1：看不到持仓期间的价格波动</b><br>
      方案A只知道开仓和平仓两个时间点的净值，中间的价格波动（可能很大！）完全看不到。
      比如上图中 BTC 在 2024-01 到 2024-03 之间可能有过大幅回撤，但方案A的曲线是直的。
    </div>
    <div class="warning" style="margin-top: 8px;">
      <b>问题2：多个持仓叠加时，曲线失真更严重</b><br>
      如果有5笔合约交叉开平仓，方案A的曲线会变成几段直线，完全无法反映真实净值变化。
    </div>
    <div class="warning" style="margin-top: 8px;">
      <b>问题3：最大回撤无法计算</b><br>
      最大回撤需要每日净值数据才能算。方案A因为没有每日数据，最大回撤永远是0或者严重低估。
    </div>
    <div class="verdict">
      <b>结论：方案A 只能作为"交易记录汇总"，不能叫"收益曲线"。</b><br>
      如果你只是想看"我赚了/亏了多少钱"，方案A够用。<br>
      但如果你想分析策略表现、评估风险，必须要有历史价格数据。
    </div>
  </div>

  <script>
    // ---------- 理想曲线 ----------
    const idealChart = echarts.init(document.getElementById('idealChart'));
    const idealOption = {{
      tooltip: {{ trigger: 'axis' }},
      xAxis: {{ type: 'category', data: {json.dumps(ideal_dates[::7])} }},
      yAxis: {{ type: 'value', name: '净值 ($)' }},
      series: [{{
        name: '合约净值',
        type: 'line',
        data: {json.dumps([v for i,v in enumerate(ideal_values) if i % 7 == 0])},
        smooth: false,
        lineStyle: {{ color: '#E53935', width: 2 }},
        areaStyle: {{ color: 'rgba(229,57,53,0.1)' }},
        itemStyle: {{ color: '#E53935' }}
      }}]
    }};
    idealChart.setOption(idealOption);

    // ---------- 方案A曲线 ----------
    const optionAChart = echarts.init(document.getElementById('optionAChart'));
    // 方案A需要把阶跃数据展开成连续日期
    const allDates = [];
    const allValues = [];
    let cursor = 0;
    const startDate = new Date('2024-01-10');
    const endDate = new Date('2024-06-30');
    for (let d = new Date(startDate); d <= endDate; d.setDate(d.getDate() + 1)) {{
      const ds = d.toISOString().slice(0,10);
      allDates.push(ds);
      // 找到 ds 之前最近的事件值
      let val = 0;
      for (let i = 0; i < {json.dumps(option_a_events)}.length; i++) {{
        if ({json.dumps(option_a_events)}[i].date <= ds) {{
          val = {json.dumps(option_a_events)}[i].value_change;
        }}
      }}
      allValues.push(val);
    }}

    const optionAOption = {{
      tooltip: {{ trigger: 'axis' }},
      xAxis: {{ type: 'category', data: allDates.filter((_,i) => i % 7 === 0) }},
      yAxis: {{ type: 'value', name: '净值 ($)' }},
      series: [{{
        name: '方案A净值（阶跃）',
        type: 'line',
        step: 'middle',  // 阶跃式
        data: allValues.filter((_,i) => i % 7 === 0),
        lineStyle: {{ color: '#FF9800', width: 2 }},
        itemStyle: {{ color: '#FF9800' }}
      }}]
    }};
    optionAChart.setOption(optionAOption);
  </script>
</body>
</html>
"""

with open("F:/WorkSpace/stock-portfolio-manager/tests/contract_curve_demo.html", "w", encoding="utf-8") as f:
    f.write(html)

print("HTML demo generated: tests/contract_curve_demo.html")
print(f"\n理想曲线数据点: {len(ideal_dates)} 个")
print(f"方案A事件点: {len(option_a_events)} 个")
print("\n方案A事件详情:")
for evt in option_a_events:
    print(f"  {evt['date']} | {evt['type']:8s} | {evt['symbol']:4s} | 净值变化: ${evt['value_change']:.0f}")
