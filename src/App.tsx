import { lazy, Suspense, useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Alert, Spin } from "antd";
import { useQuoteStore } from "./stores/quoteStore";
import MainLayout from "./components/Layout/MainLayout";
// The dashboard is the landing page, so it stays in the entry chunk.
// Every other route is loaded on demand — the Risk Center in particular pulls
// in a large amount of Ant Design table/form code that most sessions never use.
import DashboardPage from "./pages/Dashboard";
const AccountsPage = lazy(() => import("./pages/Accounts"));
const HoldingsPage = lazy(() => import("./pages/Holdings"));
const TransactionsPage = lazy(() => import("./pages/Transactions"));
const CategoriesPage = lazy(() => import("./pages/Categories"));
const StatisticsPage = lazy(() => import("./pages/Statistics"));
const PerformancePage = lazy(() => import("./pages/Performance"));
const QuarterlyPage = lazy(() => import("./pages/Quarterly"));
const SnapshotDetail = lazy(() => import("./pages/Quarterly/SnapshotDetail"));
const QuarterComparisonPage = lazy(() => import("./pages/Quarterly/QuarterComparison"));
const TrendsPage = lazy(() => import("./pages/Quarterly/TrendsPage"));
const ImportPage = lazy(() => import("./pages/Import"));
const AlertsPage = lazy(() => import("./pages/Alerts"));
const ReviewPage = lazy(() => import("./pages/Review"));
const RiskPage = lazy(() => import("./pages/Risk"));
const CryptoSpotPage = lazy(() => import("./pages/CryptoSpot"));
const CryptoContractPage = lazy(() => import("./pages/CryptoContract"));
const SettingsPage = lazy(() => import("./pages/Settings"));

function App() {
  // quoteWarning in the global store is the single source of truth for the
  // Xueqiu warning banner. All delivery paths write to it; the JSX below
  // reads from it. This avoids a split between local pendingWarning state
  // and the store copy that caused warnings set by fetchHoldingQuotes /
  // fetchQuotes to never reach the Alert.
  const quoteWarning = useQuoteStore((s) => s.quoteWarning);
  const setQuoteWarning = useQuoteStore((s) => s.setQuoteWarning);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    // Path 1 (fast): the background startup refresh emits `quote-warning`
    // carrying the warning text directly in the payload via peek (so
    // LAST_QUOTE_WARNING is NOT consumed and remains available below).
    listen<string>("quote-warning", (event) => {
      if (event.payload) setQuoteWarning(event.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Path 2 (polling fallback): poll take_quote_warning every 3 s for up to
    // 30 s.  Xueqiu failures now surface after at most one 15-second timeout,
    // so a 30-second budget is more than sufficient.  This catches warnings
    // that arrive before the quote-warning event listener is registered, or
    // when the event is missed due to webview timing.  Stops on first hit.
    let pollCount = 0;
    const MAX_POLLS = 10;          // 10 × 3 s = 30 s total
    const POLL_INTERVAL_MS = 3000; // 3 seconds between checks
    const intervalId = window.setInterval(async () => {
      if (cancelled) {
        window.clearInterval(intervalId);
        return;
      }
      pollCount += 1;
      if (pollCount > MAX_POLLS) {
        window.clearInterval(intervalId);
        return;
      }
      const w = await invoke<string | null>("take_quote_warning").catch(() => null);
      if (w) {
        setQuoteWarning(w);
        window.clearInterval(intervalId);
      }
    }, POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
      unsubs.forEach((fn) => fn());
    };
  }, [setQuoteWarning]);

  return (
    <>
      {/* Xueqiu warning banner — rendered in the React tree (not a portal) so
          it is guaranteed to display in Tauri's webview regardless of startup
          timing. Driven by quoteStore.quoteWarning, the single source of truth
          written by fetchHoldingQuotes, fetchQuotes, and the event/poll paths. */}
      {quoteWarning && (
        <div style={{
          position: "fixed",
          top: 16,
          right: 16,
          zIndex: 9999,
          maxWidth: 400,
          width: "calc(100vw - 32px)",
          boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
          borderRadius: 8,
        }}>
          <Alert
            type="warning"
            message="行情获取提示"
            description={quoteWarning}
            showIcon
            closable
            onClose={() => setQuoteWarning(null)}
          />
        </div>
      )}
      <MainLayout>
        <Suspense fallback={<div style={{ padding: 48, textAlign: "center" }}><Spin /></div>}>
        <Routes>
          <Route path="/" element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<DashboardPage />} />
          <Route path="/statistics" element={<StatisticsPage />} />
          <Route path="/performance" element={<PerformancePage />} />
          <Route path="/accounts" element={<AccountsPage />} />
          <Route path="/holdings" element={<HoldingsPage />} />
          <Route path="/transactions" element={<TransactionsPage />} />
          <Route path="/categories" element={<CategoriesPage />} />
          <Route path="/quarterly" element={<QuarterlyPage />} />
          <Route path="/quarterly/compare" element={<QuarterComparisonPage />} />
          <Route path="/quarterly/trends" element={<TrendsPage />} />
          <Route path="/quarterly/:snapshotId" element={<SnapshotDetail />} />
          <Route path="/import" element={<ImportPage />} />
          <Route path="/alerts" element={<AlertsPage />} />
          <Route path="/review" element={<ReviewPage />} />
          <Route path="/risk" element={<RiskPage />} />
          <Route path="/crypto-spot" element={<CryptoSpotPage />} />
          <Route path="/crypto-contract" element={<CryptoContractPage />} />
          <Route path="/settings" element={<SettingsPage />} />
        </Routes>
        </Suspense>
      </MainLayout>
    </>
  );
}

export default App;
