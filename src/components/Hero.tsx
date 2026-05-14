import { displayTitle, type MediaItem } from "@/lib/plex-data";
import { plexImage } from "@/lib/image";
import { HeroActions } from "./HeroActions";
import { HeroTrailer } from "./HeroTrailer";

export function Hero({ item }: { item: MediaItem }) {
  const backdrop = plexImage(item.art ?? item.thumb, 1920, 1080);
  // For Continue Watching episodes the hero surfaces a specific episode but
  // the modal should open the parent show (so the season/episode list shows
  // up). Mirror what Card.tsx does.
  const modalRatingKey =
    item.type === "episode" && item.grandparentRatingKey
      ? item.grandparentRatingKey
      : item.ratingKey;

  return (
    <section className="relative h-[70vh] min-h-120 w-full overflow-hidden">
      {backdrop && (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          src={backdrop}
          alt=""
          className="zf-fade-in absolute inset-0 h-full w-full object-cover"
        />
      )}
      {(item.type === "movie" || item.type === "show") && (
        <HeroTrailer
          type={item.type}
          title={item.title}
          year={item.year}
          delayMs={3000}
        />
      )}
      <div className="absolute inset-0 bg-linear-to-r from-black via-black/70 to-transparent" />
      <div className="absolute inset-x-0 bottom-0 h-56 bg-linear-to-t from-black via-black/60 to-transparent" />

      <div className="relative z-10 flex h-full max-w-2xl flex-col justify-end px-12 pb-36">
        <h1 className="mb-5 text-5xl font-black uppercase leading-[0.95] tracking-tight drop-shadow-lg sm:text-6xl">
          {displayTitle(item)}
        </h1>
        {item.summary && (
          <p className="mb-7 line-clamp-3 max-w-lg text-base leading-relaxed text-white/90 drop-shadow">
            {item.summary}
          </p>
        )}
        <HeroActions
          playRatingKey={item.ratingKey}
          modalRatingKey={modalRatingKey}
        />
      </div>
    </section>
  );
}
