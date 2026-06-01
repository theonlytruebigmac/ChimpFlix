"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@/components/admin/ui";

/// Consolidated Tasks & jobs page: Overview, Queue, Activity, and the
/// pipeline Flow as tabs (was the /scheduled-tasks subtree). These views poll
/// on a timer, so — unlike the form consolidations — only the ACTIVE tab is
/// mounted (lazy), ensuring just one poller runs at a time. The per-kind
/// drill-in lives at /settings/admin/tasks/kind/[kind].
export function AdminTasksTabs({
  initialTab,
  overview,
  queue,
  activity,
  flow,
}: {
  initialTab: string;
  overview: ReactNode;
  queue: ReactNode;
  activity: ReactNode;
  flow: ReactNode;
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
          { id: "overview", label: "Overview" },
          { id: "queue", label: "Queue" },
          { id: "activity", label: "Activity" },
          { id: "flow", label: "Flow" },
        ]}
        active={tab}
        onSelect={select}
      />
      {tab === "overview" && overview}
      {tab === "queue" && queue}
      {tab === "activity" && activity}
      {tab === "flow" && flow}
    </>
  );
}
