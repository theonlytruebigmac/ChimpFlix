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
  // pass it in as a slot rather than spinning up a client component. Styled
  // with the cf-* console design system to match the redesign mockup.
  const agentsTable = (
    <div>
      <div className="cf-banner cf-info">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <circle cx="12" cy="12" r="9" />
          <path d="M12 8v.5M12 11v5" />
        </svg>
        <div>
          This is the catalogue of providers and whether each has credentials.
          Set the <b>priority order per library</b> on the Libraries tab.
        </div>
      </div>
      <div className="cf-card" style={{ marginBottom: 0 }}>
        <table className="cf-table">
          <thead>
            <tr>
              <th>Agent</th>
              <th>Supports</th>
              <th>Credentials</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {agentsRes.agents.map((a) => (
              <tr key={a.name}>
                <td>
                  <b>{a.display_name}</b>
                  <div className="cf-mono cf-faint" style={{ fontSize: 11 }}>
                    {a.name}
                  </div>
                </td>
                <td className="cf-muted">{a.supported_kinds.join(" · ")}</td>
                <td className={`cf-mono${a.configured ? "" : " cf-faint"}`}>
                  {a.configured ? "configured" : "none"}
                </td>
                <td>
                  {a.configured ? (
                    <span className="cf-pill cf-ok">
                      <span className="cf-dot" />
                      Ready
                    </span>
                  ) : (
                    <span className="cf-pill cf-warn">
                      <span className="cf-dot" />
                      Needs key
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
      libraryCount={libs.libraries.length}
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
