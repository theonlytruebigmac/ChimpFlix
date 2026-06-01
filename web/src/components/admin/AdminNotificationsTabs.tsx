"use client";

import { useState, type ComponentProps } from "react";
import { Tabs } from "@/components/admin/ui";
import { AdminEmailClient } from "./AdminEmailClient";
import { AdminWebhooksClient } from "./AdminWebhooksClient";

/// Consolidated Notifications page: SMTP email config and webhook endpoints
/// as two tabs (was /settings/admin/server/notifications + .../webhooks).
/// Both clients stay mounted (inactive hidden) so form state survives a tab
/// switch; the active tab mirrors to ?tab=.
export function AdminNotificationsTabs({
  initialTab,
  email,
  webhooks,
}: {
  initialTab: "email" | "webhooks";
  email: ComponentProps<typeof AdminEmailClient>["initial"];
  webhooks: ComponentProps<typeof AdminWebhooksClient>["initial"];
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
          { id: "email", label: "Email" },
          { id: "webhooks", label: "Webhooks" },
        ]}
        active={tab}
        onSelect={select}
      />
      <div className={tab === "email" ? "" : "hidden"}>
        <AdminEmailClient initial={email} />
      </div>
      <div className={tab === "webhooks" ? "" : "hidden"}>
        <AdminWebhooksClient initial={webhooks} />
      </div>
    </>
  );
}
