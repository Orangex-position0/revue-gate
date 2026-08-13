// 渠道管理页：渠道列表 + CRUD / 启停（ticket 04）。
// 数据本地 useState + load()，CRUD 后就地刷新（见 Architecture-frontend.md「业务数据不进 store」）。
import { useCallback, useEffect, useState } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { ChannelForm } from "./ChannelForm";
import { CHANNEL_TYPE_LABELS } from "@/lib/constants";
import { channelApi, invokeErrorMessage } from "@/lib/api";
import type { Channel } from "@/types";

/** 表单状态：null = 关闭；{ channel: null } = 新建；{ channel } = 编辑。 */
type FormState = { channel: Channel | null } | null;

const cellCls = "px-3 py-2 align-middle text-sm";

export function ChannelsPage() {
  const [channels, setChannels] = useState<Channel[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(null);

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

  /** 保存回调：关闭表单后重拉列表——后端按「优先级升序 → 名称升序」排序，
   *  重取保证界面顺序与数据库一致（创建 / 编辑都可能改变排序）。 */
  function handleSaved() {
    setForm(null);
    void load();
  }

  /** 启停：调 set_channel_enabled，用返回状态就地替换，保证与数据库一致。 */
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

  async function handleDelete(channel: Channel) {
    if (!window.confirm(`确定删除渠道「${channel.name}」？此操作不可撤销。`)) {
      return;
    }
    try {
      await channelApi.remove(channel.id);
      setChannels((prev) => prev.filter((c) => c.id !== channel.id));
    } catch (error) {
      window.alert(invokeErrorMessage(error));
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
              <th className="px-3 py-2">状态</th>
              <th className="px-3 py-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {loading ? (
              <tr>
                <td className={cellCls} colSpan={8}>
                  加载中…
                </td>
              </tr>
            ) : channels.length === 0 ? (
              <tr>
                <td className={cellCls} colSpan={8}>
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
                        onClick={() => setForm({ channel })}
                        aria-label={`编辑 ${channel.name}`}
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                      >
                        <Pencil className="h-4 w-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => void handleDelete(channel)}
                        aria-label={`删除 ${channel.name}`}
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-danger"
                      >
                        <Trash2 className="h-4 w-4" />
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
        />      )}
    </div>
  );
}
