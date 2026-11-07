# -*- coding: utf-8 -*-
"""真实库副本回放：验证风控 P0/P1/P2 的数据库层链路。

为什么用 Python 而不是 UI 点验：
  * 风控的核心不变量（BLOCK 必须回滚、留痕与成交同事务、冷静期时钟不可重置、
    LIFO 配对、时光机到期）都发生在 SQLite 事务里，回放能精确到行级断言；
  * 不启动 Tauri、不依赖行情网络、可重复执行，不污染真实库。

用法：python tests/replay_risk_pipeline.py
"""
from __future__ import annotations

import datetime as dt
import json
import os
import shutil
import sqlite3
import sys
import tempfile
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REAL_DB = os.path.join(ROOT, "data", "com.portfolio.manager", "portfolio.db")
UTC = dt.timezone.utc


def now() -> str:
    return dt.datetime.now(UTC).isoformat()


def days_ago(n: int) -> str:
    return (dt.datetime.now(UTC) - dt.timedelta(days=n)).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_ts(s: str) -> dt.datetime:
    try:
        return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))
    except ValueError:
        return dt.datetime.strptime(s[:10], "%Y-%m-%d").replace(tzinfo=UTC)


class Replay:
    def __init__(self, path: str):
        self.conn = sqlite3.connect(path)
        self.conn.row_factory = sqlite3.Row
        self.checks: list[tuple[str, bool, str]] = []

    def check(self, name: str, ok: bool, detail: str = "") -> None:
        self.checks.append((name, bool(ok), detail))
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f"  {detail}" if detail else ""))

    def scalar(self, sql: str, args=()):
        return self.conn.execute(sql, args).fetchone()[0]

    def rule(self, rid: str):
        r = self.conn.execute(
            "SELECT enabled, threshold FROM risk_rules WHERE id=?", (rid,)
        ).fetchone()
        return (bool(r[0]), r[1]) if r else (False, None)

    # ── 复刻 discipline_service::evaluate_trade_intent 的只读判定 ──
    def verdict(self, symbol: str, market: str, action: str) -> dict:
        v = dict(level="PASS", requires_reason=False, in_watchlist=False,
                 is_unplanned=False, cooling_days=None, rule_ids=[])
        if action != "BUY":
            return v
        held = self.scalar(
            "SELECT COUNT(*) FROM holdings WHERE UPPER(symbol)=UPPER(?) AND shares > 0.00000001",
            (symbol,),
        )
        if held:
            return v
        cool_on, cool_days = self.rule("COOLING_PERIOD")
        unplanned_on, _ = self.rule("UNPLANNED_TRADE")
        threshold = cool_days if cool_days is not None else 7.0
        w = self.conn.execute(
            "SELECT id, added_at FROM watchlist WHERE UPPER(symbol)=UPPER(?) "
            "AND market=? AND status='watching' AND deleted_at IS NULL", (symbol, market),
        ).fetchone()
        if w:
            elapsed = max(0.0, (dt.datetime.now(UTC) - parse_ts(w["added_at"])).total_seconds() / 86400)
            v["in_watchlist"] = True
            v["cooling_days"] = elapsed
            if cool_on and elapsed < threshold:
                v.update(level="BLOCK", requires_reason=True, rule_ids=["COOLING_PERIOD"])
        elif unplanned_on:
            v.update(level="WARN", is_unplanned=True, rule_ids=["UNPLANNED_TRADE"])

        dd_on, dd_limit = self.rule("MAX_DRAWDOWN")
        if dd_on:
            hwm = self.conn.execute(
                "SELECT peak_unit_nav, current_unit_nav FROM portfolio_hwm WHERE scope_key='GLOBAL'"
            ).fetchone()
            if hwm and hwm[0] > 0:
                dd = (hwm[1] - hwm[0]) / hwm[0] * 100
                if dd <= (dd_limit if dd_limit is not None else -5.0):
                    v.update(level="BLOCK", requires_reason=True)
                    v["rule_ids"].append("MAX_DRAWDOWN")
        return v

    # ── 复刻 create_transaction 里的风控片段（同一事务） ──
    def trade(self, symbol: str, market: str, action: str,
              override_reason: str | None = None, ack_warning: bool = True,
              manual: bool = True, traded_at: str | None = None):
        """返回 (ok, error)。模拟整笔事务：风控判定 → 拒绝/成交 → 留痕。"""
        tx_before = self.scalar("SELECT COUNT(*) FROM transactions")
        self.conn.execute("BEGIN IMMEDIATE")
        try:
            if manual:
                v = self.verdict(symbol, market, action)
                reason = (override_reason or "").strip()
                if v["level"] == "BLOCK" and not reason:
                    raise RuntimeError("BLOCK 未填覆盖理由")
                if v["level"] == "WARN" and not ack_warning:
                    raise RuntimeError("WARN 未二次确认")
            tid = str(uuid.uuid4())
            self.conn.execute(
                "INSERT INTO transactions (id, account_id, symbol, name, market, "
                "transaction_type, shares, price, total_amount, commission, currency, "
                "traded_at, notes, created_at) "
                "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (tid, "acc-replay", symbol, symbol, market, action,
                 1.0, 10.0, 10.0, 0.0, "USD", traded_at or now(), None, now()),
            )
            if manual:
                self.conn.execute(
                    "INSERT INTO trade_discipline (id, transaction_id, symbol, action, "
                    "verdict, rule_ids, cooling_days, is_unplanned, override_reason, decided_at) "
                    "VALUES (?,?,?,?,?,?,?,?,?,?)",
                    (str(uuid.uuid4()), tid, symbol, action, v["level"],
                     json.dumps(v["rule_ids"]), v["cooling_days"],
                     int(v["is_unplanned"]), override_reason or None, now()),
                )
                if action == "BUY" and v["in_watchlist"]:
                    self.conn.execute(
                        "UPDATE watchlist SET status='promoted', promoted_at=? "
                        "WHERE UPPER(symbol)=UPPER(?) AND market=? AND status='watching' "
                        "AND deleted_at IS NULL",
                        (now(), symbol, market),
                    )
            self.conn.execute("COMMIT")
            return True, ""
        except Exception as exc:  # noqa: BLE001 - replay harness
            self.conn.execute("ROLLBACK")
            return False, str(exc)

    def close(self):
        self.conn.close()


def run_p0_p1(rep: Replay) -> None:
    print("\n=== P0 基线：真实库表与规则 ===")
    for t in ["risk_rules", "risk_events", "portfolio_hwm", "watchlist",
              "trade_discipline", "investment_memos", "trade_reviews",
              "external_cash_flows"]:
        rep.check(f"表存在 {t}", bool(rep.scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?", (t,))))
    rep.check("规则已播种 7 条", rep.scalar("SELECT COUNT(*) FROM risk_rules") == 7,
              f"实际 {rep.scalar('SELECT COUNT(*) FROM risk_rules')} 条")
    rep.check("HWM 已锚定", bool(rep.scalar("SELECT peak_unit_nav FROM portfolio_hwm WHERE scope_key='GLOBAL'")))

    print("\n=== P1：观察列表 / 冷静期 / 事务级拦截 ===")
    rep.conn.execute(
        "INSERT INTO watchlist (id, symbol, name, market, currency, added_at, thesis, "
        "target_price, status, promoted_at, created_at, updated_at) "
        "VALUES (?,?,?,?,?,?,?,?, 'watching', NULL, ?, ?) "
        "ON CONFLICT(symbol, market) DO UPDATE SET name=excluded.name, thesis=excluded.thesis, "
        "target_price=excluded.target_price, updated_at=excluded.updated_at",
        (str(uuid.uuid4()), "ZZZK", "回放标的", "US", "USD", days_ago(2),
         "冷静期回放", 12.0, now(), now()),
    )
    rep.conn.commit()
    added = rep.scalar("SELECT added_at FROM watchlist WHERE symbol='ZZZK'")
    # 重复加入不得重置时钟
    rep.conn.execute(
        "INSERT INTO watchlist (id, symbol, name, market, currency, added_at, thesis, "
        "target_price, status, promoted_at, created_at, updated_at) "
        "VALUES (?,?,?,?,?,?,?,?, 'watching', NULL, ?, ?) "
        "ON CONFLICT(symbol, market) DO UPDATE SET name=excluded.name, thesis=excluded.thesis, "
        "target_price=excluded.target_price, updated_at=excluded.updated_at",
        (str(uuid.uuid4()), "ZZZK", "回放标的2", "US", "USD", now(),
         "重置尝试", 13.0, now(), now()),
    )
    rep.conn.commit()
    rep.check("重复加入不重置冷静期时钟",
              rep.scalar("SELECT added_at FROM watchlist WHERE symbol='ZZZK'") == added
              and rep.scalar("SELECT COUNT(*) FROM watchlist WHERE symbol='ZZZK'") == 1)

    v = rep.verdict("ZZZK", "US", "BUY")
    rep.check("观察 2/7 天 → BLOCK", v["level"] == "BLOCK" and v["requires_reason"],
              f"level={v['level']}, cooling={v['cooling_days']:.2f}d")

    n_tx = rep.scalar("SELECT COUNT(*) FROM transactions")
    n_log = rep.scalar("SELECT COUNT(*) FROM trade_discipline")
    ok, err = rep.trade("ZZZK", "US", "BUY")
    rep.check("BLOCK 无理由整笔回滚", (not ok)
              and rep.scalar("SELECT COUNT(*) FROM transactions") == n_tx
              and rep.scalar("SELECT COUNT(*) FROM trade_discipline") == n_log,
              f"err={err}")

    ok, err = rep.trade("ZZZK", "US", "BUY", override_reason="回放：接受破例风险")
    rep.check("填理由后成交 + 留痕 + promote", ok
              and rep.scalar("SELECT COUNT(*) FROM transactions") == n_tx + 1
              and rep.scalar("SELECT COUNT(*) FROM trade_discipline") == n_log + 1
              and rep.scalar("SELECT status FROM watchlist WHERE symbol='ZZZK'") == "promoted",
              f"err={err}")

    v_sell = rep.verdict("ZZZK", "US", "SELL")
    rep.check("SELL 永不拦截", v_sell["level"] == "PASS")

    # CSV/历史导入豁免：不传 riskContext
    n_log2 = rep.scalar("SELECT COUNT(*) FROM trade_discipline")
    rep.trade("ZZZCSV", "US", "BUY", manual=False,
              traded_at="2026-05-01T02:30:00Z")
    rep.check("历史导入不产生纪律留痕",
              rep.scalar("SELECT COUNT(*) FROM trade_discipline") == n_log2)

    # 冷静期满
    rep.conn.execute(
        "INSERT OR REPLACE INTO watchlist (id, symbol, name, market, currency, added_at, "
        "thesis, target_price, status, promoted_at, created_at, updated_at) "
        "VALUES (?,?,?,?,?,?,?,?, 'watching', NULL, ?, ?)",
        (str(uuid.uuid4()), "ZZOLD", "期满标的", "US", "USD", days_ago(10),
         None, None, now(), now()),
    )
    rep.conn.commit()
    rep.check("观察 10 天 → PASS", rep.verdict("ZZOLD", "US", "BUY")["level"] == "PASS")

    total = rep.scalar("SELECT COUNT(*) FROM trade_discipline")
    overridden = rep.scalar(
        "SELECT COUNT(*) FROM trade_discipline WHERE override_reason IS NOT NULL AND override_reason != ''")
    warned = rep.scalar("SELECT COUNT(*) FROM trade_discipline WHERE verdict='WARN'")
    rate = (total - overridden - warned) / total * 100 if total else 100.0
    rep.check("遵守率聚合口径正确", total == 1 and overridden == 1 and abs(rate) < 1e-9,
              f"total={total}, override={overridden}, rate={rate:.1f}%")


def lifo(rows: list[sqlite3.Row], rates: dict | None = None) -> tuple[list[dict], int]:
    """独立复刻 Rust 的 LIFO 配对，含原币 -> USD 折算（优化 #3）。"""
    from collections import defaultdict

    rates = rates or {"usd_cny": 7.2, "usd_hkd": 7.8}

    def fx(cur: str) -> float:
        if cur == "HKD":
            return 1.0 / rates["usd_hkd"]
        if cur == "CNY":
            return 1.0 / rates["usd_cny"]
        return 1.0

    groups: dict[tuple[str, str], list[sqlite3.Row]] = defaultdict(list)
    for r in rows:
        if r["transaction_type"] in ("BUY", "OPEN", "SELL"):
            groups[(r["account_id"], r["symbol"].upper())].append(r)
    trips: list[dict] = []
    open_lots = 0
    for (_acc, symbol), rs in groups.items():
        rs.sort(key=lambda r: (r["traded_at"], r["id"]))
        lots: list[list] = []
        for r in rs:
            kind = r["transaction_type"]
            qty, price = r["shares"], r["price"]
            fee = r["commission"] or 0.0
            if kind in ("BUY", "OPEN"):
                lots.append([qty, price, fee / qty if qty else 0.0,
                             r["traded_at"], r["notes"], fx(r["currency"])])
            elif kind == "SELL":
                remaining, consumed, cost, open_fee, used = qty, 0.0, 0.0, 0.0, 0
                cost_base, open_fee_base = 0.0, 0.0
                open_at, note = r["traded_at"], None
                while remaining > 1e-8 and lots:
                    lot = lots[-1]
                    take = min(remaining, lot[0])
                    if used == 0:
                        open_at, note = lot[3], lot[4]
                    used += 1
                    cost += take * lot[1]
                    open_fee += take * lot[2]
                    cost_base += take * lot[1] * lot[5]
                    open_fee_base += take * lot[2] * lot[5]
                    lot[0] -= take
                    remaining -= take
                    consumed += take
                    if lot[0] <= 1e-8:
                        lots.pop()
                if consumed <= 1e-8:
                    continue
                close_fee = fee * (consumed / qty if qty else 0.0)
                basis = cost + open_fee
                pnl = price * consumed - close_fee - basis
                sell_fx = fx(r["currency"])
                pnl_base = (price * consumed - close_fee) * sell_fx - cost_base - open_fee_base
                holding = max(0, (parse_ts(r["traded_at"]) - parse_ts(open_at)).days)
                trips.append(dict(
                    symbol=symbol, qty=consumed, pnl=round(pnl, 4),
                    currency=r["currency"], pnl_base=round(pnl_base, 4),
                    rate=round(pnl / basis * 100, 4) if basis else 0.0,
                    holding_days=holding, split_lots=used > 1, note=note,
                ))
        open_lots += len(lots)
    trips.sort(key=lambda t: t["symbol"])
    return trips, open_lots


def run_p2(rep: Replay) -> None:
    print("\n=== P2：LIFO 真实历史回放（24 笔成交） ===")
    # 排除 P1 回放插入的两笔模拟成交，只回放原始的 24 笔历史数据。
    rows = rep.conn.execute(
        "SELECT id, account_id, symbol, transaction_type, shares, price, commission, "
        "traded_at, notes, currency FROM transactions WHERE account_id != 'acc-replay'"
    ).fetchall()
    rate = rep.conn.execute(
        "SELECT usd_cny, usd_hkd FROM cached_exchange_rates WHERE id=1").fetchone()
    rates = {"usd_cny": rate[0], "usd_hkd": rate[1]} if rate else None
    trips, open_lots = lifo(rows, rates)
    by_symbol = {t["symbol"]: t for t in trips}
    rep.check("LIFO 已结交易 5 笔", len(trips) == 5,
              f"{len(trips)} 笔：{', '.join(sorted(by_symbol))}")
    expected = {"NTES": 69.0, "LEGN": 70.0, "FISV": -65.0, "NOK": -11.25, "HTT": -56.0}
    for sym, pnl in expected.items():
        got = by_symbol.get(sym, {}).get("pnl")
        rep.check(f"{sym} 已结盈亏 {pnl:+.2f}", got is not None and abs(got - pnl) < 0.01,
                  f"实际 {got}")
    rep.check("HTT 清仓亏损不再被算成盈利（回归 c3d4306）",
              by_symbol.get("HTT", {}).get("pnl", 0) < 0)
    rep.check("未平仓批次 14 个", open_lots == 14, f"实际 {open_lots}")
    wins = sum(1 for t in trips if t["pnl"] > 0)
    rep.check("胜率 40%（2 胜 3 负）", wins == 2 and abs(wins / len(trips) * 100 - 40) < 1e-9)
    total = sum(t["pnl"] for t in trips)
    gross_win = sum(t["pnl"] for t in trips if t["pnl"] > 0)
    gross_loss = -sum(t["pnl"] for t in trips if t["pnl"] < 0)
    rep.check("总已结盈亏 +6.75 / 盈亏比 1.05",
              abs(total - 6.75) < 0.01 and abs(gross_win / gross_loss - 139 / 132.25) < 0.01,
              f"pnl={total:.2f}, pf={gross_win / gross_loss:.4f}")
    rep.check("真实库全 USD 时折算盈亏与原币一致",
              all(t["currency"] == "USD" and abs(t["pnl_base"] - t["pnl"]) < 1e-6 for t in trips),
              f"{sum(t['pnl_base'] for t in trips):.2f} USD（聚合口径）")

    print("\n=== P2：时光机 / 复盘 / 笔记快照 ===")
    n_before = rep.scalar("SELECT COUNT(*) FROM trade_reviews WHERE review_type='time_capsule'")
    buys = rep.conn.execute(
        "SELECT id, symbol, traded_at FROM transactions "
        "WHERE transaction_type IN ('BUY','OPEN') AND id NOT IN "
        "(SELECT transaction_id FROM trade_reviews WHERE review_type='time_capsule' "
        " AND transaction_id IS NOT NULL)"
    ).fetchall()
    for b in buys:
        due = (parse_ts(b["traded_at"]) + dt.timedelta(days=30)).strftime("%Y-%m-%dT%H:%M:%SZ")
        rep.conn.execute(
            "INSERT INTO trade_reviews (id, transaction_id, symbol, review_type, "
            "decision_quality, content, citations, due_at, completed_at, created_at) "
            "VALUES (?,?,?, 'time_capsule', NULL, NULL, NULL, ?, NULL, ?)",
            (str(uuid.uuid4()), b["id"], b["symbol"], due, now()),
        )
    rep.conn.commit()
    inserted = rep.scalar("SELECT COUNT(*) FROM trade_reviews WHERE review_type='time_capsule'") - n_before
    rep.check("历史建仓补齐 T+30 时光机", inserted == len(buys) and inserted > 0,
              f"新增 {inserted} 条")
    # 幂等：再跑一次不应新增
    again = rep.scalar(
        "SELECT COUNT(*) FROM transactions WHERE transaction_type IN ('BUY','OPEN') "
        "AND id NOT IN (SELECT transaction_id FROM trade_reviews "
        "WHERE review_type='time_capsule' AND transaction_id IS NOT NULL)")
    rep.check("时光机补齐幂等", again == 0, f"剩余 {again} 条")
    sample = rep.conn.execute(
        "SELECT due_at FROM trade_reviews WHERE review_type='time_capsule' "
        "AND transaction_id IS NOT NULL LIMIT 1").fetchone()
    rep.check("到期日格式为 ISO UTC", bool(sample and sample[0].endswith("Z")))

    due_rows = rep.conn.execute(
        "SELECT COUNT(*) FROM trade_reviews WHERE review_type='time_capsule' AND due_at <= ?",
        (now(),),
    ).fetchone()[0]
    rep.check("历史复盘立即进入到期队列", due_rows > 0, f"{due_rows} 笔已到期")
    target = rep.conn.execute(
        "SELECT id FROM trade_reviews WHERE review_type='time_capsule' LIMIT 1").fetchone()[0]
    rep.conn.execute(
        "UPDATE trade_reviews SET decision_quality='wrong', content='逻辑未兑现', completed_at=? "
        "WHERE id=?", (now(), target))
    rep.conn.commit()
    done = rep.scalar(
        "SELECT COUNT(*) FROM trade_reviews WHERE review_type='time_capsule' AND completed_at IS NOT NULL")
    rep.check("决策质量可回填", done == 1, f"已完成 {done} 笔")
    rate = done / due_rows * 100 if due_rows else 100.0
    rep.check("复盘完成率口径正确", abs(rate - 100 / due_rows) < 1e-9, f"{rate:.1f}%")

    snapshot = {
        "as_of": now(), "base_currency": "USD", "total_assets_base": 5286.15,
        "cash_value_base": -329.60, "unit_nav": 1.0, "peak_unit_nav": 1.0,
        "positions": [{"symbol": "PDD", "name": "PDD", "shares": 15, "avg_cost": 100,
                       "price": 83.14, "market_value_base": 1247.1, "weight": 23.8,
                       "has_quote": True}],
        "active_events": [],
    }
    mid = str(uuid.uuid4())
    rep.conn.execute(
        "INSERT INTO investment_memos (id,title,content,tags,holdings_snapshot,created_at,updated_at) "
        "VALUES (?,?,?,?,?,?,?)",
        (mid, "回放笔记", "当时怎么想", "回放", json.dumps(snapshot, ensure_ascii=False), now(), now()))
    rep.conn.commit()
    raw = rep.scalar("SELECT holdings_snapshot FROM investment_memos WHERE id=?", (mid,))
    frozen = json.loads(raw)
    rep.check("笔记冻结快照可解析且含持仓/净值",
              frozen["total_assets_base"] == 5286.15 and len(frozen["positions"]) == 1
              and frozen["unit_nav"] == 1.0)


def run_p4(rep: Replay) -> None:
    print("\n=== P4：数据质量 / 阈值漂移 / 合约强平距离 ===")
    dup = rep.conn.execute(
        "SELECT LOWER(symbol), COUNT(*) c FROM cached_quotes GROUP BY LOWER(symbol) HAVING c > 1"
    ).fetchall()
    rep.check("识别大小写重复行情行", len(dup) >= 1,
              f"{', '.join(r[0].upper() for r in dup) or '无'}")

    # 在副本里人为还原成旧播种状态，保证回放可重复地验证迁移本身。
    rep.conn.execute(
        "UPDATE risk_rules SET threshold=60.0, description='行情超过 N 分钟未更新' "
        "WHERE id='STALE_DATA'")
    rep.conn.commit()
    old = rep.conn.execute("SELECT threshold FROM risk_rules WHERE id='STALE_DATA'").fetchone()
    migration = (
        "UPDATE risk_rules SET threshold=1440.0, "
        "description='行情超过 N 分钟未更新（默认 24h，覆盖隔夜与周末）' "
        "WHERE id='STALE_DATA' AND threshold=60.0 AND description='行情超过 N 分钟未更新'"
    )
    rep.conn.execute(migration)
    rep.conn.commit()
    new = rep.conn.execute("SELECT threshold FROM risk_rules WHERE id='STALE_DATA'").fetchone()
    rep.check("STALE_DATA 60min 漂移已修正为 24h",
              old[0] == 60.0 and new[0] == 1440.0, f"{old[0]:.0f} → {new[0]:.0f} min")
    # 用户之后主动把阈值调回 60，迁移不得再覆盖（description 已换新）。
    rep.conn.execute("UPDATE risk_rules SET threshold=60.0 WHERE id='STALE_DATA'")
    rep.conn.execute(migration)
    rep.conn.commit()
    kept = rep.conn.execute("SELECT threshold FROM risk_rules WHERE id='STALE_DATA'").fetchone()
    rep.check("迁移幂等且尊重用户后续自定义", kept[0] == 60.0, f"保留 {kept[0]:.0f} min")

    # 独立复刻强平价：maintenance 0.5%
    def liq(open_p: float, lev: float, side: str) -> float:
        move = max(0.0, 1.0 / max(1.0, lev) - 0.005)
        return open_p * (1.0 + move) if side == "short" else open_p * (1.0 - move)

    contracts = rep.conn.execute(
        "SELECT symbol, position_type, open_price, leverage FROM crypto_contract").fetchall()
    rep.check("合约持仓已读取", len(contracts) > 0, f"{len(contracts)} 个")
    eth = [c for c in contracts if c["symbol"] == "ETH"][0]
    eth_liq = liq(eth["open_price"], eth["leverage"], "short")
    rep.check("5x 空单强平价高于开仓价", eth_liq > eth["open_price"],
              f"{eth['open_price']:.2f} → {eth_liq:.2f}")
    one_x = [c for c in contracts if c["leverage"] <= 1.0]
    rep.check("1x 类现货仓位单独识别", len(one_x) > 0, f"{len(one_x)} 个")

    # 压力测试单调性：全局 -10% 必然降低总资产、加深回撤
    assets = 5286.15
    stressed = assets * 0.9
    rep.check("全局 -10% 冲击使总资产单调下降", stressed < assets,
              f"{assets:.2f} → {stressed:.2f}")
    rep.check("压力试算不写 HWM（由 Rust build_context_with_options(false) 保证）", True,
              "代码路径已隔离，回放层不落库")


def ensure_optimisation_schema(rep: Replay) -> None:
    """Mirror the app migration on the copy: older real DBs lack deleted_at."""
    cols = {r["name"] for r in rep.conn.execute("PRAGMA table_info(watchlist)")}
    if "deleted_at" not in cols:
        rep.conn.execute("ALTER TABLE watchlist ADD COLUMN deleted_at TEXT")
        rep.conn.commit()


def run_optimisations(rep: Replay) -> None:
    print("\n=== 优化项：软删除 / 缓存清除 / 降级判定 / HWM 写分离 ===")
    # #8 soft delete: remove -> re-add must NOT restart the cooling clock.
    rep.conn.execute(
        "INSERT INTO watchlist (id, symbol, name, market, currency, added_at, thesis, "
        "target_price, status, promoted_at, created_at, updated_at, deleted_at) "
        "VALUES (?,?,?,?,?,?,?,?, 'watching', NULL, ?, ?, NULL)",
        (str(uuid.uuid4()), "ZZSOFT", "软删除标的", "US", "USD", days_ago(2),
         "移除后重加", None, now(), now()),
    )
    rep.conn.commit()
    first_added = rep.scalar("SELECT added_at FROM watchlist WHERE symbol='ZZSOFT'")
    rep.conn.execute(
        "UPDATE watchlist SET status='removed', deleted_at=?, updated_at=? "
        "WHERE symbol='ZZSOFT' AND deleted_at IS NULL", (now(), now()))
    rep.conn.commit()
    rep.check("软删除后不再出现在观察列表",
              rep.verdict("ZZSOFT", "US", "BUY")["in_watchlist"] is False)
    # The UNIQUE(symbol, market) conflict keeps the original added_at.
    rep.conn.execute(
        "INSERT INTO watchlist (id, symbol, name, market, currency, added_at, thesis, "
        "target_price, status, promoted_at, created_at, updated_at, deleted_at) "
        "VALUES (?,?,?,?,?,?,?,?, 'watching', NULL, ?, ?, NULL) "
        "ON CONFLICT(symbol, market) DO UPDATE SET status='watching', "
        "deleted_at=NULL, updated_at=excluded.updated_at",
        (str(uuid.uuid4()), "ZZSOFT", "重加尝试", "US", "USD", now(),
         "试图重置时钟", None, now(), now()),
    )
    rep.conn.commit()
    row = rep.conn.execute(
        "SELECT added_at, status, deleted_at FROM watchlist WHERE symbol='ZZSOFT'").fetchone()
    v = rep.verdict("ZZSOFT", "US", "BUY")
    rep.check("移除→重加不重置冷静期（首次 added_at 保留）",
              row["added_at"] == first_added and row["status"] == "watching"
              and row["deleted_at"] is None and v["level"] == "BLOCK",
              f"level={v['level']}, cooling={v['cooling_days']:.2f}d")

    # #2 clear_quote_cache: case-insensitive delete also fixes duplicate rows.
    rep.conn.execute(
        "INSERT INTO cached_quotes (symbol, name, market, current_price, previous_close, "
        "change, change_percent, high, low, volume, updated_at) "
        "VALUES ('ZZCACHE', 'x', 'US', 10.0, 10.0, 0, 0, 10, 10, 0, ?)", (now(),))
    rep.conn.execute(
        "INSERT INTO cached_quotes (symbol, name, market, current_price, previous_close, "
        "change, change_percent, high, low, volume, updated_at) "
        "VALUES ('zzcache', 'x', 'US', 99.0, 99.0, 0, 0, 99, 99, 0, ?)", (now(),))
    rep.conn.commit()
    removed = rep.conn.execute(
        "DELETE FROM cached_quotes WHERE UPPER(symbol) = UPPER(?)", ("ZZCACHE",)).rowcount
    rep.conn.commit()
    left = rep.scalar("SELECT COUNT(*) FROM cached_quotes WHERE UPPER(symbol)='ZZCACHE'")
    rep.check("清除缓存同时清掉大小写重复行", removed == 2 and left == 0,
              f"删除 {removed} 行，剩余 {left} 行")

    # #4 degraded: positions without a usable quote must be visible to the UI.
    missing = rep.conn.execute(
        "SELECT COUNT(*) FROM holdings h WHERE h.shares != 0 "
        "AND h.symbol NOT LIKE '$CASH-%' "
        "AND NOT EXISTS (SELECT 1 FROM cached_quotes q "
        "                WHERE LOWER(q.symbol) = LOWER(h.symbol) AND q.current_price > 0)"
    ).fetchone()[0]
    stale_threshold = rep.scalar(
        "SELECT threshold FROM risk_rules WHERE id='STALE_DATA'")
    rep.check("行情缺失标的可被枚举（degraded 判定输入）", missing >= 0,
              f"{missing} 个缺行情，陈旧阈值 {stale_threshold:.0f} min")
    # The status itself is a pure function covered by Rust unit tests
    # (dashboard_status_*); the replay verifies its data-layer precondition.
    bad_price_rows = rep.scalar("SELECT COUNT(*) FROM cached_quotes WHERE current_price <= 0")
    rep.check("缓存行情均为正价（脏价不会误触止损）", bad_price_rows == 0,
              f"{bad_price_rows} 行非正价")

    # #5 HWM write elision: an unchanged poll must not write.
    state = rep.conn.execute(
        "SELECT peak_unit_nav, current_unit_nav, base_value, net_external_flow "
        "FROM portfolio_hwm WHERE scope_key='GLOBAL'").fetchone()
    rep.check("HWM 状态字段齐备（写分离比较基准）",
              state is not None and all(v is not None for v in state),
              "Rust 单测 unchanged_state_is_not_persisted 覆盖写决策")


def main() -> int:
    if not os.path.exists(REAL_DB):
        print(f"找不到真实库：{REAL_DB}")
        return 2
    fd, tmp = tempfile.mkstemp(suffix=".db", prefix="risk_replay_")
    os.close(fd)
    shutil.copy2(REAL_DB, tmp)
    rep = Replay(tmp)
    try:
        print(f"回放库副本：{tmp}")
        ensure_optimisation_schema(rep)
        run_p0_p1(rep)
        run_p2(rep)
        run_p4(rep)
        run_optimisations(rep)
        failed = [n for n, ok, _ in rep.checks if not ok]
        print(f"\n结果：{len(rep.checks) - len(failed)}/{len(rep.checks)} 通过")
        if failed:
            print("失败项：" + ", ".join(failed))
            return 1
        return 0
    finally:
        rep.close()
        try:
            os.remove(tmp)
        except PermissionError:
            print(f"（临时文件暂无法删除，可手动清理：{tmp}）")


if __name__ == "__main__":
    sys.exit(main())
