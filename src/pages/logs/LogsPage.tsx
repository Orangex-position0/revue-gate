// 请求日志页：分页 / 筛选 / 详情 / 删除（ticket 10 填充完整内容，本阶段为可导航壳页）。
export function LogsPage() {
  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">日志</h2>
      <p className="text-sm text-muted-foreground">
        请求日志列表与筛选、详情查看入口将显示于此。
      </p>
    </div>
  );
}
