import Link from "next/link";
import { Suspense } from "react";
import { brandNameUpper } from "@/lib/env";
import { NavLinks } from "./NavLinks";
import { ProfileMenu } from "./ProfileMenu";
import { SearchBar } from "./SearchBar";

export function TopNav() {
  return (
    <header className="fixed inset-x-0 top-0 z-50 bg-linear-to-b from-black/85 via-black/55 to-transparent">
      <div className="flex items-center gap-10 px-12 py-4">
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
          <ProfileMenu />
        </div>
      </div>
    </header>
  );
}
