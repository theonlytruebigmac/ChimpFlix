"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { ApiClientError, chimpflix } from "@/lib/api";
import type { ItemDetail } from "@/lib/types";
import { TopBar } from "@/components/TopBar";
import { formatRuntime } from "@/lib/format";

export default function ItemDetailPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const id = Number(params.id);
  const validId = Number.isFinite(id);
  const [item, setItem] = useState<ItemDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!validId) return;
    let cancelled = false;
    chimpflix.items
      .get(id)
      .then((d) => {
        if (!cancelled) setItem(d);
      })
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof ApiClientError && e.isUnauthorized) {
          router.replace("/login");
          return;
        }
        setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [id, validId, router]);

  if (!validId) {
    return (
      <main className="relative min-h-screen px-6 pt-32 sm:px-12">
        <TopBar />
        <p className="text-(--color-accent)">invalid item id</p>
      </main>
    );
  }
  if (error) {
    return (
      <main className="relative min-h-screen px-6 pt-32 sm:px-12">
        <TopBar />
        <p className="text-(--color-accent)">{error}</p>
      </main>
    );
  }
  if (!item) {
    return (
      <main className="relative min-h-screen px-6 pt-32 sm:px-12">
        <TopBar />
        <p className="text-white/55">Loading…</p>
      </main>
    );
  }

  const firstFile = item.files?.[0];

  return (
    <main className="relative min-h-screen pb-16">
      <TopBar />
      {item.backdrop_path && (
        <div className="absolute inset-x-0 top-0 h-[420px] overflow-hidden">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={item.backdrop_path}
            alt=""
            className="h-full w-full object-cover opacity-50"
          />
          <div className="absolute inset-0 bg-linear-to-b from-transparent via-black/40 to-black" />
        </div>
      )}
      <div className="relative px-6 pt-32 sm:px-12">
        <div className="grid gap-8 md:grid-cols-[200px_1fr] lg:grid-cols-[240px_1fr]">
          {item.poster_path && (
            // eslint-disable-next-line @next/next/no-img-element
            <img
              src={item.poster_path}
              alt={item.title}
              className="w-48 rounded shadow-2xl md:w-full"
            />
          )}
          <div>
            <h1 className="text-3xl font-bold tracking-tight sm:text-4xl">
              {item.title}
            </h1>
            <div className="mt-2 flex flex-wrap items-center gap-3 text-sm text-white/60">
              {item.year && <span>{item.year}</span>}
              {item.duration_ms && <span>{formatRuntime(item.duration_ms)}</span>}
              {item.rating_audience !== null && (
                <span>{item.rating_audience.toFixed(1)} ★</span>
              )}
            </div>
            {item.tagline && (
              <p className="mt-3 italic text-white/70">{item.tagline}</p>
            )}
            {item.summary && (
              <p className="mt-4 max-w-2xl leading-relaxed text-white/85">
                {item.summary}
              </p>
            )}
            {item.genres.length > 0 && (
              <p className="mt-3 text-sm text-white/55">
                {item.genres.join(" · ")}
              </p>
            )}

            <div className="mt-8 flex flex-wrap gap-3">
              {item.kind === "movie" && firstFile && (
                <Link
                  href={`/watch/${firstFile.id}`}
                  className="inline-flex items-center gap-2 rounded bg-white px-6 py-3 font-semibold text-black transition-colors hover:bg-white/85"
                >
                  <svg
                    width="20"
                    height="20"
                    viewBox="0 0 24 24"
                    fill="currentColor"
                    aria-hidden
                  >
                    <path d="M6 4l14 8-14 8V4z" />
                  </svg>
                  Play
                </Link>
              )}
              {item.kind === "show" && (
                <div className="rounded border border-white/20 px-4 py-3 text-sm text-white/60">
                  Season / episode picker coming in a follow-up. The seasons
                  are listed below.
                </div>
              )}
            </div>

            {item.kind === "show" &&
              item.seasons &&
              item.seasons.length > 0 && (
                <div className="mt-10">
                  <h2 className="text-sm font-semibold uppercase tracking-wider text-white/55">
                    Seasons
                  </h2>
                  <ul className="mt-2 space-y-1 text-sm text-white/80">
                    {item.seasons.map((s) => (
                      <li
                        key={s.id}
                        className="flex items-center gap-3 rounded border border-white/10 bg-(--color-surface) px-3 py-2"
                      >
                        <span className="font-mono text-xs text-white/55">
                          S{String(s.season_number).padStart(2, "0")}
                        </span>
                        <span>{s.title ?? `Season ${s.season_number}`}</span>
                        <span className="text-white/45">
                          {s.episode_count} episode{s.episode_count === 1 ? "" : "s"}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}

            {item.kind === "movie" && item.files && item.files.length > 0 && (
              <div className="mt-10 max-w-2xl">
                <h2 className="text-sm font-semibold uppercase tracking-wider text-white/55">
                  Files
                </h2>
                <ul className="mt-2 space-y-1 text-sm text-white/70">
                  {item.files.map((f) => (
                    <li
                      key={f.id}
                      className="flex flex-wrap items-center gap-3 rounded border border-white/10 bg-(--color-surface) px-3 py-2"
                    >
                      <span className="font-mono text-xs text-white/55">
                        #{f.id}
                      </span>
                      {f.container && <span>{f.container}</span>}
                      {f.width && f.height && (
                        <span>
                          {f.width}×{f.height}
                        </span>
                      )}
                      {f.duration_ms && (
                        <span className="text-white/55">
                          {formatRuntime(f.duration_ms)}
                        </span>
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}
