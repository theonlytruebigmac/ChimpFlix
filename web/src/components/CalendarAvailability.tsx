/// Small "is this episode actually watchable yet?" indicator for the calendar
/// surfaces. The calendar shows episodes by *air date*, but an episode airing
/// "today" might not have a downloaded file until later in the day — so a green
/// "Available" (downloaded, playable) is distinguished from a muted "Not yet"
/// (placeholder / not downloaded). Driven by `CalendarEpisode.hasFile`.
export function CalendarAvailability({
  hasFile,
  className = "",
}: {
  hasFile: boolean;
  className?: string;
}) {
  return (
    <span
      className={
        "inline-flex items-center gap-1.5 text-[10.5px] font-semibold uppercase tracking-wide " +
        (hasFile ? "text-emerald-400" : "text-white/55") +
        (className ? " " + className : "")
      }
    >
      <span
        className={
          "h-1.5 w-1.5 rounded-full " +
          (hasFile ? "bg-emerald-400" : "border border-white/50 bg-transparent")
        }
        aria-hidden
      />
      {hasFile ? "Available" : "Not yet"}
    </span>
  );
}
