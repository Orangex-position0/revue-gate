// API key form modal: one form shared by create / edit (initial null = create, otherwise = edit).
// The key itself cannot be set via the form (the backend generates it); it is not pre-filled on edit. Empty quota limit = unlimited.
import { useState, type FormEvent } from "react";
import { X } from "lucide-react";
import { apiKeyApi, invokeErrorMessage } from "@/lib/api";
import type { ApiKey, ApiKeyInput } from "@/types";

interface ApiKeyFormProps {
  /** Edit target; null = create mode. */
  initial: ApiKey | null;
  onCancel: () => void;
  onSaved: (saved: ApiKey) => void;
}

const inputCls =
  "w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-primary";
const labelCls = "block text-sm font-medium";

export function ApiKeyForm({ initial, onCancel, onSaved }: ApiKeyFormProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [quotaLimit, setQuotaLimit] = useState(
    initial?.quota.limit != null ? String(initial.quota.limit) : "",
  );
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) {
      setSubmitError("名称不能为空");
      return;
    }
    const input: ApiKeyInput = {
      name: name.trim(),
      quotaLimit: quotaLimit.trim() === "" ? null : Number(quotaLimit),
      enabled,
    };
    setSaving(true);
    try {
      const saved = initial
        ? await apiKeyApi.update(initial.id, input)
        : await apiKeyApi.create(input);
      onSaved(saved);
    } catch (error) {
      setSubmitError(invokeErrorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label={initial ? "编辑密钥" : "新建密钥"}
    >
      <form
        onSubmit={handleSubmit}
        className="w-full max-w-md rounded-lg border border-border bg-card p-5 shadow-lg"
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-base font-semibold">
            {initial ? "编辑密钥" : "新建密钥"}
          </h3>
          <button
            type="button"
            onClick={onCancel}
            aria-label="关闭"
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4">
          <div>
            <label className={labelCls} htmlFor="api-key-name">
              名称
            </label>
            <input
              id="api-key-name"
              className={inputCls}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如 my-client"
              autoFocus
            />
          </div>

          <div>
            <label className={labelCls} htmlFor="api-key-quota">
              配额上限（留空 = 无上限）
            </label>
            <input
              id="api-key-quota"
              className={inputCls}
              type="number"
              min={0}
              value={quotaLimit}
              onChange={(e) => setQuotaLimit(e.target.value)}
              placeholder="如 10000"
            />
            <p className="mt-1 text-xs text-muted-foreground">
              已用额度随代理请求自动累加；达到上限后请求返回 429。
            </p>
          </div>

          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="h-4 w-4 accent-[var(--primary)]"
            />
            启用该密钥
          </label>

          {submitError && (
            <p className="text-sm text-danger" role="alert">
              {submitError}
            </p>
          )}
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-md border border-border px-3 py-1.5 text-sm hover:bg-muted"
          >
            取消
          </button>
          <button
            type="submit"
            disabled={saving}
            className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </form>
    </div>
  );
}
