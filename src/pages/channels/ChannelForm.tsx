// Channel form modal: one form shared by create / edit (initial null = create, otherwise = edit).
// apiKey is not pre-filled on edit; empty = keep the original key (backend update semantics, see usecases/channel.rs).
// The model list is entered as comma-separated text; model mappings use a row editor with add/remove. On submit, calls back onSaved(channel).
import { useState, type FormEvent } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { CHANNEL_TYPE_LABELS } from "@/lib/constants";
import { channelApi, invokeErrorMessage } from "@/lib/api";
import type { Channel, ChannelInput, ChannelType, ModelMapping } from "@/types";

interface ChannelFormProps {
  /** Edit target; null = create mode. */
  initial: Channel | null;
  onCancel: () => void;
  onSaved: (channel: Channel) => void;
}

const inputCls =
  "w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-primary";
const labelCls = "block text-sm font-medium";

/** Model mapping row: id provides a stable key (the list can add/remove; index keys are forbidden, see react/patterns.md). */
interface MappingRow {
  id: string;
  clientModel: string;
  upstreamModel: string;
}

function newMappingRow(): MappingRow {
  return { id: crypto.randomUUID(), clientModel: "", upstreamModel: "" };
}

export function ChannelForm({ initial, onCancel, onSaved }: ChannelFormProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [channelType, setChannelType] = useState<ChannelType>(
    initial?.channelType ?? "openai",
  );
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState(initial?.models.join(", ") ?? "");
  const [priority, setPriority] = useState(initial ? String(initial.priority) : "0");
  const [weight, setWeight] = useState(initial ? String(initial.weight) : "1");
  const [mappings, setMappings] = useState<MappingRow[]>(
    initial?.modelMappings.map((m) => ({ id: crypto.randomUUID(), ...m })) ?? [],
  );
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  function updateMapping(id: string, field: keyof ModelMapping, value: string) {
    setMappings((prev) =>
      prev.map((m) => (m.id === id ? { ...m, [field]: value } : m)),
    );
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) {
      setSubmitError("名称不能为空");
      return;
    }
    const input: ChannelInput = {
      name: name.trim(),
      channelType,
      baseUrl: baseUrl.trim() || null,
      apiKey: apiKey.trim() || null,
      models: models
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      priority: Number(priority) || 0,
      weight: Number(weight) || 0,
      modelMappings: mappings
        .filter((m) => m.clientModel.trim() && m.upstreamModel.trim())
        .map((m) => ({
          clientModel: m.clientModel.trim(),
          upstreamModel: m.upstreamModel.trim(),
        })),
      enabled,
    };
    setSaving(true);
    try {
      const saved = initial
        ? await channelApi.update(initial.id, input)
        : await channelApi.create(input);
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
      aria-label={initial ? "编辑渠道" : "新建渠道"}
    >
      <form
        onSubmit={handleSubmit}
        className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-lg border border-border bg-card p-5 shadow-lg"
      >
        <div className="mb-4 flex items-center justify-between">
          <h3 className="text-base font-semibold">
            {initial ? "编辑渠道" : "新建渠道"}
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
            <label className={labelCls} htmlFor="channel-name">
              名称
            </label>
            <input
              id="channel-name"
              className={inputCls}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如 openai-prod"
              autoFocus
            />
          </div>

          <div>
            <label className={labelCls} htmlFor="channel-type">
              类型
            </label>
            <select
              id="channel-type"
              className={inputCls}
              value={channelType}
              onChange={(e) => setChannelType(e.target.value as ChannelType)}
            >
              {(Object.keys(CHANNEL_TYPE_LABELS) as ChannelType[]).map((t) => (
                <option key={t} value={t}>
                  {CHANNEL_TYPE_LABELS[t]}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className={labelCls} htmlFor="channel-base-url">
              Base URL
            </label>
            <input
              id="channel-base-url"
              className={inputCls}
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="留空由 ProviderAdaptor 提供默认值"
            />
          </div>

          <div>
            <label className={labelCls} htmlFor="channel-api-key">
              上游 API Key
            </label>
            <input
              id="channel-api-key"
              className={inputCls}
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={initial ? "留空保持原密钥" : "sk-..."}
              autoComplete="off"
            />
          </div>

          <div>
            <label className={labelCls} htmlFor="channel-models">
              模型列表（逗号分隔）
            </label>
            <input
              id="channel-models"
              className={inputCls}
              value={models}
              onChange={(e) => setModels(e.target.value)}
              placeholder="gpt-4o, gpt-4o-mini"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls} htmlFor="channel-priority">
                优先级（越小越优先）
              </label>
              <input
                id="channel-priority"
                className={inputCls}
                type="number"
                value={priority}
                onChange={(e) => setPriority(e.target.value)}
              />
            </div>
            <div>
              <label className={labelCls} htmlFor="channel-weight">
                权重
              </label>
              <input
                id="channel-weight"
                className={inputCls}
                type="number"
                value={weight}
                onChange={(e) => setWeight(e.target.value)}
              />
            </div>
          </div>

          <div>
            <div className="mb-1 flex items-center justify-between">
              <span className={labelCls}>模型映射（客户端 → 上游）</span>
              <button
                type="button"
                onClick={() => setMappings((prev) => [...prev, newMappingRow()])}
                className="flex items-center gap-1 text-sm text-primary hover:underline"
              >
                <Plus className="h-3.5 w-3.5" />
                添加映射
              </button>
            </div>
            <div className="space-y-2">
              {mappings.map((m) => (
                <div key={m.id} className="flex items-center gap-2">
                  <input
                    className={inputCls}
                    value={m.clientModel}
                    onChange={(e) =>
                      updateMapping(m.id, "clientModel", e.target.value)
                    }
                    placeholder="客户端模型"
                  />
                  <span className="text-muted-foreground">→</span>
                  <input
                    className={inputCls}
                    value={m.upstreamModel}
                    onChange={(e) =>
                      updateMapping(m.id, "upstreamModel", e.target.value)
                    }
                    placeholder="上游模型"
                  />
                  <button
                    type="button"
                    onClick={() =>
                      setMappings((prev) => prev.filter((row) => row.id !== m.id))
                    }
                    aria-label="删除映射"
                    className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-danger"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              ))}
              {mappings.length === 0 && (
                <p className="text-xs text-muted-foreground">
                  暂无映射；未映射的模型将原样转发。
                </p>
              )}
            </div>
          </div>

          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="h-4 w-4 accent-[var(--primary)]"
            />
            启用该渠道
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
