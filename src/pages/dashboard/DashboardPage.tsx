// Dashboard page: seven stat cards (including enabled/total channels) + 7-day request/Token trends.
// Data is local useState + load() (see Architecture-frontend.md "business data does not go into the store").
// Trend charts use inline SVG polylines, no chart library (ticket 11 needs no polymorphic charts, per the ponytail principle).
import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Activity, ArrowRight, Clock3, Hash, KeyRound, Layers3, Network, Play, Server, Settings2, ShieldCheck, Sigma } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { channelApi, statsApi, invokeErrorMessage } from "@/lib/api";
import type { DailyStat, StatsSnapshot } from "@/types";

const cardCls =
  "rounded-lg border border-border bg-card p-4";
const labelCls = "text-xs font-medium text-muted-foreground";
const valueCls = "mt-1 text-2xl font-semibold tabular-nums";
const subCls = "mt-1 text-xs text-muted-foreground";

/** Thousands-separator formatting (Tokens / request counts). */
function formatCount(n: number): string {
  return n.toLocaleString();
}

/** Availability percentage (0..=1 → 0.0%..100.0%). */
function formatPercent(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

/** Chart Y-axis scale: 0 when all values are 0, otherwise the max (empty data renders a flat line). */
function yMax(points: DailyStat[], valueOf: (d: DailyStat) => number): number {
  const max = Math.max(0, ...points.map(valueOf));
  return max === 0 ? 1 : max;
}

/**
 * Mini trend polyline: width-adaptive, Y axis scaled by the data max, X axis at 7 evenly spaced points.
 * Draws a flat baseline when all data is zero; pure presentational component, no interaction.
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
  const PAD = 8; // padding for the bottom date labels and the top
  const innerW = W - PAD * 2;
  const innerH = H - PAD * 2;
  const max = yMax(points, valueOf);

  // X evenly spaced (half-step at both ends to avoid touching edges), Y aligned to the bottom.
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
        {/* Bottom date labels */}
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
        {/* Data polyline */}
        <polyline
          points={line}
          fill="none"
          stroke={color}
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        {/* Data points */}
        {coords.map((p, i) => (
          <circle key={i} cx={p.x} cy={p.y} r={2.5} fill={color} />
        ))}
      </svg>
    </div>
  );
}

/** Stat card: label + primary value + secondary caption. */
function StatCard({
  label,
  value,
  sub,
  icon: Icon,
  tone,
}: {
  label: string;
  value: string;
  sub: string;
  icon: LucideIcon;
  tone?: "channel" | "quality";
}) {
  return (
    <div className={`${cardCls} flex min-h-36 flex-col ${tone === "channel" ? "border-primary/25 bg-accent/50" : tone === "quality" ? "border-success/25 bg-success/5" : ""}`}>
      <div className="flex items-start justify-between gap-2">
        <p className={labelCls}>{label}</p>
        <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ${tone === "quality" ? "bg-success/10 text-success" : "bg-accent text-primary"}`}>
          <Icon className="h-4 w-4" aria-hidden="true" />
        </span>
      </div>
      <p className={valueCls}>{value}</p>
      <p className={`${subCls} mt-auto pt-2`}>{sub}</p>
    </div>
  );
}

export function DashboardPage() {
  const navigate = useNavigate();
  const [stats, setStats] = useState<StatsSnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [channelCounts, setChannelCounts] = useState<{ enabled: number; total: number } | null>(null);
  const [channelError, setChannelError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    // The two data sources fail independently: a channel query failure must not erase log metrics.
    const [statsResult, channelsResult] = await Promise.allSettled([
      statsApi.get(new Date().getTimezoneOffset()),
      channelApi.list(),
    ]);
    if (statsResult.status === "fulfilled") {
      setStats(statsResult.value);
      setLoadError(null);
    } else {
      setStats(null);
      setLoadError(invokeErrorMessage(statsResult.reason));
    }
    if (channelsResult.status === "fulfilled") {
      setChannelCounts({
        enabled: channelsResult.value.filter((channel) => channel.enabled).length,
        total: channelsResult.value.length,
      });
      setChannelError(null);
    } else {
      setChannelCounts(null);
      setChannelError(invokeErrorMessage(channelsResult.reason));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const setupSteps = [
    { label: "添加第一个渠道", hint: "连接 OpenAI-compatible 或其他上游服务", path: "/channels", icon: Network },
    { label: "创建 API 密钥", hint: "生成下游客户端访问网关的令牌", path: "/api-keys", icon: KeyRound },
    { label: "启动本地服务", hint: "在设置中启动并复制 Base URL", path: "/settings", icon: Play },
  ];

  return (
    <div className="space-y-6">
      <section className="dashboard-intro rounded-2xl border border-border bg-card p-5 shadow-[0_10px_30px_rgba(15,23,42,0.04)] sm:p-7">
        <div className="flex flex-col gap-6 lg:flex-row lg:items-start lg:justify-between">
          <div className="max-w-2xl">
            <div className="mb-3 inline-flex items-center gap-2 rounded-full border border-primary/20 bg-accent px-3 py-1 text-xs font-medium text-accent-foreground">
              <Activity className="h-3.5 w-3.5" /> 本地网关工作台
            </div>
            <h2 className="text-2xl font-semibold tracking-tight sm:text-3xl">让模型调用更稳定、更清晰</h2>
            <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground sm:text-[15px]">
              统一管理上游渠道、访问密钥、请求日志与安全策略。先完成下面的配置，就可以开始接入你的客户端。
            </p>
            <div className="mt-5 flex flex-wrap gap-2">
              <button type="button" onClick={() => navigate("/channels")} className="inline-flex items-center gap-2 rounded-xl bg-primary px-4 py-2.5 text-sm font-semibold text-primary-foreground shadow-sm transition-colors hover:bg-primary/90">
                <Network className="h-4 w-4" /> 配置渠道
              </button>
              <button type="button" onClick={() => navigate("/usage")} className="inline-flex items-center gap-2 rounded-xl border border-border bg-background px-4 py-2.5 text-sm font-medium transition-colors hover:bg-muted">
                查看用量统计 <ArrowRight className="h-4 w-4" />
              </button>
            </div>
          </div>
          <div className="min-w-52 rounded-2xl border border-border bg-background/70 p-4">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">请求成功占比</span>
              <Server className="h-4 w-4 text-success" />
            </div>
            <div className="mt-2 text-3xl font-semibold tabular-nums">{loading || !stats ? "—" : stats.totalRequests === 0 ? "暂无请求" : formatPercent(stats.channelAvailability)}</div>
            <div className="mt-1 text-xs text-muted-foreground">全部日志中 status &lt; 400 的请求占比</div>
          </div>
        </div>
      </section>

      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-base font-semibold">运行概览</h3>
          <p className="mt-1 text-sm text-muted-foreground">快速了解网关最近的工作状态</p>
        </div>
        {!loading && !loadError && stats && <span className="text-xs text-muted-foreground">统计与请求日志一致</span>}
      </div>

      {loadError && (
        <p className="text-sm text-danger" role="alert">统计加载失败：{loadError}</p>
      )}
      {channelError && (
        <p className="text-sm text-danger" role="alert">渠道数量加载失败：{channelError}</p>
      )}

      {/* Card grid */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-4">
        <StatCard
          label="今日请求数"
          icon={Activity}
          value={loading || !stats ? "—" : formatCount(stats.todayRequests)}
          sub="本地时区今日"
        />
        <StatCard
          label="今日 Token"
          icon={Hash}
          value={loading || !stats ? "—" : formatCount(stats.todayTokens)}
          sub="本地时区今日"
        />
        <StatCard
          label="累计请求数"
          icon={Layers3}
          value={loading || !stats ? "—" : formatCount(stats.totalRequests)}
          sub="全部日志"
        />
        <StatCard
          label="累计 Token"
          icon={Sigma}
          value={loading || !stats ? "—" : formatCount(stats.totalTokens)}
          sub="全部日志"
        />
        <StatCard
          label="已启用渠道"
          icon={Network}
          value={loading || !channelCounts ? "—" : `${channelCounts.enabled} / ${channelCounts.total}`}
          sub="按渠道配置统计，非探活结果"
          tone="channel"
        />
        <StatCard
          label="平均延迟"
          icon={Clock3}
          value={
            loading || !stats
              ? "—"
              : stats.totalRequests === 0 ? "暂无请求" : `${Math.round(stats.avgLatencyMs * 10) / 10} ms`
          }
          sub="全部请求均值"
        />
        <StatCard
          label="渠道可用率"
          icon={ShieldCheck}
          value={loading || !stats ? "—" : stats.totalRequests === 0 ? "暂无请求" : formatPercent(stats.channelAvailability)}
          sub="status < 400 的请求占比"
          tone="quality"
        />
      </div>

      <section className="rounded-2xl border border-border bg-card p-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="text-base font-semibold">快速完成配置</h3>
            <p className="mt-1 text-sm text-muted-foreground">按顺序完成三个步骤，开始调用本地网关</p>
          </div>
          <Settings2 className="h-5 w-5 text-muted-foreground" />
        </div>
        <div className="mt-4 grid gap-2 md:grid-cols-3">
          {setupSteps.map(({ label, hint, path, icon: Icon }, index) => (
            <button key={path} type="button" onClick={() => navigate(path)} className="group flex items-start gap-3 rounded-xl border border-border bg-background/60 p-3 text-left transition-colors hover:border-primary/30 hover:bg-accent">
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent text-xs font-semibold text-accent-foreground">{index + 1}</span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm font-medium"><Icon className="h-3.5 w-3.5 text-primary" />{label}</span>
                <span className="mt-1 block text-xs leading-5 text-muted-foreground">{hint}</span>
              </span>
              <ArrowRight className="mt-1 h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
            </button>
          ))}
        </div>
      </section>

      {/* 7-day trend */}
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
