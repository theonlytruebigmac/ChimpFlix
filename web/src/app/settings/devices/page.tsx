import { SettingsSessionsClient } from "@/components/SettingsSessionsClient";

/// Devices & sessions — everywhere you're signed in, with one-tap revoke.
/// Thin wrapper in the console design language; all state + logic lives
/// in the client (list sessions, "this device" flag, per-session revoke,
/// "sign out everywhere else").
export default function DevicesSettingsPage() {
  return <SettingsSessionsClient />;
}
