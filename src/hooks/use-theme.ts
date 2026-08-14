// 主题 Hook：读取/更新主题，切换后立即 applyTheme，并持久化到后端设置（settingsApi）。
// 后端是主题权威源：挂载时加载并校准；localStorage 仅作渲染前首帧缓存（main.tsx 预应用）。
// 顶栏与设置页各持一份实例，各管各的加载；持久化在每次 changeTheme 时重新读取后端
// 最新设置再合并主题，避免覆盖设置页刚保存的其它字段（不缓存会过期的设置快照）。
import { useEffect, useRef, useState } from "react";
import { settingsApi } from "@/lib/api";
import { applyTheme, getStoredTheme, setStoredTheme, type Theme } from "@/lib/theme";

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(getStoredTheme);
  // 挂载加载完成前用户已手动切换过主题：不再用后端值覆盖（避免「点深色又弹回」）。
  const userChanged = useRef(false);

  // 挂载时加载后端设置：校准主题（权威源）。
  useEffect(() => {
    let cancelled = false;
    settingsApi
      .get()
      .then((settings) => {
        if (cancelled) return;
        if (!userChanged.current) setTheme(settings.theme);
      })
      .catch(() => {}); // 后端不可用（纯浏览器 dev）时保持 localStorage 兜底
    return () => {
      cancelled = true;
    };
  }, []);

  // 主题变化即应用（system 由 CSS 媒体查询响应）。
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const changeTheme = (next: Theme) => {
    userChanged.current = true;
    setStoredTheme(next);
    setTheme(next);
    // 持久化：每次先读后端最新设置再合并主题，失败静默（下次加载仍会校准）。
    void settingsApi
      .get()
      .then((settings) => settingsApi.save({ ...settings, theme: next }))
      .catch(() => {});
  };

  return { theme, changeTheme };
}
