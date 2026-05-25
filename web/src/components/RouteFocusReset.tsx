"use client";

import { useEffect, useRef } from "react";
import { usePathname } from "next/navigation";

/// Reset keyboard focus to the page's <main> on every route change.
///
/// Next.js App Router doesn't move focus on client-side navigation —
/// so a keyboard user who tabs through nav links lands on a new page
/// with their focus still on the (now stale) nav item. Screen readers
/// also re-announce the old element's name on the next interaction,
/// which reads as "the page didn't change."
///
/// The fix is a small effect that finds `<main>` (or the document
/// body as a fallback) and calls `.focus({ preventScroll: true })`
/// after the pathname changes. We add `tabindex="-1"` defensively in
/// case the page's main element isn't natively focusable.
///
/// The first render (mount) intentionally does NOT shift focus —
/// Next.js fresh loads already focus where the user clicked into the
/// app (address bar, link from another page). Only navigations
/// *within* the SPA need the manual reset.
export function RouteFocusReset() {
  const pathname = usePathname();
  const mounted = useRef(false);
  useEffect(() => {
    if (!mounted.current) {
      mounted.current = true;
      return;
    }
    const main = document.querySelector("main");
    const target = main ?? document.body;
    if (!target.hasAttribute("tabindex")) {
      target.setAttribute("tabindex", "-1");
    }
    (target as HTMLElement).focus({ preventScroll: true });
  }, [pathname]);
  return null;
}
