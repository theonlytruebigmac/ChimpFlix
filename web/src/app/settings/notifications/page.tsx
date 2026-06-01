import { requireUser } from "@/lib/chimpflix-server";
import { SettingsNotificationsClient } from "@/components/SettingsNotificationsClient";

/// Personal notification preferences — per-kind mute, email channel, and a
/// quiet-hours window. Prefs persist on the user (notification_prefs_json)
/// and are enforced server-side by the notifier.
export default async function NotificationsSettingsPage() {
  const user = await requireUser("/settings/notifications");
  return (
    <div className="divide-y divide-white/10">
      <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
          Notifications
        </h2>
        <div>
          <SettingsNotificationsClient initial={user} />
        </div>
      </section>
    </div>
  );
}
