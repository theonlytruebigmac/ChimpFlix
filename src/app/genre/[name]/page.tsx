import { Suspense } from "react";
import { Card } from "@/components/Card";
import { ModalRoot } from "@/components/ModalRoot";
import { CardSkeleton } from "@/components/Skeleton";
import { TopNav } from "@/components/TopNav";
import { readHiddenLibraries } from "@/lib/library-prefs";
import {
  sectionByGenre,
  sections,
  type Section,
  type ServerAuth,
} from "@/lib/plex-data";
import { requireServerAuth } from "@/lib/session";

export default async function GenrePage({
  params,
}: {
  params: Promise<{ name: string }>;
}) {
  const { name } = await params;
  const decoded = decodeURIComponent(name);
  const auth = await requireServerAuth();

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <div className="px-12 pb-24 pt-28">
        <h1 className="mb-10 text-4xl font-bold tracking-tight">{decoded}</h1>
        <Suspense fallback={<GridSkeleton />}>
          <GenreGrid auth={auth} genre={decoded} />
        </Suspense>
      </div>
      <ModalRoot />
    </main>
  );
}

async function GenreGrid({
  auth,
  genre,
}: {
  auth: ServerAuth;
  genre: string;
}) {
  const [allSections, hidden] = await Promise.all([
    sections(auth),
    readHiddenLibraries(),
  ]);
  const targets = allSections.filter(
    (s) =>
      (s.type === "movie" || s.type === "show") && !hidden.has(s.key),
  );

  return (
    <div className="space-y-14">
      {targets.map((sec) => (
        <Suspense
          key={sec.key}
          fallback={
            <SectionSkeleton title={targets.length > 1 ? sec.title : null} />
          }
        >
          <SectionForGenre
            auth={auth}
            section={sec}
            genre={genre}
            showHeading={targets.length > 1}
          />
        </Suspense>
      ))}
      {targets.length === 0 && (
        <p className="text-white/60">
          No {genre} titles in your Plex libraries.
        </p>
      )}
    </div>
  );
}

async function SectionForGenre({
  auth,
  section,
  genre,
  showHeading,
}: {
  auth: ServerAuth;
  section: Section;
  genre: string;
  showHeading: boolean;
}) {
  const items = await sectionByGenre(auth, section.key, genre, 60);
  if (items.length === 0) return null;
  return (
    <section className="zf-rise-in">
      {showHeading && (
        <h2 className="mb-5 text-xl font-semibold tracking-tight">
          {section.title}
        </h2>
      )}
      <ul className="flex flex-wrap gap-3">
        {items.map((item) => (
          <li key={item.ratingKey} className="flex-none">
            <Card item={item} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function GridSkeleton() {
  return (
    <div className="space-y-14">
      <SectionSkeleton title={null} />
    </div>
  );
}

function SectionSkeleton({ title }: { title: string | null }) {
  return (
    <section>
      {title && (
        <h2 className="mb-5 text-xl font-semibold tracking-tight">{title}</h2>
      )}
      <ul className="flex flex-wrap gap-3">
        {Array.from({ length: 12 }).map((_, i) => (
          <li key={i} className="flex-none">
            <CardSkeleton />
          </li>
        ))}
      </ul>
    </section>
  );
}
