"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { ApiClientError, chimpflix } from "@/lib/api";
import type { Item } from "@/lib/types";
import { TopBar } from "@/components/TopBar";
import { ItemCard } from "@/components/ItemCard";

export default function HomePage() {
  const router = useRouter();
  const [items, setItems] = useState<Item[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const page = await chimpflix.items.list({ page_size: 60 });
        if (cancelled) return;
        setItems(page.items);
      } catch (e) {
        if (cancelled) return;
        if (e instanceof ApiClientError && e.isUnauthorized) {
          router.replace("/login");
          return;
        }
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [router]);

  return (
    <main className="relative min-h-screen pb-16">
      <TopBar />
      <div className="px-6 pt-28 sm:px-12 sm:pt-32">
        <h1 className="mb-8 text-3xl font-bold tracking-tight">Library</h1>
        {loading && <p className="text-white/55">Loading…</p>}
        {error && (
          <div className="rounded border border-(--color-accent)/40 bg-(--color-accent)/10 p-4 text-sm">
            {error}
          </div>
        )}
        {!loading && !error && items.length === 0 && <EmptyLibrary />}
        {items.length > 0 && (
          <div className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-4 sm:grid-cols-[repeat(auto-fill,minmax(200px,1fr))] sm:gap-6">
            {items.map((item) => (
              <ItemCard key={item.id} item={item} />
            ))}
          </div>
        )}
      </div>
    </main>
  );
}

function EmptyLibrary() {
  return (
    <div className="mx-auto max-w-xl rounded-lg border border-white/10 bg-(--color-surface) p-8 text-center">
      <div className="text-lg font-semibold">No content yet</div>
      <p className="mt-2 text-sm text-white/60">
        Add a library and trigger a scan. The web UI doesn&apos;t expose
        library-management yet — use the API directly:
      </p>
      <pre className="mt-4 overflow-x-auto rounded border border-white/10 bg-black/50 p-3 text-left text-xs text-white/70">{`# create the library
curl -X POST http://localhost:8080/api/v1/libraries \\
  -H 'Content-Type: application/json' \\
  --cookie /tmp/cf.jar \\
  -d '{"name":"Movies","kind":"movies","paths":["/path/to/movies"]}'

# trigger a scan
curl -X POST http://localhost:8080/api/v1/libraries/1/scan \\
  --cookie /tmp/cf.jar`}</pre>
    </div>
  );
}
