import Link from "next/link";
import { Suspense } from "react";
import { brandNameUpper } from "@/lib/env";
import { MobileNavTrigger, NavLinks } from "./NavLinks";
import { NotificationsBell } from "./NotificationsBell";
import { ProfileMenu } from "./ProfileMenu";
import { SearchBar } from "./SearchBar";
import { TopNavScrollEffect } from "./TopNavScrollEffect";

export function TopNav() {
  // Two background layers stacked. The gradient is visible at the top
  // (scroll = 0) and fades out as the user scrolls; the solid black
  // fades in to replace it. TopNavScrollEffect toggles a body class
  // that drives the transition. Server-rendered, no flash on hydrate.
  return (
    <header className="fixed inset-x-0 top-0 z-50">
      <TopNavScrollEffect />
      <div
        aria-hidden
        className="absolute inset-0 bg-linear-to-b from-black/85 via-black/55 to-transparent transition-opacity duration-200 [body.cf-nav-scrolled_&]:opacity-0"
      />
      <div
        aria-hidden
        className="absolute inset-0 bg-black opacity-0 transition-opacity duration-200 [body.cf-nav-scrolled_&]:opacity-100"
      />
      {/* Safe-area-inset padding so the row content doesn't go under a
          notch in landscape. `max()` keeps the design padding as the
          floor when the env() value is 0 (devices without a notch). */}
      <div className="relative flex items-center gap-3 py-3 pl-[max(1rem,env(safe-area-inset-left))] pr-[max(1rem,env(safe-area-inset-right))] pt-[max(0.75rem,env(safe-area-inset-top))] sm:gap-6 sm:pl-[max(1.5rem,env(safe-area-inset-left))] sm:pr-[max(1.5rem,env(safe-area-inset-right))] md:gap-10 md:py-4 md:pl-[max(3rem,env(safe-area-inset-left))] md:pr-[max(3rem,env(safe-area-inset-right))]">
        {/* Hamburger sits to the left of the logo on mobile and disappears
            at md+. The logo collapses one size on phones so the row fits
            with the bell + avatar on a 360px screen. */}
        <MobileNavTrigger />
        <Link
          href="/"
          className="select-none text-xl font-black tracking-tight text-(--color-accent) sm:text-2xl md:text-[1.65rem]"
        >
          {brandNameUpper()}
        </Link>
        <NavLinks />
        <div className="ml-auto flex items-center gap-3 text-sm sm:gap-5">
          {/*
            SearchBar uses useSearchParams() which requires a Suspense
            boundary so the prerender of static fallbacks (loading.tsx,
            /_not-found) can bail out cleanly. The fallback is empty —
            the client-side hydration paints the icon a frame later.
          */}
          <Suspense fallback={null}>
            <SearchBar />
          </Suspense>
          <NotificationsBell />
          <ProfileMenu />
        </div>
      </div>
    </header>
  );
}
