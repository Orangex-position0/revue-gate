// 密钥管理页：本地密钥列表 + CRUD / 启停 / 配额查看（ticket 05）。
// 数据本地 useState + load()，CRUD 后就地刷新（同 ChannelsPage）。创建返回的密钥明文
// 只在创建后一次性展示（create 返回明文，其余路径后端已遮蔽为占位符），关闭后不可再见。
import { useCallback, useEffect, useState } from "react";
import { Check, Pencil, Plus, Trash2, X } from "lucide-react";
import { ApiKeyForm } from "./ApiKeyForm";
import { apiKeyApi, invokeErrorMessage } from "@/lib/api";
import type { ApiKey } from "@/types";

/** 表单状态：null = 关闭；{ key: null } = 新建；{ key } = 编辑。 */
type FormState = { key: ApiKey | null } | null;

const cellCls = "px-3 py-2 align-middle text-sm";

/** 已用 / 上限的展示文案：无上限时只显示已用额度。 */
function quotaLabel(key: ApiKey): string {
  const { used, limit } = key.quota;
  return limit == null ? `${used} / 无上限` : `${used} / ${limit}`;
}

export function ApiKeysPage() {
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(null);
  /** 刚创建的密钥明文（一次性展示弹窗）；null = 无。 */
  const [createdKey, setCreatedKey] = useState<ApiKey | null>(null);
  const [copied, setCopied] = useState(false);

  const load = useCallback(async () => {
    try {
      setKeys(await apiKeyApi.list());
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

  /** 保存回调：关闭表单后重拉列表；新建时弹出一次性明文展示。
   *  创建 / 编辑在打开表单时由 `form.key === null` 区分，故此处可直接判定。 */
  function handleSaved(saved: ApiKey) {
    const isCreate = form?.key === null;
    setForm(null);
    void load();
    if (isCreate) {
      setCreatedKey(saved);
    }
  }

  async function copyCreatedKey() {
    if (!createdKey) return;
    try {
      await navigator.clipboard.writeText(createdKey.key);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  /** 启停：调 set_api_key_enabled，用返回状态就地替换，保证与数据库一致。 */
  async function handleToggle(key: ApiKey) {
    try {
      const updated = await apiKeyApi.setEnabled(key.id, !key.enabled);
      setKeys((prev) => prev.map((k) => (k.id === updated.id ? updated : k)));
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  async function handleDelete(key: ApiKey) {
    if (!window.confirm(`确定删除密钥「${key.name}」？此操作不可撤销。`)) {
      return;
    }
    try {
      await apiKeyApi.remove(key.id);
      setKeys((prev) => prev.filter((k) => k.id !== key.id));
    } catch (error) {
      window.alert(invokeErrorMessage(error));
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">密钥</h2>
        <button
          type="button"
          onClick={() => setForm({ key: null })}
          className="flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
        >
          <Plus className="h-4 w-4" />
          新建密钥
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
              <th className="px-3 py-2">密钥</th>
              <th className="px-3 py-2">配额（已用 / 上限）</th>
              <th className="px-3 py-2">状态</th>
              <th className="px-3 py-2">创建时间</th>
              <th className="px-3 py-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {loading ? (
              <tr>
                <td className={cellCls} colSpan={6}>
                  加载中…
                </td>
              </tr>
            ) : keys.length === 0 ? (
              <tr>
                <td className={cellCls} colSpan={6}>
                  暂无密钥，点击右上角「新建密钥」创建。
                </td>
              </tr>
            ) : (
              keys.map((key) => (
                <tr key={key.id} className="hover:bg-muted/50">
                  <td className={`${cellCls} font-medium`}>{key.name}</td>
                  <td className={`${cellCls} font-mono text-muted-foreground`}>
                    {key.key}
                  </td>
                  <td className={cellCls}>{quotaLabel(key)}</td>
                  <td className={cellCls}>
                    <button
                      type="button"
                      onClick={() => void handleToggle(key)}
                      aria-pressed={key.enabled}
                      title={key.enabled ? "点击禁用" : "点击启用"}
                      className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                        key.enabled ? "bg-success" : "bg-muted"
                      }`}
                    >
                      <span
                        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                          key.enabled ? "translate-x-4" : "translate-x-0.5"
                        }`}
                      />
                    </button>
                  </td>
                  <td className={`${cellCls} text-muted-foreground`}>
                    {new Date(key.createdAt).toLocaleString()}
                  </td>
                  <td className={`${cellCls} text-right`}>
                    <div className="flex justify-end gap-1">
                      <button
                        type="button"
                        onClick={() => setForm({ key })}
                        aria-label={`编辑 ${key.name}`}
                        className="rounded p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                      >
                        <Pencil className="h-4 w-4" />
                      </button>
                      <button
                        type="button"
                        onClick={() => void handleDelete(key)}
                        aria-label={`删除 ${key.name}`}
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
        <ApiKeyForm
          initial={form.key}
          onCancel={() => setForm(null)}
          onSaved={handleSaved}
        />
      )}

      {createdKey && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          role="dialog"
          aria-modal="true"
          aria-label="密钥已创建"
        >
          <div className="w-full max-w-md rounded-lg border border-border bg-card p-5 shadow-lg">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-base font-semibold">密钥已创建</h3>
              <button
                type="button"
                onClick={() => setCreatedKey(null)}
                aria-label="关闭"
                className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <p className="mb-3 text-sm text-danger">
              密钥明文仅此一次展示，关闭后将不可再见，请立即复制保存。
            </p>
            <div className="mb-3 break-all rounded-md border border-border bg-muted p-3 font-mono text-sm">
              {createdKey.key}
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setCreatedKey(null)}
                className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-muted"
              >
                关闭
              </button>
              <button
                type="button"
                onClick={() => void copyCreatedKey()}
                className="flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
              >
                {copied ? (
                  <>
                    <Check className="h-4 w-4" />
                    已复制
                  </>
                ) : (
                  "复制密钥"
                )}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
