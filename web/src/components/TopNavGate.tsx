"use client";

import { usePathname } from "next/navigation";
import { TopNav } from "./TopNav";

/// Renders `<TopNav />` everywhere EXCEPT auth, player, settings, and
/// onboarding routes. Lives in the root layout so the nav (and its
/// SearchBar) survives client-side navigation between browse pages.
///
/// Previously each browse page rendered its own `<TopNav />`. That
/// meant any soft-navigation (e.g. SearchBar's debounce pushing to
/// `/search?q=...`) tore down the entire `<main>` tree on the
/// outgoing page, including the search input. The user lost focus
/// mid-type — surfaced as "I typed two letters and it kicked me out
/// of the box." Hoisting TopNav into the root layout keeps the
/// SearchBar mounted across route changes, so the URL updates but
/// the input you're typing in never disappears.
///
/// Routes that need a different shell (the immersive player, the
/// login screen, settings sidebar nav, the onboarding wizard) keep
/// rendering nothing here.
const HIDE_PREFIXES: readonly string[] = [
  "/login",
  "/signup",
  "/reset",
  "/watch",
  "/settings",
  "/onboarding",
];

export function TopNavGate() {
  const pathname = usePathname() ?? "";
  const hide = HIDE_PREFIXES.some(
    (p) => pathname === p || pathname.startsWith(`${p}/`),
  );
  if (hide) return null;
  return <TopNav />;
}
