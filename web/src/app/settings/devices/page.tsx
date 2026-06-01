import { SettingsSessionsClient } from "@/components/SettingsSessionsClient";

/// Devices & sessions — everywhere you're signed in, with one-tap revoke.
/// (Moved out of the Account page into its own personal section.)
export default function DevicesSettingsPage() {
  return (
    <div className="divide-y divide-white/10">
      <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
          Devices &amp; sessions
        </h2>
        <div className="space-y-3">
          <SettingsSessionsClient />
        </div>
      </section>
    </div>
  );
}
