"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@/components/admin/ui";

/// Consolidated Logs & audit page: the live application log tail and the
/// admin audit trail as tabs (was maintenance/logs + .../audit). The log tail
/// polls, so only the ACTIVE tab is mounted (lazy) — no background streaming
/// while you're reading the audit trail.
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
  return (
    <>
      <Tabs
        tabs={[
          { id: "logs", label: "Server logs" },
          { id: "audit", label: "Audit" },
        ]}
        active={tab}
        onSelect={select}
      />
      {tab === "logs" && logs}
      {tab === "audit" && audit}
    </>
  );
}
