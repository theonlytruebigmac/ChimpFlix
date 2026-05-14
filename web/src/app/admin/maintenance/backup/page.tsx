import { SettingsBackupClient } from "@/components/SettingsBackupClient";

export default async function AdminBackupPage() {
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Backup</h1>
        <p className="mt-1 text-sm text-white/60">
          On-demand SQLite snapshot via <code>VACUUM INTO</code>. The
          server writes a self-contained .db file and streams it back as a
          download.
        </p>
      </header>
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsBackupClient />
      </section>
    </div>
  );
}
