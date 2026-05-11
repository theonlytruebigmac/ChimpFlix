import { redirect } from "next/navigation";
import { ServerPicker } from "@/components/ServerPicker";
import {
  getResources,
  pickConnectionUri,
  type PlexResource,
} from "@/lib/plex";
import { readActiveServer, readAdminToken } from "@/lib/session";

export default async function SelectServerPage({
  searchParams,
}: {
  searchParams: Promise<{ manual?: string }>;
}) {
  const masterToken = await readAdminToken();
  if (!masterToken) redirect("/login");

  // If the user already has a server picked, send them home. The Switch
  // Server flow re-renders this page after explicitly clearing the
  // cookie, so it won't bounce in that case.
  const existing = await readActiveServer();
  if (existing) redirect("/");

  // `?manual=1` means the user explicitly clicked "Pick a different
  // server" after a reachability failure — don't auto-pick the same
  // broken connection again, show them every option so they can force a
  // specific URL.
  const { manual } = await searchParams;
  const manualMode = manual === "1";

  let resources: PlexResource[] = [];
  let error: string | null = null;
  try {
    resources = await getResources(masterToken);
  } catch (e) {
    error = e instanceof Error ? e.message : String(e);
  }

  // If the user has exactly one accessible server, skip the picker UI
  // and select it automatically. This is the common case for a homelab
  // user with no shared servers.
  if (resources.length === 1 && !manualMode) {
    const r = resources[0];
    const uri = pickConnectionUri(r);
    if (uri) {
      // Render a tiny client component that POSTs and redirects, so we
      // don't need a server-side mutation here.
      return (
        <ServerPicker
          servers={[
            {
              id: r.id,
              name: r.name,
              owned: r.owned,
              presence: r.presence,
              productVersion: r.productVersion,
              platform: r.platform,
              defaultUri: uri,
              connections: r.connections.map((c) => ({
                uri: c.uri,
                local: c.local,
                relay: c.relay,
              })),
            },
          ]}
          autoSelect
          error={null}
        />
      );
    }
  }

  const servers = resources
    .map((r) => ({
      id: r.id,
      name: r.name,
      owned: r.owned,
      presence: r.presence,
      productVersion: r.productVersion,
      platform: r.platform,
      defaultUri: pickConnectionUri(r) ?? "",
      connections: r.connections.map((c) => ({
        uri: c.uri,
        local: c.local,
        relay: c.relay,
      })),
    }))
    .sort((a, b) => {
      if (a.owned !== b.owned) return a.owned ? -1 : 1;
      if (a.presence !== b.presence) return a.presence ? -1 : 1;
      return a.name.localeCompare(b.name);
    });

  return (
    <main className="relative min-h-screen bg-black text-white">
      <ServerPicker servers={servers} autoSelect={false} error={error} />
    </main>
  );
}
