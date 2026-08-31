// Settings center page: server start/stop + host/port / theme / tray and autostart / failure retry policy.
// Data is local useState + load() (business data does not go into the store, see Architecture-frontend.md).
// The start/stop buttons only call serverApi: the running state is synced via useServerStore from server events, never mutated here.
// The theme is held in a single place by useTheme (same source as the top bar); saving merges it into the full settings package.
import { useCallback, useEffect, useState } from "react";
import { ThemeSelector } from "@/components/ThemeSelector";
import { useTheme } from "@/hooks/use-theme";
import { serverApi, settingsApi, invokeErrorMessage } from "@/lib/api";
import { useServerStore } from "@/stores/use-server-store";
import type { GatewaySettings } from "@/types";

const cardCls = "rounded-lg border border-border bg-card p-4";
const labelCls = "block text-sm font-medium";
const inputCls =
  "w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50";
const hintCls = "mt-1 text-xs text-muted-foreground";

export function SettingsPage() {
  const { theme, changeTheme } = useTheme();
  const { running, host, port } = useServerStore();
  const [settings, setSettings] = useState<GatewaySettings | null>(null);
  // Numeric fields are carried as strings and parsed/validated on save (consistent with the ApiKeyForm quota semantics).
  const [portInput, setPortInput] = useState("");
  const [retryInput, setRetryInput] = useState("");
  const [auditScanLimitInput, setAuditScanLimitInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [serverError, setServerError] = useState<string | null>(null);
  const [serverBusy, setServerBusy] = useState<"start" | "stop" | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const loaded = await settingsApi.get();
      setSettings(loaded);
      setPortInput(String(loaded.port));
      setRetryInput(
        loaded.retry.max_retries != null ? String(loaded.retry.max_retries) : "",
      );
      setAuditScanLimitInput(String(loaded.audit.scanByteLimit));
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

  /** Update boolean / host fields in place (immutably). */
  const setField = <K extends keyof GatewaySettings>(
    key: K,
    value: GatewaySettings[K],
  ) => setSettings((prev) => (prev ? { ...prev, [key]: value } : prev));

  const setRetryEnabled = (enabled: boolean) =>
    setSettings((prev) =>
      prev ? { ...prev, retry: { ...prev.retry, enabled } } : prev,
    );

  const setAuditField = <K extends keyof GatewaySettings["audit"]>(
    key: K,
    value: GatewaySettings["audit"][K],
  ) =>
    setSettings((prev) =>
      prev ? { ...prev, audit: { ...prev.audit, [key]: value } } : prev,
    );

  async function handleSave() {
    if (!settings) return;
    const parsedPort = Number(portInput);
    if (!Number.isInteger(parsedPort) || parsedPort < 0 || parsedPort > 65535) {
      setSaveError("端口必须是 0-65535 的整数（0 = 随机端口）");
      return;
    }
    const parsedRetry = retryInput.trim() === "" ? null : Number(retryInput);
    if (
      parsedRetry !== null &&
      (!Number.isInteger(parsedRetry) || parsedRetry < 0)
    ) {
      setSaveError("重试次数必须是非负整数，留空表示无上限");
      return;
    }
    const parsedScanLimit = Number(auditScanLimitInput);
    if (
      !Number.isInteger(parsedScanLimit) ||
      parsedScanLimit < 0 ||
      parsedScanLimit > 10_000_000
    ) {
      setSaveError("审计扫描字节上限必须是 0-10000000 的整数");
      return;
    }
    setSaving(true);
    setSaveError(null);
    setSavedAt(null);
    try {
      await settingsApi.save({
        host: settings.host,
        port: parsedPort,
        theme,
        minimizeToTray: settings.minimizeToTray,
        closeToTray: settings.closeToTray,
        autostart: settings.autostart,
        retry: { enabled: settings.retry.enabled, max_retries: parsedRetry },
        audit: {
          ...settings.audit,
          scanByteLimit: parsedScanLimit,
        },
        serviceModules: settings.serviceModules,
      });
      setSavedAt(new Date().toLocaleTimeString());
    } catch (error) {
      setSaveError(invokeErrorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  async function handleStart() {
    setServerBusy("start");
    setServerError(null);
    try {
      await serverApi.start();
    } catch (error) {
      setServerError(invokeErrorMessage(error));
    } finally {
      setServerBusy(null);
    }
  }

  async function handleStop() {
    setServerBusy("stop");
    setServerError(null);
    try {
      await serverApi.stop();
    } catch (error) {
      setServerError(invokeErrorMessage(error));
    } finally {
      setServerBusy(null);
    }
  }

  const endpoint = running && host && port ? `${host}:${port}` : null;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xl font-semibold">设置</h2>
        {savedAt && (
          <span className="text-sm text-muted-foreground">
            已保存 {savedAt}
          </span>
        )}
      </div>

      {loading && <p className="text-sm text-muted-foreground">加载中…</p>}
      {loadError && (
        <p className="text-sm text-danger" role="alert">
          加载失败：{loadError}
        </p>
      )}
      {!loading && !loadError && settings && (
        <>
          {/* Server running: start/stop buttons trigger the backend; the status indicator syncs via the event bridge */}
          <section className={cardCls}>
            <h3 className="mb-2 text-base font-semibold">服务运行</h3>
            <div className="flex items-center gap-3">
              {running ? (
                <button
                  type="button"
                  onClick={handleStop}
                  disabled={serverBusy === "stop"}
                  className="rounded-md border border-border px-3 py-1.5 text-sm text-danger hover:bg-muted disabled:opacity-50"
                >
                  {serverBusy === "stop" ? "停止中…" : "停止服务"}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={handleStart}
                  disabled={serverBusy === "start"}
                  className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90 disabled:opacity-50"
                >
                  {serverBusy === "start" ? "启动中…" : "启动服务"}
                </button>
              )}
              <p className="text-sm text-muted-foreground">
                {endpoint ? `数据面监听 ${endpoint}` : "数据面未启动"}
              </p>
            </div>
            <p className={hintCls}>
              启动按已保存的 host / 端口生效；运行状态由服务事件实时同步。
            </p>
            {serverError && (
              <p className="mt-1 text-sm text-danger" role="alert">
                {serverError}
              </p>
            )}
          </section>

          {/* Server listener: host + port (0 = random) */}
          <section className={cardCls}>
            <h3 className="mb-3 text-base font-semibold">服务监听</h3>
            <div className="grid gap-4 sm:grid-cols-2">
              <div>
                <label className={labelCls} htmlFor="settings-host">
                  Host
                </label>
                <input
                  id="settings-host"
                  className={inputCls}
                  value={settings.host}
                  onChange={(e) => setField("host", e.target.value)}
                  placeholder="如 127.0.0.1"
                />
              </div>
              <div>
                <label className={labelCls} htmlFor="settings-port">
                  端口
                </label>
                <input
                  id="settings-port"
                  className={inputCls}
                  type="number"
                  min={0}
                  max={65535}
                  value={portInput}
                  onChange={(e) => setPortInput(e.target.value)}
                  placeholder="如 3000"
                />
                <p className={hintCls}>0 = 随机可用端口。</p>
              </div>
            </div>
          </section>

          {/* Theme tri-state: same source as the top bar; changes apply and persist immediately */}
          <section className={cardCls}>
            <h3 className="mb-3 text-base font-semibold">界面主题</h3>
            <div className="flex items-center gap-3">
              <ThemeSelector theme={theme} onChange={changeTheme} />
              <span className="text-sm text-muted-foreground">
                跟随系统不写暗色变量，由 CSS 媒体查询响应。
              </span>
            </div>
          </section>

          {/* Tray and autostart */}
          <section className={cardCls}>
            <h3 className="mb-3 text-base font-semibold">托盘与开机自启</h3>
            <div className="space-y-2">
              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={settings.minimizeToTray}
                  onChange={(e) => setField("minimizeToTray", e.target.checked)}
                  className="h-4 w-4 accent-[var(--primary)]"
                />
                最小化到托盘
              </label>
              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={settings.closeToTray}
                  onChange={(e) => setField("closeToTray", e.target.checked)}
                  className="h-4 w-4 accent-[var(--primary)]"
                />
                关闭窗口时隐藏到托盘
              </label>
              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={settings.autostart}
                  onChange={(e) => setField("autostart", e.target.checked)}
                  className="h-4 w-4 accent-[var(--primary)]"
                />
                开机自启
              </label>
            </div>
            <p className={hintCls}>
              托盘行为即时生效；开机自启在保存时同步操作系统启动项。
            </p>
          </section>

          {/* Failure retry policy */}
          <section className={cardCls}>
            <h3 className="mb-3 text-base font-semibold">失败重试</h3>
            <div className="space-y-4">
              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={settings.retry.enabled}
                  onChange={(e) => setRetryEnabled(e.target.checked)}
                  className="h-4 w-4 accent-[var(--primary)]"
                />
                启用失败重试
              </label>
              <div className="max-w-xs">
                <label className={labelCls} htmlFor="settings-retry">
                  最大重试次数（留空 = 无上限）
                </label>
                <input
                  id="settings-retry"
                  className={inputCls}
                  type="number"
                  min={0}
                  value={retryInput}
                  onChange={(e) => setRetryInput(e.target.value)}
                  placeholder="留空 = 无上限"
                  disabled={!settings.retry.enabled}
                />
                <p className={hintCls}>
                  首次请求失败后按渠道优先级逐个重试；次数为首次之后的额外尝试次数。
                </p>
              </div>
            </div>
          </section>

          {/* 安全审计策略 */}
          <section className={cardCls}>
            <h3 className="mb-3 text-base font-semibold">安全审计</h3>
            <div className="space-y-4">
              <label className="flex items-center gap-2 text-sm font-medium">
                <input
                  type="checkbox"
                  checked={settings.audit.enabled}
                  onChange={(e) => setAuditField("enabled", e.target.checked)}
                  className="h-4 w-4 accent-[var(--primary)]"
                />
                启用请求审计
              </label>
              <div className="grid gap-4 sm:grid-cols-2">
                <div>
                  <label className={labelCls} htmlFor="settings-audit-mode">
                    模式
                  </label>
                  <select
                    id="settings-audit-mode"
                    className={inputCls}
                    value={settings.audit.mode}
                    onChange={(e) =>
                      setAuditField(
                        "mode",
                        e.target.value as GatewaySettings["audit"]["mode"],
                      )
                    }
                    disabled={!settings.audit.enabled}
                  >
                    <option value="observe">观察</option>
                    <option value="enforce">拦截</option>
                  </select>
                </div>
                <div>
                  <label className={labelCls} htmlFor="settings-audit-limit">
                    扫描字节上限
                  </label>
                  <input
                    id="settings-audit-limit"
                    className={inputCls}
                    type="number"
                    min={0}
                    max={10000000}
                    value={auditScanLimitInput}
                    onChange={(e) => setAuditScanLimitInput(e.target.value)}
                    disabled={!settings.audit.enabled}
                  />
                </div>
                <div>
                  <label className={labelCls} htmlFor="settings-audit-evidence">
                    证据级别
                  </label>
                  <select
                    id="settings-audit-evidence"
                    className={inputCls}
                    value={settings.audit.evidenceLevel}
                    onChange={(e) =>
                      setAuditField(
                        "evidenceLevel",
                        e.target.value as GatewaySettings["audit"]["evidenceLevel"],
                      )
                    }
                    disabled={!settings.audit.enabled}
                  >
                    <option value="summary">摘要</option>
                    <option value="detailed">详细</option>
                  </select>
                </div>
              </div>
              <div className="space-y-2">
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="checkbox"
                    checked={settings.audit.blockCritical}
                    onChange={(e) =>
                      setAuditField("blockCritical", e.target.checked)
                    }
                    disabled={!settings.audit.enabled}
                    className="h-4 w-4 accent-[var(--primary)] disabled:opacity-50"
                  />
                  拦截 critical 风险
                </label>
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="checkbox"
                    checked={settings.audit.scanSystemMessages}
                    onChange={(e) =>
                      setAuditField("scanSystemMessages", e.target.checked)
                    }
                    disabled={!settings.audit.enabled}
                    className="h-4 w-4 accent-[var(--primary)] disabled:opacity-50"
                  />
                  扫描 system messages
                </label>
                <label className="flex items-center gap-2 text-sm font-medium">
                  <input
                    type="checkbox"
                    checked={settings.audit.storePayload}
                    onChange={(e) =>
                      setAuditField("storePayload", e.target.checked)
                    }
                    className="h-4 w-4 accent-[var(--primary)]"
                  />
                  保存原始请求体
                </label>
              </div>
            </div>
          </section>

          {saveError && (
            <p className="text-sm text-danger" role="alert">
              保存失败：{saveError}
            </p>
          )}
          <div className="flex justify-end">
            <button
              type="button"
              onClick={handleSave}
              disabled={saving}
              className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90 disabled:opacity-50"
            >
              {saving ? "保存中…" : "保存设置"}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
