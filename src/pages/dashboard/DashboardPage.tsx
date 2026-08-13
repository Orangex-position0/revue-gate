// 仪表盘页：统计卡片 + 7 天趋势（ticket 11 填充完整内容，本阶段为可导航壳页）。
export function DashboardPage() {
  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">仪表盘</h2>
      <p className="text-sm text-muted-foreground">
        请求数 / Token 统计卡片与 7 天趋势折线将显示于此。
      </p>
    </div>
  );
}
