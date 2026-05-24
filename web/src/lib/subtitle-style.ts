// Canonical subtitle-style model. Server-synced per account via
// `users.subtitle_*` columns (phase 89). Drives both browser-side
// `::cue` rendering of external WebVTT sidecars AND server-side ASS
// burn-in of embedded subtitle tracks, so what the user sees in the
// player gear matches what gets burned into transcodes.
//
// Pre-phase-89 this was split across two parallel localStorage layers
// (`cf_prefs_v1.subtitle*` and `chimpflix:subtitle:appearance`) with
// different field names + units. Both are now retired in favor of this
// single source of truth.
//
// The model carries presentation only — *which* subtitle track to
// display (language, embedded vs external) is orthogonal and lives
// in `User.default_subtitle_lang` + per-session selection state.
//
// All fields are required at this layer. `subtitleStyleFromUser`
// substitutes `DEFAULT_SUBTITLE_STYLE` for any null/missing column
// so consumers always get a concrete style to render against.
import type { User } from "./chimpflix-api";

export type SubtitleFontFamily = "default" | "sans" | "serif" | "mono";
export type SubtitleEdge = "none" | "outline" | "shadow";

export interface SubtitleStyle {
  /** CSS font-size in pixels. 24 is the sweet spot at 1080p; the
   * preset palette is 18 / 24 / 32 / 42 (S/M/L/XL). */
  fontSizePx: number;
  /** Foreground text color. `#RRGGBB`. */
  textColor: string;
  /** Background applied to the cue box. `rgba(...)` so alpha is part
   * of the palette — "transparent" is a valid pick. */
  backgroundColor: string;
  /** Generic font family. Kept generic on purpose so the docker image
   * doesn't need to bundle specific TTFs — libass + fontconfig + the
   * browser all resolve these via their built-in fallbacks. */
  fontFamily: SubtitleFontFamily;
  /** Edge style for glyphs. Outline reads safest over busy footage;
   * shadow is cleaner on dark cinema content; none lets the cue box
   * do the contrast work. */
  edge: SubtitleEdge;
  /** Bottom inset as a percentage of the player height. 5-15 is the
   * typical range; higher pushes subs closer to the middle of the
   * frame (useful when held in portrait on phones). */
  bottomInsetPct: number;
}

export const DEFAULT_SUBTITLE_STYLE: SubtitleStyle = {
  fontSizePx: 24,
  textColor: "#ffffff",
  backgroundColor: "rgba(0,0,0,0.55)",
  fontFamily: "default",
  edge: "outline",
  bottomInsetPct: 8,
};

const FONT_FAMILIES: readonly SubtitleFontFamily[] = [
  "default",
  "sans",
  "serif",
  "mono",
] as const;
const EDGES: readonly SubtitleEdge[] = ["none", "outline", "shadow"] as const;

/** Build a concrete `SubtitleStyle` from the user record. Any
 * null/missing column falls back to the corresponding default so the
 * caller always has every field to render. Passing `null` (e.g.
 * signed-out preview) returns the all-defaults shape. */
export function subtitleStyleFromUser(user: User | null): SubtitleStyle {
  if (!user) return DEFAULT_SUBTITLE_STYLE;
  const fam = user.subtitle_font_family;
  const edge = user.subtitle_edge;
  return {
    fontSizePx:
      typeof user.subtitle_font_size_px === "number"
        ? user.subtitle_font_size_px
        : DEFAULT_SUBTITLE_STYLE.fontSizePx,
    textColor: user.subtitle_text_color ?? DEFAULT_SUBTITLE_STYLE.textColor,
    backgroundColor:
      user.subtitle_background_color ??
      DEFAULT_SUBTITLE_STYLE.backgroundColor,
    fontFamily:
      fam !== null && (FONT_FAMILIES as readonly string[]).includes(fam)
        ? (fam as SubtitleFontFamily)
        : DEFAULT_SUBTITLE_STYLE.fontFamily,
    edge:
      edge !== null && (EDGES as readonly string[]).includes(edge)
        ? (edge as SubtitleEdge)
        : DEFAULT_SUBTITLE_STYLE.edge,
    bottomInsetPct:
      typeof user.subtitle_bottom_inset_pct === "number"
        ? user.subtitle_bottom_inset_pct
        : DEFAULT_SUBTITLE_STYLE.bottomInsetPct,
  };
}

/** Resolve the user's subtitle font preference to a CSS font-family
 * value for the `::cue` stylesheet. `null` means "let the browser
 * pick its caption default" (no rule emitted). */
export function cssFontFamilyForSubtitleStyle(
  fam: SubtitleFontFamily,
): string | null {
  switch (fam) {
    case "sans":
      return "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif";
    case "serif":
      return "Georgia, 'Times New Roman', serif";
    case "mono":
      return "ui-monospace, SFMono-Regular, Menlo, monospace";
    case "default":
    default:
      return null;
  }
}

/** Convert a SubtitleStyle into the ffmpeg `force_style` string used
 * when burning embedded text tracks into the transcode. Returns null
 * for the all-defaults case so we don't bloat every transcode command
 * with redundant style args.
 *
 * ASS color format is `&HAABBGGRR&` (alpha first, BGR not RGB, alpha
 * inverted from CSS where 0xFF = OPAQUE in ASS but TRANSPARENT in CSS).
 *
 * Position is intentionally not threaded through to ASS — vertical
 * margins in ASS depend on PlayResY which the transcoder doesn't
 * always set consistently, and the browser path handles position
 * correctly via cue letterbox math. Burn-in lands at the ASS default
 * (bottom-centred). */
export function subtitleStyleToAss(style: SubtitleStyle): string | null {
  if (
    style.fontSizePx === DEFAULT_SUBTITLE_STYLE.fontSizePx &&
    style.textColor === DEFAULT_SUBTITLE_STYLE.textColor &&
    style.backgroundColor === DEFAULT_SUBTITLE_STYLE.backgroundColor &&
    style.fontFamily === DEFAULT_SUBTITLE_STYLE.fontFamily &&
    style.edge === DEFAULT_SUBTITLE_STYLE.edge
  ) {
    return null;
  }
  const fontSize = Math.max(8, Math.min(128, style.fontSizePx));
  const primary = cssColorToAss(style.textColor) ?? "&H00FFFFFF&";
  const back = cssColorToAss(style.backgroundColor) ?? "&H66000000&";
  // BorderStyle=3 = opaque box behind text (uses BackColour).
  // BorderStyle=1 = outline+shadow only (uses OutlineColour + drop).
  // Map "none" + "shadow" + "outline" onto the closest ASS shape:
  //   outline → BorderStyle=1 with Outline=2, no shadow
  //   shadow  → BorderStyle=1 with Outline=0, Shadow=2
  //   none    → BorderStyle=1 with Outline=0, Shadow=0 (rely on bg)
  // BorderStyle=3 (opaque box) kicks in only when the user has a
  // visible background, so the box reads as the box and the edge
  // toggle controls the glyph styling on top.
  const wantsBox = !isTransparent(style.backgroundColor);
  const borderStyle = wantsBox ? 3 : 1;
  const outline = style.edge === "outline" ? 2 : 0;
  const shadow = style.edge === "shadow" ? 2 : 0;
  const parts: string[] = [
    `Fontsize=${fontSize}`,
    `PrimaryColour=${primary}`,
    `BackColour=${back}`,
    `BorderStyle=${borderStyle}`,
    `Outline=${outline}`,
    `Shadow=${shadow}`,
    `Alignment=2`,
  ];
  const fontname = assFontnameFor(style.fontFamily);
  if (fontname) parts.push(`Fontname=${fontname}`);
  return parts.join(",");
}

function assFontnameFor(fam: SubtitleFontFamily): string | null {
  switch (fam) {
    case "sans":
      return "Sans";
    case "serif":
      return "Serif";
    case "mono":
      return "Monospace";
    case "default":
    default:
      return null;
  }
}

/** Parse a CSS color string to ASS `&HAABBGGRR&`. Returns null on
 * shapes we can't recognise — caller falls back to its own default.
 * Handles `#RRGGBB`, `#RRGGBBAA`, `#RGB`, and `rgb()`/`rgba()`. */
function cssColorToAss(css: string): string | null {
  const trimmed = css.trim();
  let r = 0,
    g = 0,
    b = 0,
    a = 1;
  if (trimmed.startsWith("#")) {
    const hex = trimmed.slice(1);
    if (hex.length === 3) {
      r = parseInt(hex[0] + hex[0], 16);
      g = parseInt(hex[1] + hex[1], 16);
      b = parseInt(hex[2] + hex[2], 16);
    } else if (hex.length === 6) {
      r = parseInt(hex.slice(0, 2), 16);
      g = parseInt(hex.slice(2, 4), 16);
      b = parseInt(hex.slice(4, 6), 16);
    } else if (hex.length === 8) {
      r = parseInt(hex.slice(0, 2), 16);
      g = parseInt(hex.slice(2, 4), 16);
      b = parseInt(hex.slice(4, 6), 16);
      a = parseInt(hex.slice(6, 8), 16) / 255;
    } else {
      return null;
    }
  } else {
    const m = trimmed.match(
      /^rgba?\(\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*,\s*(\d+(?:\.\d+)?)\s*(?:,\s*(\d+(?:\.\d+)?))?\s*\)$/,
    );
    if (!m) return null;
    r = Math.round(parseFloat(m[1]));
    g = Math.round(parseFloat(m[2]));
    b = Math.round(parseFloat(m[3]));
    if (m[4] !== undefined) a = parseFloat(m[4]);
  }
  if ([r, g, b].some((v) => !Number.isFinite(v) || v < 0 || v > 255)) {
    return null;
  }
  const cssAlpha = Math.max(0, Math.min(1, a));
  const assAlpha = Math.round((1 - cssAlpha) * 255);
  const hex = (n: number) => n.toString(16).toUpperCase().padStart(2, "0");
  return `&H${hex(assAlpha)}${hex(b)}${hex(g)}${hex(r)}&`;
}

function isTransparent(css: string): boolean {
  const trimmed = css.trim();
  if (trimmed === "transparent") return true;
  const m = trimmed.match(/^rgba?\([^)]*,\s*([\d.]+)\s*\)$/);
  if (m) return parseFloat(m[1]) === 0;
  return false;
}
