import Link from "next/link";
import { Suspense } from "react";
import { brandNameUpper } from "@/lib/env";
import { NavLinks } from "./NavLinks";
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
      <div className="relative flex items-center gap-10 px-12 py-4">
        <Link
          href="/"
          className="select-none text-2xl font-black tracking-tight text-(--color-accent) sm:text-[1.65rem]"
        >
          {brandNameUpper()}
        </Link>
        <NavLinks />
        <div className="ml-auto flex items-center gap-5 text-sm">
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
