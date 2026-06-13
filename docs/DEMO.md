# ChimpFlix Demo

This guide walks you through spinning up an isolated demo instance that
showcases ChimpFlix's features without touching your production database,
Docker containers, or media library.

## What the demo gives you

- 20 items (15 movies, 3 TV shows, 2 anime series) with metadata, genres, and cast
- 45 episodes across 10 seasons
- Continue Watching rail (movies and episodes in progress)
- Recently Added content with the "New" badge
- Top 10 rails for movies and shows (populated via the trending cache)
- Genre rails: Action, Crime, Drama, Sci-Fi, Thriller, and more
- My List with 6 items pre-saved
- FTS search across all titles, summaries, and cast names
- Poster and backdrop images from [picsum.photos](https://picsum.photos)
  (deterministic placeholder photos — add a TMDB API key to refresh with
  real artwork)

## Requirements

- Docker with Compose v2 (`docker compose version`)
- `sqlite3` CLI
- `curl`

## Quick start

```bash
# From the repo root:
bash scripts/demo/setup.sh
# → Opens http://localhost:3001
# → Username: demo   Password: chimpflix2026
```

The script:
1. Builds the demo images (tagged `chimpflix-server:demo` / `chimpflix-web:demo`)
2. Starts the server on **port 8081** and waits for it to be healthy
3. Creates the owner account via the setup API
4. Stops the server, applies `scripts/demo/seed.sql` directly to the DB
5. Starts both services and prints the URL

> **No production impact.** The demo uses `./data-demo/` as its data directory
> and ports 8081/3001, completely separate from any production deployment on
> 8080/3000.

## Custom credentials

```bash
DEMO_USERNAME=admin DEMO_PASSWORD=secret bash scripts/demo/setup.sh
# or:
bash scripts/demo/setup.sh --username admin --password secret
```

## Stopping and resetting

```bash
# Stop the demo (data is preserved in ./data-demo/)
docker compose -p chimpflix-demo -f docker-compose.demo.yml down

# Full reset — start completely fresh
docker compose -p chimpflix-demo -f docker-compose.demo.yml down
rm -rf ./data-demo
bash scripts/demo/setup.sh
```

## Enabling real TMDB artwork

1. Create `./.env.demo.local` (already gitignored):
   ```
   TMDB_READ_TOKEN=your_tmdb_v4_read_access_token
   ```
   This is the only metadata key read from the environment. TVDB, OMDb, and
   MyAnimeList keys are entered in the running app under **Settings → Server →
   Credentials**, not via env vars.
2. Rebuild and restart:
   ```bash
   docker compose -p chimpflix-demo -f docker-compose.demo.yml up -d --build
   ```
3. Open the demo UI → **Settings → Libraries** → trigger a metadata refresh
   on each library.

## Pages to showcase

| Page | Path |
|------|------|
| Home (all rails) | `/` |
| Library browser | `/library/1` (Movies), `/library/2` (TV), `/library/3` (Anime) |
| Title detail (movie/show) | open any library → click a title (opens a detail modal, deep-links as `/?title={ratingKey}`) |
| Search | `/search` |
| My List | `/my-list` |
| Calendar | `/calendar` |
| Admin dashboard | `/settings/admin` |
| Library health | `/settings/admin/maintenance` (Library health section) |
| Job queue | `/settings/admin/tasks` |
| Users admin | `/settings/admin/users` |

## Manual seed (if setup.sh fails)

If the setup script can't reach the API (e.g., CHIMPFLIX_SETUP_TOKEN is
set), you can:

1. Open `http://localhost:3001` and complete the onboarding wizard manually.
2. Then apply the seed directly:
   ```bash
   docker compose -f docker-compose.demo.yml stop server
   sqlite3 ./data-demo/chimpflix.db < scripts/demo/seed.sql
   docker compose -f docker-compose.demo.yml start server
   ```

## What is NOT in the demo

- Actual video files (playback will return "no media file found")
- Trakt integration
- Push notifications
- Cast / Chromecast (requires a registered App ID)
- Email / SMTP

These features all work in a real deployment; the demo is metadata-and-UI only.
