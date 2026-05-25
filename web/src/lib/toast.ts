/// Toast / inline-notice auto-dismiss durations.
///
/// Single source of truth so the UI doesn't drift into "this one
/// confirmation lingers 2.5s, that one lingers 3s, this admin one
/// disappears in 6s." Pick the closest semantic match below; only
/// invent a new constant if a context legitimately needs a different
/// reading time (e.g. transient resume-pill vs. quick "Saved" beep).
///
/// Picking a duration:
///   - SHORT: quick wins after a button click that the user obviously
///     just triggered ("Saved.", "Updated."). 2.5s is enough to
///     register without lingering.
///   - DEFAULT: most confirmations / inline feedback ("Added to your
///     list.", "Marked watched."). 3s is the Material baseline.
///   - PLAYER: in-player notices (subtitle changed, quality switch).
///     A bit longer because the user's attention is split between the
///     video and the toast; a 3s dismiss flashes past too fast.
///   - LONG: error messages / persistent banners ("Could not save.")
///     and bigger UX events (resume pill). 6s gives the user time to
///     read, click through, or dismiss.

export const TOAST_DISMISS_SHORT_MS = 2500;
export const TOAST_DISMISS_MS = 3000;
export const TOAST_DISMISS_PLAYER_MS = 4500;
export const TOAST_DISMISS_LONG_MS = 6000;
