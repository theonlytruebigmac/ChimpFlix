import {
  libraries as librariesApi,
  admin as adminApi,
  collections as collectionsApi,
} from "@/lib/chimpflix-api";
import { AdminLibrariesTabs } from "@/components/admin/AdminLibrariesTabs";
import { AdminLibrariesClient } from "@/components/admin/AdminLibrariesClient";
import { AdminCollectionsClient } from "@/components/admin/AdminCollectionsClient";
import { AdminOptimizedClient } from "@/components/admin/AdminOptimizedClient";
import { AdminLibrarySettingsClient } from "@/components/admin/AdminLibrarySettingsClient";

const TAB_IDS = ["libraries", "collections", "agents", "optimized", "defaults"];

/// Libraries — per-library settings, collections, the metadata-agent
/// catalogue, optimized versions, and global defaults, consolidated into one
/// tabbed page. (Folds the old /settings/admin/library subtree.)
export default async function AdminLibrariesPage({
  searchParams,
}: {
  searchParams: Promise<{ tab?: string }>;
}) {
  const sp = await searchParams;
  const initialTab = TAB_IDS.includes(sp?.tab ?? "")
    ? (sp.tab as string)
    : "libraries";

  const [libs, agentsRes, colls, versions, presets, settingsRes] =
    await Promise.all([
      librariesApi.list(),
      adminApi.agents.list(),
      collectionsApi.list({ include_auto: true }),
      adminApi.versions.list(),
      adminApi.transcoder.listPresets(),
      adminApi.settings.get(),
    ]);

  // The agent catalogue is read-only — render it as plain server markup and
  // pass it in as a slot rather than spinning up a client component.
  const agentsTable = (
    <div>
      <p className="mb-4 text-[12.5px] text-white/55">
        Available metadata providers. Set the priority order per library on the
        Libraries tab.
      </p>
      <div className="overflow-hidden rounded-lg border border-white/10">
        <table className="w-full text-sm">
          <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
            <tr>
              <th className="px-4 py-2">Agent</th>
              <th className="px-4 py-2">Supports</th>
              <th className="px-4 py-2">Status</th>
            </tr>
          </thead>
          <tbody>
            {agentsRes.agents.map((a) => (
              <tr key={a.name} className="border-t border-white/5">
                <td className="px-4 py-3">
                  <div className="font-medium">{a.display_name}</div>
                  <div className="font-mono text-xs text-white/40">{a.name}</div>
                </td>
                <td className="px-4 py-3 text-white/70">
                  {a.supported_kinds.join(", ")}
                </td>
                <td className="px-4 py-3">
                  {a.configured ? (
                    <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-300">
                      Configured
                    </span>
                  ) : (
                    <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                      Not configured
                    </span>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );

  return (
    <AdminLibrariesTabs
      initialTab={initialTab}
      libraries={
        <AdminLibrariesClient
          initialLibraries={libs.libraries}
          availableAgents={agentsRes.agents}
        />
      }
      collections={<AdminCollectionsClient initial={colls.collections} />}
      agents={agentsTable}
      optimized={
        <AdminOptimizedClient
          initial={versions.versions}
          presets={presets.presets}
        />
      }
      defaults={<AdminLibrarySettingsClient settings={settingsRes.settings} />}
    />
  );
}
