import { displayTitle, type MediaItem } from "@/lib/chimpflix-types";
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
        <HeroTrailer ratingKey={item.ratingKey} delayMs={3000} />
      )}
      {/* Gradient overlays must NOT swallow pointer events — the trailer's
          mute button (and any future hero affordances) live underneath
          them in absolute-positioned space. */}
      <div className="pointer-events-none absolute inset-0 bg-linear-to-r from-black/95 via-black/55 to-transparent" />
      {/* Bottom fade dissolves the hero into the page background colour
          rather than to pure black, so the seam between the hero and the
          rails section below it is invisible. Tall and soft so the eye
          doesn't catch a hard band. */}
      <div className="pointer-events-none absolute inset-x-0 bottom-0 h-80 bg-linear-to-t from-(--color-background) via-(--color-background)/70 to-transparent" />

      <div className="relative z-10 flex h-full max-w-2xl flex-col justify-end px-4 pb-20 sm:px-8 sm:pb-28 md:px-12 md:pb-36">
        <h1 className="mb-4 text-3xl font-black uppercase leading-[0.95] tracking-tight drop-shadow-lg sm:mb-5 sm:text-5xl md:text-6xl">
          {displayTitle(item)}
        </h1>
        {item.summary && (
          <p className="mb-5 line-clamp-3 max-w-lg text-sm leading-relaxed text-white/90 drop-shadow sm:mb-7 sm:text-base">
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
