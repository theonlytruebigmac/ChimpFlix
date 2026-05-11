"use client";

import Link from "next/link";
import type { Item } from "@/lib/types";
import { formatRuntime } from "@/lib/format";

export function ItemCard({ item }: { item: Item }) {
  const progress =
    item.play_state?.position_ms && item.play_state?.duration_ms
      ? Math.min(
          100,
          (item.play_state.position_ms / item.play_state.duration_ms) * 100,
        )
      : null;

  return (
    <Link
      href={`/items/${item.id}`}
      className="group relative block w-full flex-none"
    >
      <div className="origin-center transition-transform duration-200 ease-out group-hover:scale-[1.03]">
        <div className="overflow-hidden rounded-md bg-(--color-surface) shadow-md group-hover:shadow-2xl">
          <div className="relative aspect-[2/3] bg-black">
            {item.poster_path ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={item.poster_path}
                alt={item.title}
                loading="lazy"
                decoding="async"
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center p-3 text-center text-sm text-white/40">
                {item.title}
              </div>
            )}
            <div className="pointer-events-none absolute inset-x-0 bottom-0 bg-linear-to-t from-black/95 via-black/55 to-transparent px-3 pt-14 pb-3">
              <div className="line-clamp-2 text-sm font-semibold leading-tight drop-shadow-lg">
                {item.title}
              </div>
              <div className="mt-1 flex items-center gap-2 text-xs text-white/65">
                {item.year && <span>{item.year}</span>}
                {item.kind === "movie" && item.duration_ms && (
                  <span>{formatRuntime(item.duration_ms)}</span>
                )}
                {item.kind === "show" && (
                  <span className="text-white/60">Series</span>
                )}
              </div>
            </div>
            {progress !== null && (
              <div className="absolute inset-x-0 bottom-0 h-1 bg-white/25">
                <div
                  className="h-full bg-(--color-accent)"
                  style={{ width: `${progress}%` }}
                />
              </div>
            )}
          </div>
        </div>
      </div>
    </Link>
  );
}
