// Primary navigation definition: shared by Sidebar (renders nav items) and TopBar (resolves the current page title).
import type { LucideIcon } from "lucide-react";
import { BarChart3, BookOpen, KeyRound, LayoutDashboard, Network, ScrollText, Settings } from "lucide-react";

export interface NavItem {
  path: string;
  label: string;
  Icon: LucideIcon;
}

export const SETTINGS_SECTIONS = [
  { id: "server", title: "服务配置", description: "配置本地网关监听地址并查看运行状态。" },
  { id: "desktop", title: "桌面行为", description: "设置窗口、托盘和开机自启的行为。" },
  { id: "appearance", title: "外观", description: "选择界面主题。" },
  { id: "retry", title: "重试策略", description: "配置上游渠道失败后的额外尝试。" },
  { id: "security", title: "安全审计", description: "配置后续请求的审计策略。" },
] as const;

export const NAV_ITEMS: NavItem[] = [
  { path: "/dashboard", label: "仪表盘", Icon: LayoutDashboard },
  { path: "/usage", label: "用量", Icon: BarChart3 },
  { path: "/channels", label: "渠道", Icon: Network },
  { path: "/api-keys", label: "密钥", Icon: KeyRound },
  { path: "/logs", label: "日志", Icon: ScrollText },
  { path: "/knowledge", label: "知识库", Icon: BookOpen },
  { path: "/settings", label: "设置", Icon: Settings },
];
