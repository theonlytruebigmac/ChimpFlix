"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@/components/admin/ui";

/// Consolidated Libraries page. Folds the old library subtree — per-library
/// CRUD, collections, the metadata-agent catalogue, optimized versions, and
/// global library defaults — into one page of tabs. Each tab's content is
/// rendered on the server and handed in as a slot (so the agent table can be
/// plain server markup while the others are their existing clients). Slots
/// stay mounted and the inactive ones are hidden, so master-detail selection
/// and half-edited forms survive a tab switch.
export function AdminLibrariesTabs({
  initialTab,
  libraries,
  collections,
  agents,
  optimized,
  defaults,
}: {
  initialTab: string;
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
  return (
    <>
      <Tabs
        tabs={[
          { id: "libraries", label: "Libraries" },
          { id: "collections", label: "Collections" },
          { id: "agents", label: "Agents" },
          { id: "optimized", label: "Optimized" },
          { id: "defaults", label: "Defaults" },
        ]}
        active={tab}
        onSelect={select}
      />
      <div className={tab === "libraries" ? "" : "hidden"}>{libraries}</div>
      <div className={tab === "collections" ? "" : "hidden"}>{collections}</div>
      <div className={tab === "agents" ? "" : "hidden"}>{agents}</div>
      <div className={tab === "optimized" ? "" : "hidden"}>{optimized}</div>
      <div className={tab === "defaults" ? "" : "hidden"}>{defaults}</div>
    </>
  );
}
