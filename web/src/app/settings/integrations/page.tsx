import { SettingsIntegrationsClient } from "@/components/SettingsIntegrationsClient";

/// Personal integrations. Service-tile grid + the connected-service
/// detail (Trakt today), rendered in the console design language. The
/// page is a thin wrapper; all state lives in the client.
export default function IntegrationsSettingsPage() {
  return <SettingsIntegrationsClient />;
}
