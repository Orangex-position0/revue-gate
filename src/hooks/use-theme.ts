// 主题 Hook：读取/更新本地存储的主题，切换后立即 applyTheme。顶栏与设置页共用。
import { useEffect, useState } from "react";
import { type Theme, applyTheme, getStoredTheme, setStoredTheme } from "@/lib/theme";

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);

  // 挂载与应用时保持 DOM data-theme 与状态一致（首帧由 main.tsx 渲染前预应用）。
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const changeTheme = (next: Theme) => {
    setStoredTheme(next);
    setTheme(next);
  };

  return { theme, changeTheme };
}
