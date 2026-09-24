// Read-only security detector catalog; metadata comes from the backend's registered rules.
import { useCallback, useEffect, useState } from "react";
import { invokeErrorMessage, settingsApi } from "@/lib/api";
import type { RiskLevel, RuleCatalogEntry } from "@/types";

const levelName: Record<RiskLevel, string> = {
  clean: "无风险", info: "提示", low: "低", medium: "中", high: "高", critical: "严重",
};
const categoryName: Record<string, string> = {
  credential: "凭证", sensitivePath: "敏感路径", unicodeObfuscation: "Unicode",
  promptInjection: "提示注入", pii: "个人信息", ToolRisk: "工具操作", NetworkRisk: "网络",
};

export function RuleCatalog() {
  const [rules, setRules] = useState<RuleCatalogEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const load = useCallback(async () => {
    setLoading(true);
    try {
      setRules(await settingsApi.rules());
      setError(null);
    } catch (reason) {
      setError(invokeErrorMessage(reason));
      setRules(null);
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => { void load(); }, [load]);

  return (
    <section className="rounded-xl border border-border bg-card p-5" aria-labelledby="rule-catalog-title">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 id="rule-catalog-title" className="text-base font-semibold">内置检测规则</h3>
        {rules && !loading && <span className="text-xs text-muted-foreground">{rules.length} 条运行中规则 · 只读</span>}
      </div>
      <p className="mt-1 text-xs text-muted-foreground">默认等级不等于必然阻断，最终处置受审计策略控制。本轮不提供逐条编辑或启停。</p>
      {loading && <p className="mt-4 text-sm text-muted-foreground" role="status">正在读取规则目录…</p>}
      {error && <div className="mt-4 text-sm text-danger" role="alert">规则目录加载失败：{error} <button type="button" onClick={() => void load()} className="underline">重试</button></div>}
      {rules && !loading && rules.length === 0 && <p className="mt-4 text-sm text-muted-foreground">目前没有内置检测规则。</p>}
      {rules && !loading && rules.length > 0 && (
        <div className="mt-4 overflow-x-auto">
          <table className="w-full min-w-[580px] text-left text-xs">
            <thead className="border-b border-border text-muted-foreground"><tr><th scope="col" className="px-2 py-3 font-medium">规则 ID</th><th scope="col" className="px-2 py-3 font-medium">名称</th><th scope="col" className="px-2 py-3 font-medium">类别</th><th scope="col" className="px-2 py-3 font-medium">默认等级</th><th scope="col" className="px-2 py-3 font-medium">检测范围</th></tr></thead>
            <tbody>{rules.map((rule) => <tr key={rule.id} className="border-b border-border last:border-0"><td className="px-2 py-3 font-mono text-[11px]">{rule.id}</td><td className="px-2 py-3 font-medium">{rule.name}</td><td className="px-2 py-3">{categoryName[rule.category] ?? rule.category}</td><td className="px-2 py-3">{levelName[rule.defaultRiskLevel]}</td><td className="px-2 py-3 text-muted-foreground">{rule.description}</td></tr>)}</tbody>
          </table>
        </div>
      )}
    </section>
  );
}
