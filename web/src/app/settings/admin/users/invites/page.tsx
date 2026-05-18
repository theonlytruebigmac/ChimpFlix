import { SettingsInvitesClient } from "@/components/SettingsInvitesClient";

export default async function AdminInvitesPage() {
  return (
    <div>
      <header className="mb-8">
        <h1 className="text-3xl font-bold tracking-tight">Invites</h1>
        <p className="mt-1 text-sm text-white/60">
          Create signup codes for new viewers. Revoke from this page to
          take an unused invite out of circulation.
        </p>
      </header>
      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <SettingsInvitesClient />
      </section>
    </div>
  );
}
