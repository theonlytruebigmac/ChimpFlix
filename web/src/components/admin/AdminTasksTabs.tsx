"use client";

import { useState, type ReactNode } from "react";

/// Consolidated Tasks & jobs page: Overview, Queue, Activity, and the
/// pipeline Flow as tabs (was the /scheduled-tasks subtree). These views poll
/// on a timer, so — unlike the form consolidations — only the ACTIVE tab is
/// mounted (lazy), ensuring just one poller runs at a time. The per-kind
/// drill-in lives at /settings/admin/tasks/kind/[kind].
///
/// Styled with the console design system (`cf-tabs` / `cf-tab`) to match the
/// redesign mockup. There is intentionally no page title — the tabs are the
/// first thing on the page.
export function AdminTasksTabs({
  initialTab,
  initialQueueCount,
  overview,
  queue,
  activity,
  flow,
}: {
  initialTab: string;
  /// SSR snapshot of the queue badge (queued + running + failed + dead).
  /// Static after first paint — the live counts inside each tab refresh
  /// on their own pollers; the badge is just an at-a-glance hint.
  initialQueueCount: number;
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
  const tabs: { id: string; label: string; count?: number }[] = [
    { id: "overview", label: "Overview" },
    { id: "queue", label: "Queue", count: initialQueueCount },
    { id: "activity", label: "Activity" },
    { id: "flow", label: "Flow" },
  ];
  return (
    <div className="cf-content-inner cf-wide">
      <div className="cf-tabs" role="tablist" aria-label="Tasks & jobs">
        {tabs.map((t) => {
          const on = t.id === tab;
          return (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => select(t.id)}
              className={`cf-tab${on ? " cf-on" : ""}`}
            >
              {t.label}
              {t.count != null && t.count > 0 && (
                <span className="cf-pillcount">{t.count.toLocaleString()}</span>
              )}
            </button>
          );
        })}
      </div>
      {tab === "overview" && overview}
      {tab === "queue" && queue}
      {tab === "activity" && activity}
      {tab === "flow" && flow}
    </div>
  );
}
