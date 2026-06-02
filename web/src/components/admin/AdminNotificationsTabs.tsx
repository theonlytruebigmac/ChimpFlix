"use client";

import { useState, type ComponentProps } from "react";
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
  const [tab, setTab] = useState<"email" | "webhooks">(initialTab);
  const select = (id: "email" | "webhooks") => {
    setTab(id);
    if (typeof window !== "undefined")
      window.history.replaceState(null, "", `?tab=${id}`);
  };
  return (
    <div>
      <div className="cf-tabs">
        <button
          type="button"
          className={"cf-tab" + (tab === "email" ? " cf-on" : "")}
          aria-current={tab === "email" ? "page" : undefined}
          onClick={() => select("email")}
        >
          Email
        </button>
        <button
          type="button"
          className={"cf-tab" + (tab === "webhooks" ? " cf-on" : "")}
          aria-current={tab === "webhooks" ? "page" : undefined}
          onClick={() => select("webhooks")}
        >
          Webhooks
        </button>
      </div>
      <div className={tab === "email" ? "" : "hidden"}>
        <AdminEmailClient initial={email} />
      </div>
      <div className={tab === "webhooks" ? "" : "hidden"}>
        <AdminWebhooksClient initial={webhooks} />
      </div>
    </div>
  );
}
