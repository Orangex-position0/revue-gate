// Channel management page: channel list + CRUD / enable-disable / connectivity test (ticket 04 / 06).
// Data is local useState + load(), refreshed in place after CRUD (see Architecture-frontend.md "business data does not go into the store").
import { useCallback, useEffect, useState } from "react";
import { Activity, Loader2, Pencil, Plus, Trash2, X } from "lucide-react";
import { ChannelForm } from "./ChannelForm";
import { CHANNEL_TYPE_LABELS } from "@/lib/constants";
import { channelApi, invokeErrorMessage } from "@/lib/api";
import type { Channel } from "@/types";

/** Form state: null = closed; { channel: null } = create; { channel } = edit. */
type FormState = { channel: Channel | null } | null;

const cellCls = "px-3 py-2 align-middle text-sm";

/** Test time shown in a short local-timezone format (the backend stores UTC ISO-8601). */
function formatTestTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

export function ChannelsPage() {
  const [channels, setChannels] = useState<Channel[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(null);
  const [deleteTarget, setDeleteTarget] = useState<Channel | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  /** id of the channel being tested; non-null disables that row's test button. */
  const [testingId, setTestingId] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setChannels(await channelApi.list());
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

  /** Save callback: close the form and re-fetch the list — the backend sorts by "priority asc → name asc",
   *  so re-fetching keeps the UI order consistent with the database (both create and edit can change the order). */
  function handleSaved() {
    setForm(null);
    void load();
  }

  /** Enable/disable: call set_channel_enabled and replace in place with the returned state, keeping it consistent with the database. */
  async function handleToggle(channel: Channel) {
    try {
      const updated = await channelApi.setEnabled(channel.id, !channel.enabled);
      setChannels((prev) =>
        prev.map((c) => (c.id === updated.id ? updated : c)),
      );
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  /** Connectivity test: calls test_channel to echo latency / error, then re-fetches the list to refresh lastTest*. */
  async function handleTest(channel: Channel) {
    setTestingId(channel.id);
    try {
      const result = await channelApi.test(channel.id);
      const message = result.ok
        ? `测试成功：${result.latencyMs}ms`
        : `测试失败：${result.error ?? "未知原因"}`;
      window.alert(message);
      void load();
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    } finally {
      setTestingId(null);
    }
  }

  async function handleDelete(channel: Channel) {
    setDeletingId(channel.id);
    try {
      await channelApi.remove(channel.id);
      setDeleteTarget(null);
      setChannels((prev) => prev.filter((c) => c.id !== channel.id));
      void load();
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">渠道</h2>
        <button
          type="button"
          onClick={() => setForm({ channel: null })}
          className="flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
        >
          <Plus className="h-4 w-4" />
          新建渠道
        </button>
      </div>

      {loadError && (
        <p className="text-sm text-danger" role="alert">
          加载失败：{loadError}
        </p>
      )}

      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <table className="w-full">
          <thead className="bg-muted text-left text-xs uppercase text-muted-foreground">
            <tr>
              <th className="px-3 py-2">名称</th>
              <th className="px-3 py-2">类型</th>
              <th className="px-3 py-2">Base URL</th>
              <th className="px-3 py-2">模型</th>
              <th className="px-3 py-2">优先级</th>
              <th className="px-3 py-2">权重</th>
              <th className="px-3 py-2">最近测试</th>
              <th className="px-3 py-2">状态</th>
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
            ) : channels.length === 0 ? (
              <tr>
                <td className={cellCls} colSpan={9}>
                  暂无渠道，点击右上角「新建渠道」创建。
                </td>
              </tr>
            ) : (
              channels.map((channel) => (
                <tr key={channel.id} className="hover:bg-muted/50">
                  <td className={`${cellCls} font-medium`}>{channel.name}</td>
                  <td className={cellCls}>
                    <span className="rounded bg-accent px-1.5 py-0.5 text-xs text-accent-foreground">
                      {CHANNEL_TYPE_LABELS[channel.channelType]}
                    </span>
                  </td>
                  <td
                    className={`${cellCls} max-w-40 truncate text-muted-foreground`}
                    title={channel.baseUrl ?? undefined}
                  >
                    {channel.baseUrl ?? "—"}
                  </td>
                  <td
                    className={`${cellCls} max-w-48 truncate text-muted-foreground`}
                    title={channel.models.join(", ")}
                  >
                    {channel.models.length ? channel.models.join(", ") : "—"}
                  </td>
                  <td className={cellCls}>{channel.priority}</td>
                  <td className={cellCls}>{channel.weight}</td>
                  <td className={cellCls}>
                    {channel.lastTestAt ? (
                      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <span
                          className={`inline-block h-1.5 w-1.5 rounded-full ${
                            channel.lastTestOk ? "bg-success" : "bg-danger"
                          }`}
                          aria-hidden
                        />
                        {formatTestTime(channel.lastTestAt)}
                      </div>
                    ) : (
                      <span className="text-xs text-muted-foreground">
                        未测试
                      </span>
                    )}
                  </td>
                  <td className={cellCls}>
                    <button
                      type="button"
                      onClick={() => void handleToggle(channel)}
                      aria-pressed={channel.enabled}
                      title={channel.enabled ? "点击禁用" : "点击启用"}
                      className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                        channel.enabled ? "bg-success" : "bg-muted"
                      }`}
                    >
                      <span
                        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                          channel.enabled ? "translate-x-4" : "translate-x-0.5"
                        }`}
                      />
                    </button>
                  </td>
                  <td className={`${cellCls} text-right`}>
                    <div className="flex justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => void handleTest(channel)}
                        disabled={testingId === channel.id}
                        aria-label={`测试 ${channel.name}`}
                        title="连通性测试"
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"
                      >
                        {testingId === channel.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Activity className="h-4 w-4" />
                        )}
                      </button>
                      <button
                        type="button"
                        onClick={() => setForm({ channel })}
                        aria-label={`编辑 ${channel.name}`}
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                      >
                        <Pencil className="h-4 w-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => setDeleteTarget(channel)}
                        aria-label={`删除 ${channel.name}`}
                        disabled={deletingId === channel.id}
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-danger disabled:opacity-50"
                      >
                        {deletingId === channel.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </button>
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {form && (
        <ChannelForm
          initial={form.channel}
          onCancel={() => setForm(null)}
          onSaved={handleSaved}
        />
      )}

      {deleteTarget && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          role="dialog"
          aria-modal="true"
          aria-label="删除渠道"
        >
          <div className="w-full max-w-md rounded-lg border border-border bg-card p-5 shadow-lg">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-base font-semibold">删除渠道</h3>
              <button
                type="button"
                onClick={() => setDeleteTarget(null)}
                aria-label="关闭"
                disabled={deletingId === deleteTarget.id}
                className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <p className="mb-4 text-sm text-muted-foreground">
              确定删除渠道「{deleteTarget.name}」？此操作不可撤销。
            </p>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setDeleteTarget(null)}
                disabled={deletingId === deleteTarget.id}
                className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-muted disabled:opacity-50"
              >
                取消
              </button>
              <button
                type="button"
                onClick={() => void handleDelete(deleteTarget)}
                disabled={deletingId === deleteTarget.id}
                className="flex items-center gap-1 rounded-md bg-danger px-3 py-1.5 text-sm text-white hover:opacity-90 disabled:opacity-50"
              >
                {deletingId === deleteTarget.id && (
                  <Loader2 className="h-4 w-4 animate-spin" />
                )}
                删除
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
