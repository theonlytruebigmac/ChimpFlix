# Google Cast — custom receiver setup

ChimpFlix can cast to Chromecast / Google TV / Cast-built-in TVs. The
browser **sender** (the Cast button in the player) is already built; this
doc covers turning on the **custom receiver** that makes *HLS transcode*
casting work, not just direct-play.

## Why a custom receiver is required (not the stock one)

Google's stock receiver (`CC1AD845`) can play a single direct-play file
because the whole URL — token and all — is handed to it once and there
are no follow-up requests. But ChimpFlix serves transcoded video as
**HLS**, and the playlists reference their children with *bare relative
URLs*:

```
# master.m3u8 (served as .../master.m3u8?ct=<token>)
#EXT-X-STREAM-INF:...           v0/index.m3u8
#EXT-X-MEDIA:TYPE=SUBTITLES...  URI="sub/index.m3u8"

# v0/index.m3u8 (written by ffmpeg)
seg-000.ts
seg-001.ts
```

When the receiver resolves `v0/index.m3u8` against `master.m3u8?ct=<token>`,
**the query string is dropped**, so the variant/segment/caption requests
arrive with no token and the server returns `401`. The stock receiver has
no hook to fix this.

Our custom receiver ([`web/public/cast/receiver.html`](../web/public/cast/receiver.html))
captures the `ct` token from the initial media URL and re-appends it to
every same-origin manifest / segment / caption request, so HLS plays.

Auth/token internals: [`crates/server/src/auth/cast_token.rs`](../crates/server/src/auth/cast_token.rs).
Sender glue: [`web/src/lib/cast.ts`](../web/src/lib/cast.ts).

## Prerequisites

- **HTTPS.** The Cast Web Sender SDK only loads in a secure context, so
  ChimpFlix must be served over `https://` with `APP_PUBLIC_ORIGIN` set
  (e.g. `https://flix.example.com`). LAN-only HTTP deployments can't cast.
- The receiver is hosted at `https://<APP_PUBLIC_ORIGIN>/cast/receiver.html`
  (served straight out of `web/public/`, no app-router involvement). It
  must be the **same origin** as the app — the receiver fetches media from
  its own origin and the `ct` token only works there.

## One-time registration (do this in the Google Cast Developer Console)

1. Go to the [Google Cast SDK Developer Console](https://cast.google.com/publish)
   and pay the one-time **$5** developer registration fee.
2. **Add new application → Custom Receiver.**
   - Name: `ChimpFlix` (anything).
   - Receiver Application URL: `https://<your-origin>/cast/receiver.html`
   - You do **not** need Guest Mode or Google Cast for Audio.
3. Save. The console issues an **Application ID** (8-hex-ish, e.g. `1A2B3C4D`).
4. **Register your test devices:** in the console under *Cast Receiver →
   Add new device*, enter the **serial number** of each Chromecast /
   Google TV you'll test on (find it in the Google Home app → device →
   settings). Unpublished receivers only run on registered devices.
   Reboot the device after registering; allow ~15 min to propagate.
5. (Later, optional) **Publish** the receiver so it runs on any device
   without per-device registration. Publishing review is light for a
   media receiver; you can stay unpublished indefinitely for personal use.

## Point ChimpFlix at your receiver

`NEXT_PUBLIC_CAST_RECEIVER_APP_ID` is baked into the JS bundle at build
time (it is **not** read at runtime), so it must be set as a build arg and
the web image rebuilt.

**Docker Compose** — set it in your `.env` (compose reads it into the
`web` service build args automatically) and rebuild:

```sh
echo 'NEXT_PUBLIC_CAST_RECEIVER_APP_ID=1A2B3C4D' >> .env
docker compose build web
docker compose up -d web
```

**Local dev:**

```sh
NEXT_PUBLIC_CAST_RECEIVER_APP_ID=1A2B3C4D npm run build   # in web/
```

Leave the var unset to fall back to the stock receiver (direct-play
casting only).

## Verify on real hardware (Cast can't be tested in a simulator)

1. Open ChimpFlix over HTTPS in **desktop Chrome** (the most forgiving
   sender; Android Chrome and installed PWAs also work, iOS uses AirPlay
   instead). Start playing something, then click the **Cast** button and
   pick your device.
2. **Direct-play title** (a file your client can already play natively):
   should start on the TV. This works even with the stock receiver, so
   it's not a real test of the custom receiver — it just confirms the
   sender + token path.
3. **A title that forces transcode** (pick a non-native quality, burn-in
   a subtitle, or use a file with an unsupported codec): this is the real
   test. With the custom receiver it should play; if you still see a black
   screen / error, the token propagation or registration is off — see
   below.
4. **Remote debug the receiver:** on the same network, browse to the
   device's debug page at `http://<device-ip>:9222` from desktop Chrome
   (enable *Cast → Cast developer options* / the device's debug mode
   first) to get DevTools for the receiver. Check the Network tab:
   - `master.m3u8?ct=…` → 200, then
   - `v0/index.m3u8?ct=…`, `seg-000.ts?ct=…`, `sub/index.m3u8?ct=…`
     should **all still carry `?ct=`** and return 200. A `401` on a
     sub-request without `?ct=` means the request handlers aren't firing
     (wrong App ID → you got the stock receiver, or the device isn't
     registered so your custom receiver never launched).

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| Cast button never appears | Not HTTPS, non-Chromium browser, or Android **standalone PWA** (Cast IPC isn't proxied — open in the browser tab instead). |
| Direct-play casts, transcode 401s | Sender is still on the stock receiver — `NEXT_PUBLIC_CAST_RECEIVER_APP_ID` unset or image not rebuilt. |
| "Application not found" / device shows default screen | Wrong App ID, or the device isn't registered in the console (or hasn't propagated — reboot + wait). |
| Receiver loads but blank / CSP errors in `:9222` console | The receiver needs `ws:`/`wss:` in its CSP for the Cast control channel — this is already handled by the `/cast/*` CSP in `web/next.config.ts`; confirm your reverse proxy isn't injecting its own stricter CSP over `/cast/receiver.html`. |
| Playback dies after a long pause | The `ct` token has a 6-hour TTL (`cast_token::DEFAULT_TTL_MS`). A cast left paused past that will 401 on the next fetch; re-cast to mint a fresh token. |

## Known limitations / follow-ups

- **Watch progress while casting.** Local playback is paused during a
  cast, so resume position / scrobble isn't recorded from the receiver.
  The clean fix is sender-side: the browser keeps the Cast session handle
  and receives `MEDIA_STATUS` (currentTime) from the receiver, so it can
  scrobble using its existing cookie auth. Not wired up yet.
- **External (OpenSubtitles) subtitles** aren't surfaced to the receiver;
  only burned-in subs and the WebVTT sidecar from a transcode session are.
- **iOS** never uses Cast (WebKit doesn't expose the Web Sender SDK) — the
  Cast button falls through to **AirPlay** there, which reuses the local
  `<video>` element's cookies and needs none of this token machinery.
