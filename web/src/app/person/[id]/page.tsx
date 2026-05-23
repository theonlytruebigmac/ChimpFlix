import { notFound } from "next/navigation";
import { PersonBrowseClient } from "@/components/PersonBrowseClient";
import { PortraitWithFallback } from "@/components/PortraitWithFallback";
import { ModalRoot } from "@/components/ModalRoot";
import { ChimpFlixApiError, people as peopleApi } from "@/lib/chimpflix-api";
import { requireUser } from "@/lib/chimpflix-server";

function formatLifespan(birthMs: number | null, deathMs: number | null): string | null {
  if (birthMs == null && deathMs == null) return null;
  const fmt = (ms: number) =>
    new Date(ms).toLocaleDateString(undefined, {
      year: "numeric",
      month: "long",
      day: "numeric",
    });
  if (birthMs != null && deathMs != null) {
    const ageMs = deathMs - birthMs;
    const years = Math.floor(ageMs / (365.25 * 24 * 3600 * 1000));
    return `${fmt(birthMs)} – ${fmt(deathMs)} (age ${years})`;
  }
  if (birthMs != null) return `Born ${fmt(birthMs)}`;
  return `Died ${fmt(deathMs as number)}`;
}

export default async function PersonPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  await requireUser(`/person/${id}`);
  const idNum = Number.parseInt(id, 10);
  if (!Number.isFinite(idNum) || idNum <= 0) notFound();

  let detail;
  try {
    detail = await peopleApi.get(idNum);
  } catch (e) {
    if (e instanceof ChimpFlixApiError && e.status === 404) notFound();
    throw e;
  }

  const lifespan = formatLifespan(detail.birthday, detail.deathday);

  return (
    <main className="relative min-h-screen bg-background pb-24 pt-24 sm:pt-28">
      <div className="px-4 sm:px-8 md:px-12">
        {/* Hero: portrait photo + bio side-by-side. Stacks on mobile.
            People photos are 2:3 portrait orientation (TMDB convention),
            not the 16:9 backdrops collections use — different layout
            entirely, no gradient banner. */}
        <section className="mb-10 flex flex-col gap-6 md:flex-row md:items-start">
          <div className="mx-auto h-64 w-44 shrink-0 overflow-hidden rounded-lg bg-white/5 shadow-lg md:mx-0 md:h-72 md:w-48">
            <PortraitWithFallback
              src={detail.photo_url}
              name={detail.name}
              className="zf-fade-in h-full w-full object-cover"
            />
          </div>
          <div className="flex-1">
            <div className="text-xs font-bold tracking-[0.35em] text-(--color-accent)">
              {detail.known_for_department?.toUpperCase() ?? "PERSON"}
            </div>
            <h1 className="mt-1 text-4xl font-bold tracking-tight md:text-5xl">
              {detail.name}
            </h1>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-white/65">
              {lifespan && <span>{lifespan}</span>}
              {detail.place_of_birth && (
                <>
                  {lifespan && <span aria-hidden>·</span>}
                  <span>{detail.place_of_birth}</span>
                </>
              )}
            </div>
            {detail.biography && (
              <p className="mt-4 max-w-prose text-sm leading-relaxed text-white/85">
                {detail.biography}
              </p>
            )}
            <div className="mt-4 text-sm text-white/55">
              {detail.items.length}{" "}
              {detail.items.length === 1 ? "title" : "titles"} in your library
            </div>
          </div>
        </section>

        <PersonBrowseClient items={detail.items} />
      </div>
      <ModalRoot />
    </main>
  );
}
