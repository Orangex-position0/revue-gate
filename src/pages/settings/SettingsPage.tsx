// 设置中心页：端口 / 主题 / 托盘 / 自启 / 重试策略（ticket 12 填充完整内容，本阶段为可导航壳页）。
export function SettingsPage() {
  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">设置</h2>
      <p className="text-sm text-muted-foreground">
        数据面端口、主题、托盘与开机自启、失败重试策略将显示于此。
      </p>
    </div>
  );
}
