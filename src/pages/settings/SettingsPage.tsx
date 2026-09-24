// Settings center: five local subroutes with independent drafts and scoped persistence.
// Runtime server status comes only from server events, never from the start/stop button.
import { useEffect, useRef, useState } from "react";
import { NavLink, Navigate, useParams } from "react-router-dom";
import { ThemeSelector } from "@/components/ThemeSelector";
import { SETTINGS_SECTIONS as sections } from "@/components/layout/nav";
import { RuleCatalog } from "./RuleCatalog";
import { useTheme } from "@/hooks/use-theme";
import { invokeErrorMessage, serverApi, settingsApi } from "@/lib/api";
import { useServerStore } from "@/stores/use-server-store";
import type { GatewaySettings, SettingsPatch } from "@/types";

type Section = (typeof sections)[number]["id"];

const card = "rounded-xl border border-border bg-card p-5";
const input = "w-full rounded-lg border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary";
const label = "block text-sm font-medium";
const hint = "mt-1 text-xs text-muted-foreground";

function Toggle({ text, detail, checked, onChange }: { text: string; detail?: string; checked: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className="flex items-center justify-between gap-4 border-b border-border py-3 last:border-0">
      <span className="text-sm font-medium">{text}{detail && <span className="mt-1 block text-xs font-normal text-muted-foreground">{detail}</span>}</span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="h-4 w-4 shrink-0 accent-[var(--primary)]" />
    </label>
  );
}

export function SettingsPage() {
  const { section } = useParams();
  if (!sections.some((entry) => entry.id === section)) return <Navigate to="/settings/server" replace />;
  return <SettingsSection key={section} section={section as Section} />;
}

function SettingsSection({ section }: { section: Section }) {
  const { theme, changeTheme, themeError } = useTheme();
  const { running, host, port } = useServerStore();
  const [settings, setSettings] = useState<GatewaySettings | null>(null);
  const [portInput, setPortInput] = useState("");
  const [retryInput, setRetryInput] = useState("");
  const [scanInput, setScanInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [serverError, setServerError] = useState<string | null>(null);
  const [serverBusy, setServerBusy] = useState(false);
  const editVersion = useRef(0);

  useEffect(() => {
    let active = true;
    settingsApi.get().then((value) => {
      if (!active) return;
      setSettings(value);
      setPortInput(String(value.port));
      setRetryInput(value.retry.max_retries == null ? "" : String(value.retry.max_retries));
      setScanInput(String(value.audit.scanByteLimit));
      setLoadError(null);
    }).catch((error: unknown) => {
      if (active) setLoadError(invokeErrorMessage(error));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, []);

  function changeField<K extends keyof GatewaySettings>(key: K, value: GatewaySettings[K]) {
    editVersion.current += 1;
    setSettings((previous) => previous ? { ...previous, [key]: value } : previous);
    setSaved(false);
    setDirty(true);
  }
  function changeAudit<K extends keyof GatewaySettings["audit"]>(key: K, value: GatewaySettings["audit"][K]) {
    editVersion.current += 1;
    setSettings((previous) => previous ? { ...previous, audit: { ...previous.audit, [key]: value } } : previous);
    setSaved(false);
    setDirty(true);
  }

  async function save() {
    if (!settings || saving) return;
    setSaveError(null);
    setSaved(false);
    let patch: SettingsPatch;
    if (section === "server") {
      const parsed = Number(portInput);
      if (portInput.trim() === "" || !Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
        setSaveError("端口必须是 0–65535 的整数，0 表示随机可用端口。");
        return;
      }
      patch = { section, value: { host: settings.host, port: parsed } };
    } else if (section === "desktop") {
      patch = { section, value: { minimizeToTray: settings.minimizeToTray, closeToTray: settings.closeToTray, autostart: settings.autostart } };
    } else if (section === "retry") {
      const parsed = retryInput.trim() === "" ? null : Number(retryInput);
      if (parsed !== null && (!Number.isInteger(parsed) || parsed < 0 || parsed > 4294967295)) {
        setSaveError("额外重试次数必须是非负整数，留空表示无上限。");
        return;
      }
      patch = { section, value: { ...settings.retry, max_retries: parsed } };
    } else if (section === "security") {
      const parsed = Number(scanInput);
      if (scanInput.trim() === "" || !Number.isInteger(parsed) || parsed < 0 || parsed > 10_000_000) {
        setSaveError("扫描字节上限必须是 0–10000000 的整数。");
        return;
      }
      patch = { section, value: { ...settings.audit, scanByteLimit: parsed } };
    } else return;
    const requestVersion = editVersion.current;
    setSaving(true);
    try {
      const result = await settingsApi.saveSection(patch);
      if (editVersion.current === requestVersion) {
        setSettings(result);
        setDirty(false);
        setSaved(true);
      }
    } catch (error) {
      setSaveError(invokeErrorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  async function setServerRunning(start: boolean) {
    setServerBusy(true);
    setServerError(null);
    try {
      if (start) await serverApi.start();
      else await serverApi.stop();
    } catch (error) {
      setServerError(invokeErrorMessage(error));
    } finally {
      setServerBusy(false);
    }
  }

  const current = sections.find((entry) => entry.id === section)!;
  const saveControls = (
    <div className="flex flex-wrap items-center justify-end gap-3">
      {dirty && !saving && <span className="text-sm text-muted-foreground" role="status">此页有未保存更改</span>}
      {saved && <span className="text-sm text-success" role="status">此页设置已保存</span>}
      {saveError && <span className="text-sm text-danger" role="alert">保存失败：{saveError}</span>}
      <button type="button" onClick={() => void save()} disabled={saving} className="rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-primary-foreground disabled:opacity-50">{saving ? "保存中…" : "保存此页设置"}</button>
    </div>
  );
  return (
    <div className="space-y-5">
      <div><p className="text-xs font-semibold text-primary">设置中心 / {current.title}</p><h2 className="mt-1 text-2xl font-semibold">{current.title}</h2><p className="mt-1 text-sm text-muted-foreground">{current.description}</p></div>
      <div className="grid items-start gap-5 md:grid-cols-[190px_minmax(0,1fr)]">
        <nav aria-label="设置分类" className="flex gap-1 overflow-x-auto rounded-xl border border-border bg-card p-2 md:flex-col">
          {sections.map((entry) => <NavLink key={entry.id} to={`/settings/${entry.id}`} className={({ isActive }) => `shrink-0 rounded-lg px-3 py-2 text-sm focus-visible:outline-2 focus-visible:outline-primary ${isActive ? "bg-accent font-semibold text-primary" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}>{entry.title}</NavLink>)}
        </nav>
        <div className="min-w-0 space-y-4">
          {loading && <p className="text-sm text-muted-foreground" role="status">加载中…</p>}
          {loadError && <p className="text-sm text-danger" role="alert">设置加载失败：{loadError}</p>}
          {settings && !loading && section === "server" && <>
            <section className={card}><h3 className="text-base font-semibold">服务运行</h3><p className="mt-1 text-sm text-muted-foreground">{running ? `数据面监听 ${host}:${port}` : "数据面未启动"}，运行状态由服务事件更新。</p>
              <button type="button" disabled={serverBusy} onClick={() => void setServerRunning(!running)} className="mt-4 rounded-lg border border-border px-4 py-2 text-sm text-primary hover:bg-accent disabled:opacity-50">{serverBusy ? "处理中…" : running ? "停止服务" : "启动服务"}</button>
              {serverError && <p className="mt-2 text-sm text-danger" role="alert">{serverError}</p>}
            </section>
            <section className={card}><h3 className="mb-3 text-base font-semibold">服务监听</h3><div className="grid gap-4 sm:grid-cols-2"><div><label htmlFor="settings-host" className={label}>Host</label><input id="settings-host" className={input} value={settings.host} onChange={(event) => changeField("host", event.target.value)} /></div><div><label htmlFor="settings-port" className={label}>端口</label><input id="settings-port" type="number" min={0} max={65535} className={input} value={portInput} onChange={(event) => { editVersion.current += 1; setPortInput(event.target.value); setSaved(false); setDirty(true); }} /><p className={hint}>0 表示随机可用端口；启动服务使用已保存的值。</p></div></div></section>
          </>}
          {settings && !loading && section === "desktop" && <section className={card}><h3 className="text-base font-semibold">窗口与托盘</h3>
            <Toggle text="最小化到托盘" checked={settings.minimizeToTray} onChange={(value) => changeField("minimizeToTray", value)} />
            <Toggle text="关闭窗口时隐藏到托盘" checked={settings.closeToTray} onChange={(value) => changeField("closeToTray", value)} />
            <Toggle text="开机自启" detail="保存时同步操作系统启动项，失败时不会标记已保存。" checked={settings.autostart} onChange={(value) => changeField("autostart", value)} />
          </section>}
          {settings && !loading && section === "appearance" && <section className={card}><h3 className="mb-3 text-base font-semibold">界面主题</h3><ThemeSelector theme={theme} onChange={changeTheme} /><p className={hint}>主题切换即时应用并单独保存，不覆盖其他子页。</p>{themeError && <p className="mt-2 text-sm text-danger" role="alert">主题保存失败：{themeError}</p>}</section>}
          {settings && !loading && section === "retry" && <section className={card}><h3 className="text-base font-semibold">失败重试</h3><Toggle text="启用失败重试" checked={settings.retry.enabled} onChange={(value) => { changeField("retry", { ...settings.retry, enabled: value }); }} /><div className="mt-4 max-w-sm"><label className={label} htmlFor="settings-retry">最多额外重试次数</label><input id="settings-retry" className={input} type="number" min={0} value={retryInput} onChange={(event) => { editVersion.current += 1; setRetryInput(event.target.value); setSaved(false); setDirty(true); }} disabled={!settings.retry.enabled} /><p className={hint}>首次失败后按渠道顺序重试；留空表示没有额外次数上限。</p></div></section>}
          {settings && !loading && section === "security" && <>
            <section className={card}><h3 className="text-base font-semibold">请求审计</h3><Toggle text="启用安全审计" detail="只检查后续请求，不重扫历史日志；关闭不影响存储副本的脱敏。" checked={settings.audit.enabled} onChange={(value) => changeAudit("enabled", value)} />
              <div className="mt-4 grid gap-4 sm:grid-cols-2"><div><label htmlFor="settings-mode" className={label}>执行模式</label><select id="settings-mode" className={input} value={settings.audit.mode} onChange={(event) => changeAudit("mode", event.target.value as GatewaySettings["audit"]["mode"])} disabled={!settings.audit.enabled}><option value="observe">观察（记录，不阻断）</option><option value="enforce">拦截（按 critical 策略）</option></select></div><div><label htmlFor="settings-limit" className={label}>扫描字节上限</label><input id="settings-limit" className={input} type="number" min={0} max={10000000} value={scanInput} onChange={(event) => { editVersion.current += 1; setScanInput(event.target.value); setSaved(false); setDirty(true); }} disabled={!settings.audit.enabled} /></div><div><label htmlFor="settings-evidence" className={label}>证据级别</label><select id="settings-evidence" className={input} value={settings.audit.evidenceLevel} onChange={(event) => changeAudit("evidenceLevel", event.target.value as GatewaySettings["audit"]["evidenceLevel"])} disabled={!settings.audit.enabled}><option value="summary">摘要</option><option value="detailed">详细</option></select></div></div>
              <Toggle text="阻断 critical 风险" detail="仅拦截模式下对符合条件的严重风险生效。" checked={settings.audit.blockCritical} onChange={(value) => changeAudit("blockCritical", value)} />
              <Toggle text="扫描 system 消息" checked={settings.audit.scanSystemMessages} onChange={(value) => changeAudit("scanSystemMessages", value)} />
              <Toggle text="保存脱敏后的请求体" detail="本地存储脱敏独立于审计总开关，不改写发往上游的内容。" checked={settings.audit.storePayload} onChange={(value) => changeAudit("storePayload", value)} />
            </section>
            {saveControls}
            <RuleCatalog />
          </>}
          {settings && !loading && section !== "appearance" && section !== "security" && saveControls}
        </div>
      </div>
    </div>
  );
}
