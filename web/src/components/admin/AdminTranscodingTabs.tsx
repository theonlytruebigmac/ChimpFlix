"use client";

import { useState, type ComponentProps } from "react";
import { Tabs } from "@/components/admin/ui";
import { AdminTranscoderClient } from "./AdminTranscoderClient";
import { AdminPrerollClient } from "./AdminPrerollClient";

/// Consolidated Transcoding page: the engine settings form and the pre-roll
/// bumper config as two tabs (was /settings/admin/server/transcoder +
/// .../preroll). Both clients stay mounted and the inactive one is hidden so
/// half-edited forms survive a tab switch; the active tab mirrors to ?tab=.
export function AdminTranscodingTabs({
  initialTab,
  engine,
  preroll,
}: {
  initialTab: "engine" | "preroll";
  engine: ComponentProps<typeof AdminTranscoderClient>;
  preroll: ComponentProps<typeof AdminPrerollClient>;
}) {
  const [tab, setTab] = useState<string>(initialTab);
  const select = (id: string) => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };
  return (
    <>
      <Tabs
        tabs={[
          { id: "engine", label: "Engine" },
          { id: "preroll", label: "Pre-roll" },
        ]}
        active={tab}
        onSelect={select}
      />
      <div className={tab === "engine" ? "" : "hidden"}>
        <AdminTranscoderClient {...engine} />
      </div>
      <div className={tab === "preroll" ? "" : "hidden"}>
        <AdminPrerollClient {...preroll} />
      </div>
    </>
  );
}
