"use client";

import { useState, type ReactNode } from "react";

/// Consolidated Users & access page: Users (master-detail), Access matrix,
/// Access groups, Devices, and Invites as in-page tabs (was the
/// /settings/admin/users subtree + its layout tab bar). Slots stay mounted
/// (inactive hidden) so the Users master-detail selection and in-progress
/// forms survive a tab switch.
///
/// Styled with the console design system (`cf-tabs` / `cf-tab`). There is
/// intentionally no page title — the tab bar is the first thing on the page.
export function AdminUsersTabs({
  initialTab,
  usersCount,
  invitesCount,
  people,
  access,
  groups,
  devices,
  invites,
}: {
  initialTab: string;
  usersCount?: number;
  invitesCount?: number;
  people: ReactNode;
  access: ReactNode;
  groups: ReactNode;
  devices: ReactNode;
  invites: ReactNode;
}) {
  const [tab, setTab] = useState(initialTab);
  const select = (id: string) => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };
  const tabs: { id: string; label: string; count?: number }[] = [
    { id: "people", label: "Users", count: usersCount },
    { id: "access", label: "Access matrix" },
    { id: "groups", label: "Access groups" },
    { id: "devices", label: "Devices" },
    { id: "invites", label: "Invites", count: invitesCount },
  ];
  return (
    <>
      <div className="cf-tabs" role="tablist" aria-label="Users & access">
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
      <div className={tab === "people" ? "" : "hidden"}>{people}</div>
      <div className={tab === "access" ? "" : "hidden"}>{access}</div>
      <div className={tab === "groups" ? "" : "hidden"}>{groups}</div>
      <div className={tab === "devices" ? "" : "hidden"}>{devices}</div>
      <div className={tab === "invites" ? "" : "hidden"}>{invites}</div>
    </>
  );
}
