import { libraries as librariesApi } from "@/lib/chimpflix-api";
import { SettingsHiddenLibrariesClient } from "@/components/SettingsHiddenLibrariesClient";
import { SettingsLibrariesClient } from "@/components/SettingsLibrariesClient";

export default async function LibrariesSettingsPage() {
  const libsResp = await librariesApi.list();
  return (
    <div className="divide-y divide-white/10">
      <Section title="Hidden libraries">
        <SettingsHiddenLibrariesClient libraries={libsResp.libraries} />
      </Section>
      <Section title="Libraries">
        <SettingsLibrariesClient initial={libsResp.libraries} />
      </Section>
    </div>
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
    <section className="grid gap-4 py-6 md:grid-cols-[12rem_1fr] md:gap-12">
      <h2 className="text-sm font-semibold uppercase tracking-wider text-white/50">
        {title}
      </h2>
      <div className="space-y-3">{children}</div>
    </section>
  );
}
