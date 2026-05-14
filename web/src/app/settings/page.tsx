import Link from "next/link";
import { TopNav } from "@/components/TopNav";
import {
  libraries as librariesApi,
  server as serverApi,
} from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";
import { SettingsHiddenLibrariesClient } from "@/components/SettingsHiddenLibrariesClient";
import { SettingsLibrariesClient } from "@/components/SettingsLibrariesClient";
import { SettingsPlayerClient } from "@/components/SettingsPlayerClient";
import { SettingsProfileClient } from "@/components/SettingsProfileClient";

export default async function SettingsPage() {
  const user = await requireUser("/settings");
  const [info, libsResp] = await Promise.all([
    serverApi.info(),
    librariesApi.list(),
  ]);

  return (
    <main className="relative min-h-screen bg-background">
      <TopNav />
      <div className="mx-auto max-w-4xl px-6 pb-24 pt-28">
        <h1 className="mb-8 text-4xl font-bold tracking-tight">Settings</h1>

        {user.role === "owner" && (
          <Link
            href="/admin"
            className="mb-8 flex items-center justify-between gap-4 rounded-lg border border-red-500/30 bg-red-500/5 px-5 py-4 transition-colors hover:bg-red-500/10"
          >
            <div>
              <div className="flex items-center gap-2 text-sm font-semibold">
                Server administration
                <span className="rounded bg-red-500/20 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-red-300">
                  Owner
                </span>
              </div>
              <div className="mt-0.5 text-xs text-white/60">
                Dashboard, libraries, transcoder, webhooks, scheduled
                tasks, devices, and more.
              </div>
            </div>
            <span className="text-sm text-white/40">→</span>
          </Link>
        )}

        <Section title="Server">
          <Row label="Version" value={`v${info.version}`} />
          <Row label="Libraries" value={`${info.library_counts.libraries}`} />
          <Row label="Movies" value={`${info.library_counts.movies}`} />
          <Row label="Shows" value={`${info.library_counts.shows}`} />
          <Row label="Episodes" value={`${info.library_counts.episodes}`} />
          <Row
            label="TMDB enrichment"
            value={info.tmdb_enabled ? "Enabled" : "Disabled"}
          />
        </Section>

        <Section title="Profile">
          <SettingsProfileClient initial={user} />
          <div className="mt-4 border-t border-white/10 pt-3">
            <Row label="Username" value={`@${user.username}`} />
            <Row
              label="Role"
              value={user.role === "owner" ? "Owner" : "User"}
            />
          </div>
        </Section>

        <Section title="Player">
          <SettingsPlayerClient />
        </Section>

        <Section title="Hidden libraries">
          <SettingsHiddenLibrariesClient libraries={libsResp.libraries} />
        </Section>

        <Section title="Libraries">
          <SettingsLibrariesClient initial={libsResp.libraries} />
        </Section>
      </div>
    </main>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-10 rounded-lg border border-white/10 bg-white/2 p-6">
      <h2 className="mb-4 text-xl font-semibold">{title}</h2>
      <div className="space-y-2">{children}</div>
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-white/5 py-2 text-sm last:border-b-0">
      <span className="text-white/60">{label}</span>
      <span className="text-white">{value}</span>
    </div>
  );
}
