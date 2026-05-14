"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const LINKS: ReadonlyArray<{
  href: string;
  label: string;
  match: (path: string) => boolean;
}> = [
  { href: "/", label: "Home", match: (p) => p === "/" },
  {
    href: "/shows",
    label: "Shows",
    match: (p) => p === "/shows" || p.startsWith("/shows/"),
  },
  {
    href: "/movies",
    label: "Movies",
    match: (p) => p === "/movies" || p.startsWith("/movies/"),
  },
  {
    href: "/new-popular",
    label: "New & Popular",
    match: (p) => p === "/new-popular" || p.startsWith("/new-popular/"),
  },
  {
    href: "/my-list",
    label: "My List",
    match: (p) => p === "/my-list" || p.startsWith("/my-list/"),
  },
];

export function NavLinks() {
  const pathname = usePathname();

  return (
    <nav className="hidden items-center gap-5 text-sm md:flex">
      {LINKS.map((link) => {
        const active = link.match(pathname ?? "");
        return (
          <Link
            key={link.href}
            href={link.href}
            aria-current={active ? "page" : undefined}
            className={
              active
                ? "font-medium text-white"
                : "text-white/70 transition-colors hover:text-white"
            }
          >
            {link.label}
          </Link>
        );
      })}
    </nav>
  );
}
