"use client";

import { usePathname } from "next/navigation";
import { useLayoutEffect } from "react";

/// Force scroll-to-top whenever the admin pathname changes.
///
/// The default Next.js App Router scroll-to-top happens after the
/// new route's RSC has rendered, which means a scrolled operator
/// switching between admin pages briefly sees the new page rendered
/// at the previous scrollY before it snaps back up. That snap reads
/// as a "page jump". `useLayoutEffect` runs synchronously before
/// paint, so the scroll lands before the new content is visible.
///
/// Mounted once in the admin layout — invisible at runtime.
export function AdminScrollReset() {
  const pathname = usePathname();
  useLayoutEffect(() => {
    if (typeof window === "undefined") return;
    window.scrollTo({ top: 0, left: 0, behavior: "instant" });
  }, [pathname]);
  return null;
}
