import { requireUser } from "@/lib/chimpflix-server";
import { SettingsNotificationsClient } from "@/components/SettingsNotificationsClient";

/// Personal notification preferences — per-kind mute, email channel, and a
/// quiet-hours window (HH:MM in the user's time zone). Prefs persist on the
/// user (notification_prefs_json + notify_via_email + timezone) and are
/// enforced server-side by the notifier. Owner-only kinds (job failures,
/// new signups) render only when the user is an owner. No page title — the
/// SettingsShell sidebar already names the section.
export default async function NotificationsSettingsPage() {
  const user = await requireUser("/settings/notifications");
  return (
    <SettingsNotificationsClient
      initial={user}
      isOwner={user.role === "owner"}
    />
  );
}
