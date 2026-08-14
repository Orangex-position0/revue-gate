// Root app component: Layout wrapping 5 page routes (thin shell). Routes are defined here; no business logic or startup side effects.
import { MemoryRouter, Navigate, Route, Routes } from "react-router-dom";
import { AppLayout } from "@/components/layout/AppLayout";
import { ApiKeysPage } from "@/pages/api-keys/ApiKeysPage";
import { ChannelsPage } from "@/pages/channels/ChannelsPage";
import { DashboardPage } from "@/pages/dashboard/DashboardPage";
import { LogsPage } from "@/pages/logs/LogsPage";
import { SettingsPage } from "@/pages/settings/SettingsPage";

export function App() {
  return (
    <MemoryRouter>
      <Routes>
        <Route element={<AppLayout />}>
          <Route index element={<Navigate to="/dashboard" replace />} />
          <Route path="/dashboard" element={<DashboardPage />} />
          <Route path="/channels" element={<ChannelsPage />} />
          <Route path="/api-keys" element={<ApiKeysPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/dashboard" replace />} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}
