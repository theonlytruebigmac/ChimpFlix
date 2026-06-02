"use client";

import { useState, type ComponentProps } from "react";
import { AdminTranscoderClient } from "./AdminTranscoderClient";
import { AdminPresetsClient } from "./AdminPresetsClient";
import { AdminPrerollClient } from "./AdminPrerollClient";

/// Consolidated Transcoding page: the engine settings form, the quality
/// presets table, and the pre-roll bumper config as three tabs (was
/// /settings/admin/server/transcoder + .../preroll). All clients stay
/// mounted and the inactive ones are hidden so half-edited forms survive a
/// tab switch; the active tab mirrors to ?tab=.
type TabId = "engine" | "presets" | "preroll";

const TABS: ReadonlyArray<{ id: TabId; label: string }> = [
  { id: "engine", label: "Engine" },
  { id: "presets", label: "Presets" },
  { id: "preroll", label: "Pre-roll" },
];

export function AdminTranscodingTabs({
  initialTab,
  engine,
  presets,
  preroll,
}: {
  initialTab: TabId;
  engine: ComponentProps<typeof AdminTranscoderClient>;
  presets: ComponentProps<typeof AdminPresetsClient>;
  preroll: ComponentProps<typeof AdminPrerollClient>;
}) {
  const [tab, setTab] = useState<TabId>(initialTab);
  const select = (id: TabId) => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };
  return (
    <div>
      <div className="cf-tabs" role="tablist" aria-label="Transcoding">
        {TABS.map((t) => {
          const on = t.id === tab;
          return (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={on}
              className={"cf-tab" + (on ? " cf-on" : "")}
              onClick={() => select(t.id)}
            >
              {t.label}
            </button>
          );
        })}
      </div>
      <div className={tab === "engine" ? "" : "hidden"}>
        <AdminTranscoderClient {...engine} />
      </div>
      <div className={tab === "presets" ? "" : "hidden"}>
        <AdminPresetsClient {...presets} />
      </div>
      <div className={tab === "preroll" ? "" : "hidden"}>
        <AdminPrerollClient {...preroll} />
      </div>
    </div>
  );
}
