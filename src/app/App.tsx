// Root app component: Layout and routes, including independent settings subroutes.
import { MemoryRouter, Navigate, Route, Routes } from "react-router-dom";
import { AppLayout } from "@/components/layout/AppLayout";
import { ApiKeysPage } from "@/pages/api-keys/ApiKeysPage";
import { ChannelsPage } from "@/pages/channels/ChannelsPage";
import { DashboardPage } from "@/pages/dashboard/DashboardPage";
import { LogsPage } from "@/pages/logs/LogsPage";
import { KnowledgeBasePage } from "@/pages/knowledge/KnowledgeBasePage";
import { SettingsPage } from "@/pages/settings/SettingsPage";
import { UsagePage } from "@/pages/usage/UsagePage";
import { ThemeProvider } from "@/hooks/use-theme";

export function App() {
  return (
    <ThemeProvider>
      <MemoryRouter>
        <Routes>
        <Route element={<AppLayout />}>
          <Route index element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<DashboardPage />} />
          <Route path="/usage" element={<UsagePage />} />
          <Route path="/channels" element={<ChannelsPage />} />
          <Route path="/api-keys" element={<ApiKeysPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/knowledge" element={<KnowledgeBasePage />} />
          <Route path="/settings" element={<Navigate to="/settings/server" replace />} />
          <Route path="/settings/:section" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/dashboard" replace />} />
        </Route>
        </Routes>
      </MemoryRouter>
    </ThemeProvider>
  );
}
