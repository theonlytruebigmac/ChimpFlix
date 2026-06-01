"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@/components/admin/ui";

/// Consolidated Maintenance page: on-demand Cleanup, the library Health
/// snapshot, Backups & restore, and Bulk item operations as tabs (folds the
/// old maintenance overview + backup + bulk pages). Slots stay mounted so any
/// in-progress operation/result survives a tab switch.
export function AdminMaintenanceTabs({
  initialTab,
  cleanup,
  health,
  backups,
  bulk,
}: {
  initialTab: string;
  cleanup: ReactNode;
  health: ReactNode;
  backups: ReactNode;
  bulk: ReactNode;
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
          { id: "cleanup", label: "Cleanup" },
          { id: "health", label: "Library health" },
          { id: "backups", label: "Backups" },
          { id: "bulk", label: "Bulk ops" },
        ]}
        active={tab}
        onSelect={select}
      />
      <div className={tab === "cleanup" ? "" : "hidden"}>{cleanup}</div>
      <div className={tab === "health" ? "" : "hidden"}>{health}</div>
      <div className={tab === "backups" ? "" : "hidden"}>{backups}</div>
      <div className={tab === "bulk" ? "" : "hidden"}>{bulk}</div>
    </>
  );
}
