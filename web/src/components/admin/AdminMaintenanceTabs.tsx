"use client";

import { useState, type ReactNode } from "react";

/// Consolidated Maintenance page: on-demand Cleanup, the library Health
/// snapshot, Backups & restore, and Bulk item operations as tabs (folds the
/// old maintenance overview + backup + bulk pages). Slots stay mounted so any
/// in-progress operation/result survives a tab switch.
///
/// Tab bar is rendered inline with the console `cf-tabs`/`cf-tab` design
/// system (matching docs/redesign/admin-maintenance.html) rather than the
/// shared Tailwind `Tabs` component.
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
  const tabs = [
    { id: "cleanup", label: "Cleanup" },
    { id: "health", label: "Library health" },
    { id: "backups", label: "Backups" },
    { id: "bulk", label: "Bulk ops" },
  ];
  return (
    <>
      <div className="cf-tabs">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            className={`cf-tab${tab === t.id ? " cf-on" : ""}`}
            aria-current={tab === t.id ? "page" : undefined}
            onClick={() => select(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className={tab === "cleanup" ? "" : "hidden"}>{cleanup}</div>
      <div className={tab === "health" ? "" : "hidden"}>{health}</div>
      <div className={tab === "backups" ? "" : "hidden"}>{backups}</div>
      <div className={tab === "bulk" ? "" : "hidden"}>{bulk}</div>
    </>
  );
}
