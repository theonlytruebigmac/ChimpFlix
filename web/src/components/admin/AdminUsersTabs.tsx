"use client";

import { useState, type ReactNode } from "react";
import { Tabs } from "@/components/admin/ui";

/// Consolidated Users page: People, Invites, Access, Groups, and Devices as
/// tabs (was the /settings/admin/users subtree + its layout tab bar). Slots
/// stay mounted (inactive hidden) so the People master-detail selection and
/// in-progress forms survive a tab switch.
export function AdminUsersTabs({
  initialTab,
  people,
  invites,
  access,
  groups,
  devices,
}: {
  initialTab: string;
  people: ReactNode;
  invites: ReactNode;
  access: ReactNode;
  groups: ReactNode;
  devices: ReactNode;
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
          { id: "people", label: "People" },
          { id: "invites", label: "Invites" },
          { id: "access", label: "Access" },
          { id: "groups", label: "Groups" },
          { id: "devices", label: "Devices" },
        ]}
        active={tab}
        onSelect={select}
      />
      <div className={tab === "people" ? "" : "hidden"}>{people}</div>
      <div className={tab === "invites" ? "" : "hidden"}>{invites}</div>
      <div className={tab === "access" ? "" : "hidden"}>{access}</div>
      <div className={tab === "groups" ? "" : "hidden"}>{groups}</div>
      <div className={tab === "devices" ? "" : "hidden"}>{devices}</div>
    </>
  );
}
