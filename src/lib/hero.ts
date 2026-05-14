// Deterministic hero rotation. We pick from the candidate pool by a time
// bucket rather than Math.random() so the same hero stays put across page
// navigations within the bucket. Without this, the user got a different
// billboard every time they hit Home / Movies / Shows — which feels jittery
// and triggers a fresh trailer fetch every navigation.
//
// Bucket length picks the rotation cadence. 15 minutes feels close to
// Netflix's "varies between visits but doesn't whip around mid-session".

const BUCKET_MS = 15 * 60_000;

export function pickHeroIndex<T>(pool: T[], salt = ""): number {
  if (pool.length === 0) return 0;
  const bucket = Math.floor(Date.now() / BUCKET_MS);
  // Salt by section name so /movies and /shows cycle independently — picking
  // the same bucket would otherwise yield the same index, which means index 0
  // every time across pools of the same length.
  let h = 2166136261;
  const s = `${salt}:${bucket}`;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % pool.length;
}
