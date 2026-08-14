// 仪表盘页：6 张统计卡片（今日/累计请求数与 Token、平均延迟、渠道可用率）+ 7 天请求数/Token 趋势折线。
// 数据本地 useState + load()（见 Architecture-frontend.md「业务数据不进 store」）。
// 趋势图用内联 SVG 折线，不引入图表库（ticket 11 无多态图要求，见 ponytail 原则）。
import { useCallback, useEffect, useState } from "react";
import { statsApi, invokeErrorMessage } from "@/lib/api";
import type { DailyStat, StatsSnapshot } from "@/types";

const cardCls =
  "rounded-lg border border-border bg-card p-4";
const labelCls = "text-xs font-medium text-muted-foreground";
const valueCls = "mt-1 text-2xl font-semibold tabular-nums";
const subCls = "mt-1 text-xs text-muted-foreground";

/** 千分位格式化（Token / 请求数）。 */
function formatCount(n: number): string {
  return n.toLocaleString();
}

/** 可用率百分比（0..=1 → 0.0%..100.0%）。 */
function formatPercent(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

/** 图表 Y 轴刻度：为 0 时显示 0，否则显示最大值（空数据为平直线）。 */
function yMax(points: DailyStat[], valueOf: (d: DailyStat) => number): number {
  const max = Math.max(0, ...points.map(valueOf));
  return max === 0 ? 1 : max;
}

/**
 * 迷你趋势折线：宽度自适应，Y 轴按数据最大值缩放，X 轴等距 7 点。
 * 数据全零时画底部平直线；纯展示组件，无交互。
 */
function TrendChart({
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
  const W = 600;
  const H = 140;
  const PAD = 8; // 底部日期标签与顶部留白
  const innerW = W - PAD * 2;
  const innerH = H - PAD * 2;
  const max = yMax(points, valueOf);

  // X 等距（首尾留半格避免贴边），Y 底部对齐。
  const step = points.length > 1 ? innerW / (points.length - 1) : innerW;
  const coords = points.map((d, i) => ({
    x: PAD + i * step,
    y: PAD + innerH * (1 - valueOf(d) / max),
  }));
  const line = coords.map((p) => `${p.x},${p.y}`).join(" ");

  return (
    <div>
      <div className="mb-1 flex items-center justify-between">
        <span className={labelCls}>{label}</span>
        <span className="flex items-center gap-1 text-xs text-muted-foreground">
          <span className="inline-block h-1.5 w-3 rounded-full" style={{ background: color }} />
          近 7 天
        </span>
      </div>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full"
        role="img"
        aria-label={`${label}近 7 天趋势`}
      >
        {/* 底部日期标签 */}
        {points.map((d, i) => (
          <text
            key={d.date}
            x={coords[i]?.x ?? 0}
            y={H - 1}
            textAnchor="middle"
            className="fill-muted-foreground"
            style={{ fontSize: 10 }}
          >
            {d.date.slice(5)}
          </text>
        ))}
        {/* 数据折线 */}
        <polyline
          points={line}
          fill="none"
          stroke={color}
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        {/* 数据点 */}
        {coords.map((p, i) => (
          <circle key={i} cx={p.x} cy={p.y} r={2.5} fill={color} />
        ))}
      </svg>
    </div>
  );
}

/** 统计卡片：标签 + 主值 + 副说明。 */
function StatCard({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub: string;
}) {
  return (
    <div className={cardCls}>
      <p className={labelCls}>{label}</p>
      <p className={valueCls}>{value}</p>
      <p className={subCls}>{sub}</p>
    </div>
  );
}

export function DashboardPage() {
  const [stats, setStats] = useState<StatsSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // 后端按前端本地时区日界聚合；getTimezoneOffset() 返回分钟西偏（与后端约定一致）。
      setStats(await statsApi.get(new Date().getTimezoneOffset()));
      setLoadError(null);
    } catch (error) {
      setLoadError(invokeErrorMessage(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">仪表盘</h2>
        {!loading && !loadError && stats && (
          <span className="text-sm text-muted-foreground">
            统计与请求日志一致
          </span>
        )}
      </div>

      {loadError && (
        <p className="text-sm text-danger" role="alert">
          加载失败：{loadError}
        </p>
      )}

      {/* 卡片区 */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <StatCard
          label="今日请求数"
          value={loading || !stats ? "—" : formatCount(stats.todayRequests)}
          sub="本地时区今日"
        />
        <StatCard
          label="今日 Token"
          value={loading || !stats ? "—" : formatCount(stats.todayTokens)}
          sub="本地时区今日"
        />
        <StatCard
          label="累计请求数"
          value={loading || !stats ? "—" : formatCount(stats.totalRequests)}
          sub="全部日志"
        />
        <StatCard
          label="累计 Token"
          value={loading || !stats ? "—" : formatCount(stats.totalTokens)}
          sub="全部日志"
        />
        <StatCard
          label="平均延迟"
          value={
            loading || !stats
              ? "—"
              : `${Math.round(stats.avgLatencyMs * 10) / 10} ms`
          }
          sub="全部请求均值"
        />
        <StatCard
          label="渠道可用率"
          value={loading || !stats ? "—" : formatPercent(stats.channelAvailability)}
          sub="status < 400 占比"
        />
      </div>

      {/* 7 天趋势 */}
      {loading ? (
        <p className="text-sm text-muted-foreground">加载中…</p>
      ) : !stats ? null : (
        <div className="grid gap-3 lg:grid-cols-2">
          <div className="rounded-lg border border-border bg-card p-4">
            <TrendChart
              points={stats.trend}
              valueOf={(d) => d.requests}
              color="var(--color-primary)"
              label="请求数"
            />
          </div>
          <div className="rounded-lg border border-border bg-card p-4">
            <TrendChart
              points={stats.trend}
              valueOf={(d) => d.tokens}
              color="var(--color-primary)"
              label="Token"
            />
          </div>
        </div>
      )}
    </div>
  );
}
