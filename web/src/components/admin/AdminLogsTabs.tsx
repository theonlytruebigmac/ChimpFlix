"use client";

import { useState, type ReactNode } from "react";

/// Consolidated Logs & audit page: the live application log tail and the
/// admin audit trail as tabs (was maintenance/logs + .../audit). The log tail
/// polls, so only the ACTIVE tab is mounted (lazy) — no background streaming
/// while you're reading the audit trail.
///
/// Rendered in the console design language (cf-* classes from console.css):
/// the in-page `cf-tabs` bar, no page title — the page starts here.
export function AdminLogsTabs({
  initialTab,
  logs,
  audit,
}: {
  initialTab: string;
  logs: ReactNode;
  audit: ReactNode;
}) {
  const [tab, setTab] = useState(initialTab);
  const select = (id: string) => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };
  const TABS = [
    { id: "logs", label: "Server logs" },
    { id: "audit", label: "Audit" },
  ];
  return (
    <>
      <div className="cf-tabs" role="tablist" aria-label="Logs & audit">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            onClick={() => select(t.id)}
            className={`cf-tab${tab === t.id ? " cf-on" : ""}`}
          >
            {t.label}
          </button>
        ))}
      </div>
      {tab === "logs" && logs}
      {tab === "audit" && audit}
    </>
  );
}
