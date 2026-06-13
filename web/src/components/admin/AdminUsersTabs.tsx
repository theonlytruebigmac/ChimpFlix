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
              id={`tab-${t.id}`}
              type="button"
              role="tab"
              aria-selected={on}
              aria-controls={`panel-${t.id}`}
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
      {/* role="tabpanel" + aria-labelledby wire each panel back to its tab button.
          The HTML `hidden` attribute (not className) ensures AT skips inactive panels. */}
      <div id="panel-people" role="tabpanel" aria-labelledby="tab-people" hidden={tab !== "people"}>{people}</div>
      <div id="panel-access" role="tabpanel" aria-labelledby="tab-access" hidden={tab !== "access"}>{access}</div>
      <div id="panel-groups" role="tabpanel" aria-labelledby="tab-groups" hidden={tab !== "groups"}>{groups}</div>
      <div id="panel-devices" role="tabpanel" aria-labelledby="tab-devices" hidden={tab !== "devices"}>{devices}</div>
      <div id="panel-invites" role="tabpanel" aria-labelledby="tab-invites" hidden={tab !== "invites"}>{invites}</div>
    </>
  );
}
