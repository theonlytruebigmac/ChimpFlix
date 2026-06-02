"use client";

import { useState, type ReactNode } from "react";

/// Consolidated Libraries page. Folds the old library subtree — per-library
/// CRUD, collections, the metadata-agent catalogue, optimized versions, and
/// global library defaults — into one page of tabs. Each tab's content is
/// rendered on the server and handed in as a slot (so the agent table can be
/// plain server markup while the others are their existing clients). Slots
/// stay mounted and the inactive ones are hidden, so master-detail selection
/// and half-edited forms survive a tab switch.
export function AdminLibrariesTabs({
  initialTab,
  libraryCount,
  libraries,
  collections,
  agents,
  optimized,
  defaults,
}: {
  initialTab: string;
  libraryCount: number;
  libraries: ReactNode;
  collections: ReactNode;
  agents: ReactNode;
  optimized: ReactNode;
  defaults: ReactNode;
}) {
  const [tab, setTab] = useState(initialTab);
  const select = (id: string) => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };

  const TABS: { id: string; label: string; count?: number }[] = [
    { id: "libraries", label: "Libraries", count: libraryCount },
    { id: "collections", label: "Collections" },
    { id: "agents", label: "Metadata agents" },
    { id: "optimized", label: "Optimized versions" },
    { id: "defaults", label: "Defaults" },
  ];

  return (
    <>
      <div className="cf-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            className={`cf-tab${tab === t.id ? " cf-on" : ""}`}
            onClick={() => select(t.id)}
          >
            {t.label}
            {t.count != null && (
              <span className="cf-pillcount">{t.count}</span>
            )}
          </button>
        ))}
      </div>
      <div className={tab === "libraries" ? "" : "hidden"}>{libraries}</div>
      <div className={tab === "collections" ? "" : "hidden"}>{collections}</div>
      <div className={tab === "agents" ? "" : "hidden"}>{agents}</div>
      <div className={tab === "optimized" ? "" : "hidden"}>{optimized}</div>
      <div className={tab === "defaults" ? "" : "hidden"}>{defaults}</div>
    </>
  );
}
