import { SettingsPlayerClient } from "@/components/SettingsPlayerClient";

export default function PlayerSettingsPage() {
  return (
    <div className="divide-y divide-white/10">
      <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
          Player
        </h2>
        <div>
          <SettingsPlayerClient />
        </div>
      </section>
    </div>
  );
}
