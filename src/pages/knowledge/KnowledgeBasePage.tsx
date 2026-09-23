// Knowledge base workspace: CRUD, document status, and configurable retrieval search.
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { BookOpen, Database, Loader2, Plus, Search, Trash2, X } from "lucide-react";
import { knowledgeApi, invokeErrorMessage } from "@/lib/api";
import type { KnowledgeBase, KnowledgeDocument, KnowledgeSearchResponse } from "@/types";

export function KnowledgeBasePage() {
  const [items, setItems] = useState<KnowledgeBase[]>([]);
  const [selected, setSelected] = useState<KnowledgeBase | null>(null);
  const [documents, setDocuments] = useState<KnowledgeDocument[]>([]);
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<"hybrid" | "vector" | "keyword">("hybrid");
  const [results, setResults] = useState<KnowledgeSearchResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createName, setCreateName] = useState("");
  const [creating, setCreating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<KnowledgeBase | null>(null);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await knowledgeApi.list();
      setItems(next);
      setSelected((current) => next.find((item) => item.id === current?.id) ?? next[0] ?? null);
      setError(null);
    } catch (reason) {
      setError(invokeErrorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (!selected) {
      setDocuments([]);
      return;
    }
    void knowledgeApi.documents(selected.id).then(setDocuments).catch(() => setDocuments([]));
  }, [selected]);

  async function create() {
    if (!createName.trim()) return;
    setCreating(true);
    try {
      await knowledgeApi.create({ name: createName.trim() });
      setCreateName("");
      await load();
    } catch (reason) {
      setError(invokeErrorMessage(reason));
    } finally {
      setCreating(false);
    }
  }

  async function remove() {
    if (!deleteTarget) return;
    setDeleting(true);
    try {
      await knowledgeApi.remove(deleteTarget.id);
      setDeleteTarget(null);
      await load();
    } catch (reason) {
      setError(invokeErrorMessage(reason));
    } finally {
      setDeleting(false);
    }
  }

  async function search(event: FormEvent) {
    event.preventDefault();
    if (!selected || !query.trim()) return;
    setSearching(true);
    try {
      setResults(await knowledgeApi.search({ query: query.trim(), kbId: selected.id, mode, limit: 8, scope: {}, filters: {}, fusion: { vectorWeight: 0.7, keywordWeight: 0.3 } }));
    } catch (reason) {
      setError(invokeErrorMessage(reason));
    } finally {
      setSearching(false);
    }
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">知识库</h2>
          <p className="mt-1 text-sm text-muted-foreground">管理文档索引，并用混合检索验证召回效果。</p>
        </div>
        <div className="flex w-full gap-2 sm:w-auto">
          <input value={createName} onChange={(event) => setCreateName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void create(); }} placeholder="新知识库名称" aria-label="新知识库名称" className="h-9 min-w-0 flex-1 rounded-lg border border-border bg-card px-3 text-sm sm:w-48" />
          <button type="button" className="inline-flex h-9 shrink-0 items-center gap-1.5 rounded-lg bg-primary px-3 text-sm font-medium text-primary-foreground" onClick={() => void create()} disabled={creating || !createName.trim()}>
            {creating ? <Loader2 className="h-4 w-4 animate-spin" /> : <Plus className="h-4 w-4" />} 新建
          </button>
        </div>
      </div>

      {error && <div className="flex items-center justify-between rounded-xl border border-danger/20 bg-danger/10 px-3 py-2 text-sm text-danger" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)} aria-label="关闭提示"><X className="h-4 w-4" /></button></div>}

      <div className="grid gap-4 lg:grid-cols-[18rem_1fr]">
        <aside className="rounded-2xl border border-border bg-card p-3 shadow-sm">
          <div className="mb-2 flex items-center justify-between px-1"><span className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">我的知识库</span><Database className="h-4 w-4 text-muted-foreground" /></div>
          {loading ? <div className="flex items-center gap-2 p-3 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />加载中…</div> : items.length === 0 ? <div className="rounded-xl border border-dashed border-border p-5 text-center"><BookOpen className="mx-auto h-7 w-7 text-muted-foreground/60" /><p className="mt-2 text-sm font-medium">还没有知识库</p><p className="mt-1 text-xs leading-5 text-muted-foreground">创建一个知识库开始管理文档。</p></div> : <div className="space-y-1">{items.map((item) => <button type="button" key={item.id} className={`block w-full rounded-xl p-3 text-left transition-colors ${selected?.id === item.id ? "bg-accent text-accent-foreground" : "hover:bg-muted"}`} onClick={() => { setSelected(item); setResults(null); }}><span className="block truncate text-sm font-medium">{item.name}</span><span className="mt-1 block text-xs text-muted-foreground">{item.docCount} 文档 · {item.chunkCount} 分块</span></button>)}</div>}
          {selected && <button type="button" className="mt-4 inline-flex items-center gap-1.5 px-2 text-xs text-danger hover:underline" onClick={() => setDeleteTarget(selected)}><Trash2 className="h-3.5 w-3.5" />删除当前知识库</button>}
        </aside>

        <main className="space-y-4">
          {!selected ? <div className="rounded-2xl border border-dashed border-border bg-card p-12 text-center text-muted-foreground"><BookOpen className="mx-auto h-10 w-10 opacity-50" /><p className="mt-3 text-sm">选择或创建一个知识库</p></div> : <>
            <section className="rounded-2xl border border-border bg-card p-5 shadow-sm"><div className="flex items-start justify-between gap-3"><div><h2 className="text-lg font-semibold">{selected.name}</h2><p className="mt-1 text-sm text-muted-foreground">{selected.description ?? "未填写描述"}</p></div><span className="rounded-full bg-accent px-2.5 py-1 text-xs text-accent-foreground">{selected.indexStatus}</span></div><div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-3"><div><div className="text-xs text-muted-foreground">文档</div><div className="mt-1 text-lg font-semibold tabular-nums">{selected.docCount}</div></div><div><div className="text-xs text-muted-foreground">分块</div><div className="mt-1 text-lg font-semibold tabular-nums">{selected.chunkCount}</div></div><div><div className="text-xs text-muted-foreground">Token</div><div className="mt-1 text-lg font-semibold tabular-nums">{selected.totalTokens.toLocaleString()}</div></div></div></section>
            <section className="rounded-2xl border border-border bg-card p-5 shadow-sm"><div className="mb-3"><h3 className="font-semibold">检索测试</h3><p className="mt-1 text-xs text-muted-foreground">输入一个问题，检查当前索引能否召回相关内容。</p></div><form className="flex flex-col gap-2 sm:flex-row" onSubmit={(event) => void search(event)}><div className="relative min-w-0 flex-1"><Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="输入检索问题" className="h-9 w-full rounded-lg border border-border bg-background pl-9 pr-3 text-sm" /></div><select value={mode} onChange={(event) => setMode(event.target.value as typeof mode)} className="h-9 rounded-lg border border-border bg-background px-2 text-sm"><option value="hybrid">混合检索</option><option value="vector">向量检索</option><option value="keyword">关键词检索</option></select><button type="submit" disabled={searching || !query.trim()} className="inline-flex h-9 items-center justify-center gap-1.5 rounded-lg bg-primary px-3 text-sm text-primary-foreground disabled:opacity-50">{searching && <Loader2 className="h-4 w-4 animate-spin" />}搜索</button></form>{results && <div className="mt-4 space-y-2">{results.results.length === 0 ? <p className="rounded-xl bg-muted p-4 text-center text-sm text-muted-foreground">没有召回结果，尝试换一个问题或检索模式。</p> : results.results.map((result) => <article key={result.chunkId} className="rounded-xl border border-border bg-background/60 p-3"><div className="flex justify-between gap-3 text-xs text-muted-foreground"><span className="truncate">{result.citation.filename as string ?? result.chunkId}</span><span className="shrink-0 font-mono">{result.score.toFixed(4)}</span></div><p className="mt-1.5 text-sm leading-6">{result.snippet}</p></article>)}</div>}</section>
            <section className="rounded-2xl border border-border bg-card p-5 shadow-sm"><div className="mb-2 flex items-center justify-between"><h3 className="font-semibold">文档</h3><span className="text-xs text-muted-foreground">{documents.length} 个文档</span></div>{documents.length === 0 ? <p className="rounded-xl bg-muted p-4 text-center text-sm text-muted-foreground">暂无文档</p> : <ul className="divide-y divide-border">{documents.map((document) => <li key={document.id} className="flex items-center justify-between gap-3 py-3 text-sm"><span className="min-w-0 truncate">{document.filename}</span><span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">{document.status}</span></li>)}</ul>}</section>
          </>}
        </main>
      </div>

      {deleteTarget && <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-label="删除知识库"><div className="w-full max-w-sm rounded-2xl border border-border bg-card p-5 shadow-2xl"><div className="flex items-start justify-between"><div><h3 className="font-semibold">删除知识库？</h3><p className="mt-1 text-sm text-muted-foreground">「{deleteTarget.name}」及其索引内容将被删除。</p></div><button type="button" onClick={() => setDeleteTarget(null)} disabled={deleting} aria-label="关闭"><X className="h-4 w-4 text-muted-foreground" /></button></div><div className="mt-5 flex justify-end gap-2"><button type="button" onClick={() => setDeleteTarget(null)} disabled={deleting} className="rounded-lg border border-border px-3 py-1.5 text-sm hover:bg-muted">取消</button><button type="button" onClick={() => void remove()} disabled={deleting} className="inline-flex items-center gap-1.5 rounded-lg bg-danger px-3 py-1.5 text-sm text-white">{deleting && <Loader2 className="h-4 w-4 animate-spin" />}删除</button></div></div></div>}
    </div>
  );
}
