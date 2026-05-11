import { Suspense } from "react";
import { Card } from "@/components/Card";
import { GenresDropdown } from "@/components/GenresDropdown";
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

const MOVIE_GENRES = [
  "Action",
  "Comedy",
  "Drama",
  "Thriller",
  "Sci-Fi",
  "Horror",
  "Romance",
  "Adventure",
  "Animation",
  "Documentary",
];

const SHOW_GENRES = [
  "Drama",
  "Comedy",
  "Animation",
  "Crime",
  "Sci-Fi",
  "Action",
  "Thriller",
  "Documentary",
  "Family",
  "Reality",
];

export default async function GenrePage({
  params,
  searchParams,
}: {
  params: Promise<{ name: string }>;
  searchParams: Promise<{ type?: string }>;
}) {
  const { name } = await params;
  const { type: typeParam } = await searchParams;
  const decoded = decodeURIComponent(name);
  const auth = await requireServerAuth();

  // Honor `?type=movie|show` so the Shows / Movies page genre dropdowns
  // can scope a genre browse to a single media type. Anything else
  // (including missing) means "both" — the legacy behavior the home /
  // hero genre rails depend on.
  const filterType: "movie" | "show" | null =
    typeParam === "movie" || typeParam === "show" ? typeParam : null;

  const dropdownGenres = filterType === "show" ? SHOW_GENRES : MOVIE_GENRES;
  const heading = filterType === "show"
    ? `${decoded} Shows`
    : filterType === "movie"
      ? `${decoded} Movies`
      : decoded;

  return (
    <main className="relative min-h-screen bg-black">
      <TopNav />
      <div className="px-12 pb-24 pt-28">
        <div className="mb-10 flex items-baseline gap-4">
          <h1 className="text-4xl font-bold tracking-tight">{heading}</h1>
          {filterType && (
            <GenresDropdown
              genres={dropdownGenres}
              type={filterType}
              current={decoded}
            />
          )}
        </div>
        <Suspense fallback={<GridSkeleton />}>
          <GenreGrid auth={auth} genre={decoded} filterType={filterType} />
        </Suspense>
      </div>
      <ModalRoot />
    </main>
  );
}

async function GenreGrid({
  auth,
  genre,
  filterType,
}: {
  auth: ServerAuth;
  genre: string;
  filterType: "movie" | "show" | null;
}) {
  const [allSections, hidden] = await Promise.all([
    sections(auth),
    readHiddenLibraries(),
  ]);
  const targets = allSections.filter((s) => {
    if (hidden.has(s.key)) return false;
    if (filterType) return s.type === filterType;
    return s.type === "movie" || s.type === "show";
  });

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
