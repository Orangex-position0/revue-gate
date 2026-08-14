// 请求日志页：分页 + 多条件筛选 / 详情查看 / 按日期前删除 / 清空全部（ticket 10）。
// 数据本地 useState + load()，操作后就地刷新（见 Architecture-frontend.md「业务数据不进 store」）。
import { useCallback, useEffect, useState, type FormEvent } from "react";
import {
  CalendarX,
  ChevronLeft,
  ChevronRight,
  Eye,
  RotateCcw,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { apiKeyApi, channelApi, logApi, invokeErrorMessage } from "@/lib/api";
import type { ApiKey, Channel, LogDetail, LogQuery, RequestLog } from "@/types";

const inputCls =
  "w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-primary";
const labelCls = "block text-xs font-medium text-muted-foreground";
const cellCls = "px-3 py-2 align-middle text-sm";

/** 本地时区某日零点 → ISO-8601 UTC：日期筛选统一按「本地日界」半开区间（见 date-handling 规范）。 */
function localDayStartIso(dateStr: string): string {
  return new Date(`${dateStr}T00:00:00`).toISOString();
}

/** 本地时区某日次日的零点 → ISO-8601 UTC：结束日期筛选的右开上界（含所选整天，`[startDay, endDay+1day)`）。 */
function localDayAfterIso(dateStr: string): string {
  const d = new Date(`${dateStr}T00:00:00`);
  d.setDate(d.getDate() + 1);
  return d.toISOString();
}

/** 时间显示为本地时区短格式（后端存 UTC ISO-8601）。 */
function formatTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

/** 状态码着色：2xx 成功 / 4xx-5xx 错误 / 其余中性。 */
function statusCls(code: number): string {
  if (code >= 200 && code < 300) return "bg-success/10 text-success";
  if (code >= 400) return "bg-danger/10 text-danger";
  return "bg-accent text-accent-foreground";
}

/** 请求体 JSON 美化：可解析则格式化，否则原样展示。 */
function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(
      typeof value === "string" ? JSON.parse(value) : value,
      null,
      2,
    );
  } catch {
    return String(value);
  }
}

/** 小徽标：流式 / 重试等布尔状态。 */
function TinyBadge({ label }: { label: string }) {
  return (
    <span className="rounded bg-accent px-1.5 py-0.5 text-xs text-accent-foreground">
      {label}
    </span>
  );
}

/** 日志详情弹窗：路由 / usage / 对话构成 / 请求参数 / 工具标签 / 原始 JSON。 */
interface LogDetailModalProps {
  detail: LogDetail;
  onClose: () => void;
  channelNameOf: (id: string | null) => string;
  keyNameOf: (id: string | null) => string;
}

function LogDetailModal({
  detail,
  onClose,
  channelNameOf,
  keyNameOf,
}: LogDetailModalProps) {
  const body = detail.requestBody;
  const paramsKeys = Object.keys(
    detail.requestParams && typeof detail.requestParams === "object"
      ? (detail.requestParams as Record<string, unknown>)
      : {},
  );
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="日志详情"
    >
      <div className="max-h-[90vh] w-full max-w-2xl overflow-y-auto rounded-lg border border-border bg-card p-5 shadow-lg">
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-base font-semibold">日志详情</h3>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 路由 / 元信息 */}
        <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          <Field k="创建时间" v={formatTime(detail.createdAt)} />
          <Field k="Trace ID" v={detail.traceId} mono />
          <Field k="渠道" v={channelNameOf(detail.channelId)} />
          <Field k="密钥" v={keyNameOf(detail.apiKeyId)} />
          <Field
            k="模型"
            v={
              detail.upstreamModel && detail.upstreamModel !== detail.model
                ? `${detail.model} → ${detail.upstreamModel}`
                : detail.model
            }
          />
          <Field k="状态码" v={String(detail.statusCode)} />
          <Field k="耗时" v={`${detail.durationMs} ms`} />
          <Field
            k="Tokens"
            v={
              detail.totalTokens != null
                ? `prompt ${detail.promptTokens ?? 0} / completion ${
                    detail.completionTokens ?? 0
                  } / total ${detail.totalTokens}`
                : "—"
            }
          />
          <div className="flex items-center gap-2">
            <dt className="text-muted-foreground">标记</dt>
            <dd className="flex gap-1.5">
              {detail.isStream && <TinyBadge label="流式" />}
              {detail.isRetry && <TinyBadge label="重试" />}
              {!detail.isStream && !detail.isRetry && <span>—</span>}
            </dd>
          </div>
          {detail.errorMessage && (
            <div className="col-span-2">
              <dt className="text-muted-foreground">错误</dt>
              <dd className="text-danger">{detail.errorMessage}</dd>
            </div>
          )}
        </dl>

        {/* 对话构成 */}
        <section className="mt-5">
          <h4 className="mb-2 text-sm font-semibold">对话构成</h4>
          {detail.conversation.length === 0 ? (
            <p className="text-sm text-muted-foreground">无对话消息。</p>
          ) : (
            <ul className="space-y-1.5">
              {detail.conversation.map((msg, i) => (
                <li
                  key={i}
                  className="rounded-md border border-border px-3 py-1.5 text-sm"
                >
                  <span className="font-medium">{msg.role}</span>
                  {msg.content && (
                    <span className="ml-2 whitespace-pre-wrap break-all text-muted-foreground">
                      {msg.content}
                    </span>
                  )}
                  {msg.toolNames.length > 0 && (
                    <span className="ml-2 text-xs text-muted-foreground">
                      工具调用：{msg.toolNames.join(", ")}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </section>

        {/* 工具标签 + 请求参数 */}
        <section className="mt-5 space-y-4">
          <div>
            <h4 className="mb-2 text-sm font-semibold">工具</h4>
            {detail.toolNames.length === 0 ? (
              <p className="text-sm text-muted-foreground">无工具调用。</p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {detail.toolNames.map((t) => (
                  <TinyBadge key={t} label={t} />
                ))}
              </div>
            )}
          </div>
          <div>
            <h4 className="mb-2 text-sm font-semibold">请求参数</h4>
            {paramsKeys.length === 0 ? (
              <p className="text-sm text-muted-foreground">无附加参数。</p>
            ) : (
              <pre className="max-h-48 overflow-auto rounded-md border border-border bg-muted p-3 text-xs">
                {prettyJson(detail.requestParams)}
              </pre>
            )}
          </div>
          <div>
            <h4 className="mb-2 text-sm font-semibold">原始请求体</h4>
            {body ? (
              <pre className="max-h-48 overflow-auto rounded-md border border-border bg-muted p-3 text-xs">
                {prettyJson(body)}
              </pre>
            ) : (
              <p className="text-sm text-muted-foreground">无请求体记录。</p>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}

/** 详情弹窗的标签/值行。 */
function Field({ k, v, mono = false }: { k: string; v: string; mono?: boolean }) {
  return (
    <div className={mono ? "truncate font-mono text-xs leading-5" : ""}>
      <dt className="inline text-muted-foreground">{k}：</dt>
      <dd className="inline break-all">{v}</dd>
    </div>
  );
}

export function LogsPage() {
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // 筛选表单（输入态）与已提交筛选（驱动查询）。
  const [keyword, setKeyword] = useState("");
  const [model, setModel] = useState("");
  const [apiKeyId, setApiKeyId] = useState("");
  const [channelId, setChannelId] = useState("");
  const [startAt, setStartAt] = useState("");
  const [endAt, setEndAt] = useState("");
  const [filters, setFilters] = useState<LogQuery>({});
  // 删除/清空后强制重新加载（setPage 可能为 no-op，需独立触发一次刷新）。
  const [refresh, setRefresh] = useState(0);

  // 下拉选项：密钥 / 渠道（同时用于详情反查名称；实体可能被删，缺失时回退 id 前缀）。
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);

  // 删除操作：按日期前删除 + 清空全部。
  const [deleteBeforeDate, setDeleteBeforeDate] = useState("");
  // 详情弹窗。
  const [detail, setDetail] = useState<LogDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const result = await logApi.list(filters, page, pageSize);
      // 删除后当前页可能越界：非首页且该页为空但有总数 → 跳到最后一页。
      if (result.items.length === 0 && page > 1 && result.total > 0) {
        setPage(Math.min(page, Math.ceil(result.total / pageSize)));
        return;
      }
      setLogs(result.items);
      setTotal(result.total);
      setLoadError(null);
    } catch (error) {
      setLoadError(invokeErrorMessage(error));
    } finally {
      setLoading(false);
    }
  }, [filters, page, pageSize, refresh]);

  useEffect(() => {
    void load();
  }, [load]);

  // 下拉选项数据只拉一次（实体列表低频变化，重拉零成本但无需每页刷新）。
  useEffect(() => {
    void apiKeyApi.list().then(setKeys).catch(() => {});
    void channelApi.list().then(setChannels).catch(() => {});
  }, []);

  /** 提交筛选：跳回第一页并提交新的 LogQuery。 */
  function handleSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPage(1);
    setFilters({
      keyword: keyword.trim() || null,
      apiKeyId: apiKeyId || null,
      channelId: channelId || null,
      model: model.trim() || null,
      startAt: startAt ? localDayStartIso(startAt) : null,
      // 结束日期取「次日零点」作右开上界，保证所选整天都在区间内。
      endAt: endAt ? localDayAfterIso(endAt) : null,
    });
  }

  /** 重置筛选表单并回到无过滤第一页。 */
  function handleReset() {
    setKeyword("");
    setModel("");
    setApiKeyId("");
    setChannelId("");
    setStartAt("");
    setEndAt("");
    setPage(1);
    setFilters({});
  }

  /** 删除早于所选日期的日志（不含当天，半开 [.., dayStart)）。 */
  async function handleDeleteBefore() {
    if (!deleteBeforeDate) return;
    const before = localDayStartIso(deleteBeforeDate);
    if (
      !window.confirm(
        `删除 ${formatTime(before)} 之前的全部请求日志？此操作不可撤销。`,
      )
    ) {
      return;
    }
    try {
      const n = await logApi.deleteBefore(before);
      window.alert(`已删除 ${n} 条日志`);
      setDeleteBeforeDate("");
      setPage(1);
      setRefresh((r) => r + 1);
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  /** 清空全部日志。 */
  async function handleClear() {
    if (
      !window.confirm("确定清空全部请求日志？此操作不可撤销。")
    ) {
      return;
    }
    try {
      const n = await logApi.clear();
      window.alert(`已删除 ${n} 条日志`);
      setPage(1);
      setRefresh((r) => r + 1);
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  /** 打开详情弹窗。 */
  async function handleDetail(id: string) {
    setDetailLoading(true);
    try {
      setDetail(await logApi.detail(id));
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    } finally {
      setDetailLoading(false);
    }
  }

  function channelNameOf(id: string | null): string {
    if (!id) return "—";
    return channels.find((c) => c.id === id)?.name ?? `渠道 ${id.slice(0, 8)}…`;
  }

  function keyNameOf(id: string | null): string {
    if (!id) return "—";
    return keys.find((k) => k.id === id)?.name ?? `密钥 ${id.slice(0, 8)}…`;
  }

  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">日志</h2>
        <span className="text-sm text-muted-foreground">共 {total} 条记录</span>
      </div>

      {/* 筛选 + 删除工具栏 */}
      <form
        onSubmit={handleSearch}
        className="space-y-3 rounded-lg border border-border bg-card p-3"
      >
        <div className="grid grid-cols-2 gap-3 md:grid-cols-3 lg:grid-cols-6">
          <div>
            <label className={labelCls} htmlFor="f-keyword">
              关键词
            </label>
            <input
              id="f-keyword"
              className={inputCls}
              value={keyword}
              onChange={(e) => setKeyword(e.target.value)}
              placeholder="模型 / trace / 错误"
            />
          </div>
          <div>
            <label className={labelCls} htmlFor="f-model">
              模型
            </label>
            <input
              id="f-model"
              className={inputCls}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="gpt-4o"
            />
          </div>
          <div>
            <label className={labelCls} htmlFor="f-key">
              密钥
            </label>
            <select
              id="f-key"
              className={inputCls}
              value={apiKeyId}
              onChange={(e) => setApiKeyId(e.target.value)}
            >
              <option value="">全部</option>
              {keys.map((k) => (
                <option key={k.id} value={k.id}>
                  {k.name}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className={labelCls} htmlFor="f-channel">
              渠道
            </label>
            <select
              id="f-channel"
              className={inputCls}
              value={channelId}
              onChange={(e) => setChannelId(e.target.value)}
            >
              <option value="">全部</option>
              {channels.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className={labelCls} htmlFor="f-start">
              起始日期
            </label>
            <input
              id="f-start"
              className={inputCls}
              type="date"
              value={startAt}
              max={endAt || undefined}
              onChange={(e) => setStartAt(e.target.value)}
            />
          </div>
          <div>
            <label className={labelCls} htmlFor="f-end">
              结束日期
            </label>
            <input
              id="f-end"
              className={inputCls}
              type="date"
              value={endAt}
              min={startAt || undefined}
              onChange={(e) => setEndAt(e.target.value)}
            />
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button
            type="submit"
            className="flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
          >
            <Search className="h-4 w-4" />
            查询
          </button>
          <button
            type="button"
            onClick={handleReset}
            className="flex items-center gap-1 rounded-md border border-border px-3 py-1.5 text-sm hover:bg-muted"
          >
            <RotateCcw className="h-4 w-4" />
            重置
          </button>
          <div className="ml-auto flex flex-wrap items-center gap-2">
            <input
              type="date"
              value={deleteBeforeDate}
              onChange={(e) => setDeleteBeforeDate(e.target.value)}
              className={`${inputCls} w-40`}
              aria-label="删除日期"
            />
            <button
              type="button"
              onClick={() => void handleDeleteBefore()}
              disabled={!deleteBeforeDate}
              className="flex items-center gap-1 rounded-md border border-danger/30 px-3 py-1.5 text-sm text-danger hover:bg-danger/10 disabled:opacity-50"
            >
              <CalendarX className="h-4 w-4" />
              删除早于
            </button>
            <button
              type="button"
              onClick={() => void handleClear()}
              className="flex items-center gap-1 rounded-md bg-danger px-3 py-1.5 text-sm text-white hover:opacity-90"
            >
              <Trash2 className="h-4 w-4" />
              清空全部
            </button>
          </div>
        </div>
      </form>

      {loadError && (
        <p className="text-sm text-danger" role="alert">
          加载失败：{loadError}
        </p>
      )}

      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <table className="w-full">
          <thead className="bg-muted text-left text-xs uppercase text-muted-foreground">
            <tr>
              <th className="px-3 py-2">时间</th>
              <th className="px-3 py-2">模型</th>
              <th className="px-3 py-2">状态</th>
              <th className="px-3 py-2">Tokens</th>
              <th className="px-3 py-2">耗时</th>
              <th className="px-3 py-2">标记</th>
              <th className="px-3 py-2">渠道</th>
              <th className="px-3 py-2">密钥</th>
              <th className="px-3 py-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {loading ? (
              <tr>
                <td className={cellCls} colSpan={9}>
                  加载中…
                </td>
              </tr>
            ) : logs.length === 0 ? (
              <tr>
                <td className={cellCls} colSpan={9}>
                  暂无日志{total === 0 ? "，发起请求后自动记录" : "（调整筛选条件）"}。
                </td>
              </tr>
            ) : (
              logs.map((log) => (
                <tr key={log.id} className="hover:bg-muted/50">
                  <td className={`${cellCls} whitespace-nowrap`}>
                    <div>{formatTime(log.createdAt)}</div>
                    <div className="max-w-36 truncate font-mono text-xs text-muted-foreground">
                      {log.traceId}
                    </div>
                  </td>
                  <td className={cellCls}>
                    {log.upstreamModel && log.upstreamModel !== log.model ? (
                      <div title={`${log.model} → ${log.upstreamModel}`}>
                        {log.model} → {log.upstreamModel}
                      </div>
                    ) : (
                      log.model
                    )}
                  </td>
                  <td className={cellCls}>
                    <span
                      className={`inline-block rounded px-1.5 py-0.5 text-xs font-medium ${statusCls(log.statusCode)}`}
                    >
                      {log.statusCode}
                    </span>
                  </td>
                  <td className={cellCls}>
                    {log.totalTokens != null ? (
                      <span title={`prompt ${log.promptTokens ?? 0} / completion ${log.completionTokens ?? 0}`}>
                        {log.totalTokens}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </td>
                  <td className={`${cellCls} text-muted-foreground`}>
                    {log.durationMs} ms
                  </td>
                  <td className={cellCls}>
                    <div className="flex gap-1.5">
                      {log.isStream && <TinyBadge label="流式" />}
                      {log.isRetry && <TinyBadge label="重试" />}
                      {log.errorMessage && (
                        <span
                          className="max-w-24 truncate rounded bg-danger/10 px-1.5 py-0.5 text-xs text-danger"
                          title={log.errorMessage}
                        >
                          失败
                        </span>
                      )}
                    </div>
                  </td>
                  <td className={`${cellCls} text-muted-foreground`}>
                    {channelNameOf(log.channelId)}
                  </td>
                  <td className={`${cellCls} text-muted-foreground`}>
                    {keyNameOf(log.apiKeyId)}
                  </td>
                  <td className={`${cellCls} text-right`}>
                    <button
                      type="button"
                      onClick={() => void handleDetail(log.id)}
                      disabled={detailLoading}
                      aria-label={`查看日志 ${log.traceId}`}
                      className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"
                    >
                      <Eye className="h-4 w-4" />
                    </button>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* 分页 */}
      <div className="flex items-center justify-between text-sm">
        <div className="flex items-center gap-2">
          <span className="text-muted-foreground">每页</span>
          <select
            value={pageSize}
            onChange={(e) => {
              setPageSize(Number(e.target.value));
              setPage(1);
            }}
            className="rounded-md border border-border bg-background px-2 py-1 focus:outline-none focus:ring-1 focus:ring-primary"
          >
            {[10, 20, 50, 100].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setPage((p) => Math.max(1, p - 1))}
            disabled={page <= 1}
            aria-label="上一页"
            className="rounded-md border border-border p-1.5 hover:bg-muted disabled:opacity-50"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <span className="text-muted-foreground">
            第 {page} / {totalPages} 页
          </span>
          <button
            type="button"
            onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
            disabled={page >= totalPages}
            aria-label="下一页"
            className="rounded-md border border-border p-1.5 hover:bg-muted disabled:opacity-50"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
      </div>

      {detail && (
        <LogDetailModal
          detail={detail}
          onClose={() => setDetail(null)}
          channelNameOf={channelNameOf}
          keyNameOf={keyNameOf}
        />
      )}
    </div>
  );
}
