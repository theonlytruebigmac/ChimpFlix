import { readHiddenLibraries } from "@/lib/library-prefs";
import { sections } from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";
import { ModalRoot } from "@/components/ModalRoot";
import { SettingsClient } from "@/components/SettingsClient";
import { TopNav } from "@/components/TopNav";

export default async function SettingsPage() {
  const auth = await requireServerAuth();

  const [libs, hidden] = await Promise.all([
    sections(auth),
    readHiddenLibraries(),
  ]);

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <SettingsClient libraries={libs} initialHidden={[...hidden]} />
      <ModalRoot />
    </main>
  );
}
