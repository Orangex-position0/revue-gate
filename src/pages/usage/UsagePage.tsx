// Usage stats page: date-range trend plus local-sort channel/model leaderboards.
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { invokeErrorMessage, statsApi } from "@/lib/api";
import type { DailyStat, RankRow, UsageStats } from "@/types";

type SortKey = "tokens" | "requests" | "avgLatencyMs" | "availability";
type RankTab = "channel" | "model";

const labelCls = "text-xs font-medium text-muted-foreground";
const tableHeadCls =
  "px-3 py-2 text-left text-xs font-medium text-muted-foreground";
const tableCellCls = "px-3 py-2 text-sm tabular-nums";

function formatDateInput(date: Date): string {
  const y = date.getFullYear();
  const m = `${date.getMonth() + 1}`.padStart(2, "0");
  const d = `${date.getDate()}`.padStart(2, "0");
  return `${y}-${m}-${d}`;
}

function localDayIso(date: string): string {
  return new Date(`${date}T00:00:00`).toISOString();
}

function localDayAfterIso(date: string): string {
  const d = new Date(`${date}T00:00:00`);
  d.setDate(d.getDate() + 1);
  return d.toISOString();
}

function formatCount(n: number): string {
  return n.toLocaleString();
}

function formatMs(n: number): string {
  return `${(Math.round(n * 10) / 10).toLocaleString()} ms`;
}

function formatPercent(n: number): string {
  return `${(n * 100).toFixed(1)}%`;
}

function maxOf(points: DailyStat[], valueOf: (d: DailyStat) => number): number {
  return Math.max(1, ...points.map(valueOf));
}

function BarChart({
  points,
  valueOf,
  color,
  label,
}: {
  points: DailyStat[];
  valueOf: (d: DailyStat) => number;
  color: string;
  label: string;
}) {
  const W = 720;
  const H = 220;
  const PAD_X = 22;
  const PAD_TOP = 12;
  const PAD_BOTTOM = 24;
  const innerW = W - PAD_X * 2;
  const innerH = H - PAD_TOP - PAD_BOTTOM;
  const max = maxOf(points, valueOf);
  const slot = points.length > 0 ? innerW / points.length : innerW;
  const barW = Math.max(4, Math.min(28, slot * 0.58));

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="h-56 w-full" role="img" aria-label={label}>
      {points.map((p, i) => {
        const value = valueOf(p);
        const h = (value / max) * innerH;
        const x = PAD_X + i * slot + (slot - barW) / 2;
        const y = PAD_TOP + innerH - h;
        return (
          <g key={p.date}>
            <rect x={x} y={y} width={barW} height={Math.max(1, h)} rx={3} fill={color} />
            <text
              x={PAD_X + i * slot + slot / 2}
              y={H - 5}
              textAnchor="middle"
              className="fill-muted-foreground"
              style={{ fontSize: 10 }}
            >
              {p.date.slice(5)}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function sortedRows(rows: RankRow[], sortKey: SortKey): RankRow[] {
  return [...rows].sort((a, b) => {
    const primary = b[sortKey] - a[sortKey];
    return primary !== 0 ? primary : a.name.localeCompare(b.name);
  });
}

function RankTable({
  rows,
  sortKey,
  onSort,
}: {
  rows: RankRow[];
  sortKey: SortKey;
  onSort: (key: SortKey) => void;
}) {
  const sorted = useMemo(() => sortedRows(rows, sortKey), [rows, sortKey]);

  return (
    <div className="overflow-hidden rounded-lg border border-border bg-card">
      <table className="w-full border-collapse">
        <thead className="bg-muted/60">
          <tr>
            <th className={tableHeadCls}>名称</th>
            <th className={tableHeadCls}>
              <button type="button" onClick={() => onSort("requests")}>请求数</button>
            </th>
            <th className={tableHeadCls}>
              <button type="button" onClick={() => onSort("tokens")}>Token</button>
            </th>
            <th className={tableHeadCls}>
              <button type="button" onClick={() => onSort("avgLatencyMs")}>平均延迟</button>
            </th>
            <th className={tableHeadCls}>
              <button type="button" onClick={() => onSort("availability")}>可用率</button>
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {sorted.map((row) => (
            <tr key={row.key}>
              <td className="px-3 py-2 text-sm">
                <div className="max-w-[22rem] truncate font-medium">{row.name}</div>
                <div className="max-w-[22rem] truncate text-xs text-muted-foreground">{row.key}</div>
              </td>
              <td className={tableCellCls}>{formatCount(row.requests)}</td>
              <td className={tableCellCls}>{formatCount(row.tokens)}</td>
              <td className={tableCellCls}>{formatMs(row.avgLatencyMs)}</td>
              <td className={tableCellCls}>{formatPercent(row.availability)}</td>
            </tr>
          ))}
          {sorted.length === 0 && (
            <tr>
              <td colSpan={5} className="px-3 py-8 text-center text-sm text-muted-foreground">
                暂无数据
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

export function UsagePage() {
  const today = useMemo(() => new Date(), []);
  const defaultStart = useMemo(() => {
    const d = new Date(today);
    d.setDate(d.getDate() - 13);
    return formatDateInput(d);
  }, [today]);
  const defaultEnd = useMemo(() => formatDateInput(today), [today]);

  const [startDate, setStartDate] = useState(defaultStart);
  const [endDate, setEndDate] = useState(defaultEnd);
  const [stats, setStats] = useState<UsageStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [rankTab, setRankTab] = useState<RankTab>("channel");
  const [sortKey, setSortKey] = useState<SortKey>("tokens");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStats(
        await statsApi.usage(
          localDayIso(startDate),
          localDayAfterIso(endDate),
          new Date().getTimezoneOffset(),
        ),
      );
      setLoadError(null);
    } catch (error) {
      setLoadError(invokeErrorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [startDate, endDate]);

  useEffect(() => {
    void load();
  }, [load]);

  const totalRequests = stats?.daily.reduce((sum, d) => sum + d.requests, 0) ?? 0;
  const totalTokens = stats?.daily.reduce((sum, d) => sum + d.tokens, 0) ?? 0;
  const tableRows = rankTab === "channel" ? stats?.byChannel ?? [] : stats?.byModel ?? [];

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-xl font-semibold">用量统计</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {loading || !stats
              ? "正在读取请求日志"
              : `${formatCount(totalRequests)} 次请求 · ${formatCount(totalTokens)} Token`}
          </p>
        </div>
        <div className="flex flex-wrap items-end gap-2">
          <label className="grid gap-1">
            <span className={labelCls}>开始</span>
            <input
              type="date"
              value={startDate}
              onChange={(e) => setStartDate(e.target.value)}
              className="h-9 rounded-md border border-border bg-card px-3 text-sm"
            />
          </label>
          <label className="grid gap-1">
            <span className={labelCls}>结束</span>
            <input
              type="date"
              value={endDate}
              onChange={(e) => setEndDate(e.target.value)}
              className="h-9 rounded-md border border-border bg-card px-3 text-sm"
            />
          </label>
          <button
            type="button"
            onClick={() => void load()}
            className="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground"
            disabled={loading}
            title="刷新"
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
      </div>

      {loadError && (
        <p className="text-sm text-danger" role="alert">
          加载失败：{loadError}
        </p>
      )}

      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className={labelCls}>每日请求数</span>
            <span className="text-xs text-muted-foreground">按本地日期</span>
          </div>
          <BarChart
            points={stats?.daily ?? []}
            valueOf={(d) => d.requests}
            color="var(--color-primary)"
            label="每日请求数"
          />
        </div>
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className={labelCls}>每日 Token</span>
            <span className="text-xs text-muted-foreground">按本地日期</span>
          </div>
          <BarChart
            points={stats?.daily ?? []}
            valueOf={(d) => d.tokens}
            color="var(--color-success)"
            label="每日 Token"
          />
        </div>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => setRankTab("channel")}
          className={`rounded-md px-3 py-1.5 text-sm ${
            rankTab === "channel" ? "bg-accent font-medium" : "text-muted-foreground hover:bg-muted"
          }`}
        >
          渠道排行
        </button>
        <button
          type="button"
          onClick={() => setRankTab("model")}
          className={`rounded-md px-3 py-1.5 text-sm ${
            rankTab === "model" ? "bg-accent font-medium" : "text-muted-foreground hover:bg-muted"
          }`}
        >
          模型排行
        </button>
      </div>

      <RankTable rows={tableRows} sortKey={sortKey} onSort={setSortKey} />
    </div>
  );
}
