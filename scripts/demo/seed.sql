-- ChimpFlix demo seed data.
--
-- Run AFTER completing first-run setup (owner account created) and
-- AFTER stopping the server (to avoid WAL conflicts):
--
--   sqlite3 ./data-demo/chimpflix.db < scripts/demo/seed.sql
--
-- Idempotent — uses INSERT OR IGNORE / INSERT OR REPLACE throughout.
-- All poster/backdrop images use picsum.photos deterministic seeds so
-- they render as real photographs without a TMDB API key.
-- Run a metadata refresh from Settings → Libraries after adding a TMDB
-- key to replace placeholders with actual artwork.

PRAGMA foreign_keys = ON;

-- ─────────────────────────────────────────────────────────────────────────────
-- Helpers: local time expression shortcuts used throughout
-- ─────────────────────────────────────────────────────────────────────────────
-- All *_at columns are Unix epoch milliseconds (INTEGER).

-- ─────────────────────────────────────────────────────────────────────────────
-- Mark setup complete so the onboarding wizard never shows again
-- ─────────────────────────────────────────────────────────────────────────────
UPDATE server_settings
SET setup_completed = 1,
    server_name     = 'ChimpFlix Demo',
    -- Point public_url at the web frontend port so the browser's
    -- Origin header (http://localhost:3001) passes CSRF checks.
    public_url      = 'http://localhost:3001',
    cors_origins    = '["http://localhost:3001"]',
    updated_at      = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE id = 1;

-- Disable BOTH automatic-scan triggers. The demo's /media/* paths are empty
-- (no real video files), so ANY scan would soft-delete every seeded
-- media_files row — and items with no active file are hidden from browse by
-- the has_active_files_clause() filter, making the whole library vanish.
--   periodic_scan_enabled = 0 → the hourly scheduler scan never runs
--   scan_automatically    = 0 → filesystem-watch events never enqueue a scan
-- (The per-library `scan_interval_s` column is NOT consulted by the scheduler
-- — the effective cadence is the global `periodic_scan_frequency`, gated on
-- `periodic_scan_enabled` — so disabling that setting is the correct knob.
-- A manual scan from Settings → Libraries would still clear the files; that's
-- an explicit operator action, not something that happens on its own.)
UPDATE server_settings
SET periodic_scan_enabled = 0,
    scan_automatically    = 0
WHERE id = 1;

-- ─────────────────────────────────────────────────────────────────────────────
-- Libraries
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO libraries (id, name, kind, scan_interval_s, created_at, updated_at)
VALUES
    (1, 'Movies',   'movies', 3600, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),
    (2, 'TV Shows', 'shows',  3600, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),
    (3, 'Anime',    'shows',  3600, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

INSERT OR IGNORE INTO library_paths (library_id, path)
VALUES
    (1, '/media/movies'),
    (2, '/media/shows'),
    (3, '/media/anime');

-- Grant owner (user_id=1) access to all three libraries
INSERT OR IGNORE INTO library_access (user_id, library_id) VALUES (1, 1), (1, 2), (1, 3);

-- ─────────────────────────────────────────────────────────────────────────────
-- Genres
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO genres (id, name) VALUES
    (1,  'Action'),
    (2,  'Adventure'),
    (3,  'Animation'),
    (4,  'Comedy'),
    (5,  'Crime'),
    (6,  'Drama'),
    (7,  'Fantasy'),
    (8,  'History'),
    (9,  'Horror'),
    (10, 'Mystery'),
    (11, 'Romance'),
    (12, 'Science Fiction'),
    (13, 'Thriller'),
    (14, 'War'),
    (15, 'Western');

-- ─────────────────────────────────────────────────────────────────────────────
-- Movies (library 1, items 1-15)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO items
    (id, library_id, kind, title, sort_title, original_title, summary, tagline, year,
     rating_age, rating_audience, duration_ms, tmdb_id, imdb_id, added_at, updated_at)
VALUES
(1, 1, 'movie', 'The Dark Knight', 'Dark Knight, The', NULL,
 'When the menace known as the Joker wreaks havoc and chaos on the people of Gotham, Batman must accept one of the greatest psychological and physical tests of his ability to fight injustice.',
 'Why So Serious?',
 2008, 'PG-13', 9.0, 9156000, 155, 'tt0468569',
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(2, 1, 'movie', 'Inception', 'Inception', NULL,
 'A thief who steals corporate secrets through the use of dream-sharing technology is given the inverse task of planting an idea into the mind of a C.E.O.',
 'Your mind is the scene of the crime.',
 2010, 'PG-13', 8.8, 8880000, 27205, 'tt1375666',
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(3, 1, 'movie', 'The Shawshank Redemption', 'Shawshank Redemption, The', NULL,
 'Two imprisoned men bond over a number of years, finding solace and eventual redemption through acts of common decency.',
 'Fear can hold you prisoner. Hope can set you free.',
 1994, 'R', 9.3, 8520000, 278, 'tt0111161',
 (strftime('%s','now') - 15552000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(4, 1, 'movie', 'The Matrix', 'Matrix, The', NULL,
 'A computer hacker learns from mysterious rebels about the true nature of his reality and his role in the war against its controllers.',
 'Welcome to the Real World.',
 1999, 'R', 8.7, 8160000, 603, 'tt0133093',
 (strftime('%s','now') - 12960000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(5, 1, 'movie', 'Interstellar', 'Interstellar', NULL,
 'A team of explorers travel through a wormhole in space in an attempt to ensure humanity''s survival.',
 'Mankind was born on Earth. It was never meant to die here.',
 2014, 'PG-13', 8.6, 10140000, 157336, 'tt0816692',
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(6, 1, 'movie', 'Pulp Fiction', 'Pulp Fiction', NULL,
 'The lives of two mob hitmen, a boxer, a gangster and his wife, and a pair of diner bandits intertwine in four tales of violence and redemption.',
 'Just because you are a character doesn''t mean you have character.',
 1994, 'R', 8.9, 9360000, 680, 'tt0110912',
 (strftime('%s','now') - 15552000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(7, 1, 'movie', 'The Godfather', 'Godfather, The', NULL,
 'The aging patriarch of an organized crime dynasty transfers control of his clandestine empire to his reluctant son.',
 'An offer you can''t refuse.',
 1972, 'R', 9.2, 10560000, 238, 'tt0068646',
 (strftime('%s','now') - 15552000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(8, 1, 'movie', 'Fight Club', 'Fight Club', NULL,
 'An insomniac office worker and a devil-may-care soap maker form an underground fight club that evolves into an anarchist organization.',
 'Mischief. Mayhem. Soap.',
 1999, 'R', 8.8, 8400000, 550, 'tt0137523',
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(9, 1, 'movie', 'Goodfellas', 'Goodfellas', NULL,
 'The story of Henry Hill and his life in the mob, covering his brutal rise to the top of the criminal world.',
 'Three Decades of Life in the Mafia.',
 1990, 'R', 8.7, 8820000, 769, 'tt0099685',
 (strftime('%s','now') - 12960000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(10, 1, 'movie', 'Dune: Part One', 'Dune: Part One', NULL,
 'A noble family becomes embroiled in a war for control over the galaxy''s most valuable asset while its heir becomes troubled by visions of a dark future.',
 'It Begins.',
 2021, 'PG-13', 8.1, 9360000, 438631, 'tt1160419',
 (strftime('%s','now') - 2592000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(11, 1, 'movie', 'Avengers: Endgame', 'Avengers: Endgame', NULL,
 'After the devastating events of Infinity War, the Avengers assemble once more in order to reverse Thanos'' actions and restore balance to the universe.',
 'Part of the journey is the end.',
 2019, 'PG-13', 8.4, 10860000, 299534, 'tt4154796',
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(12, 1, 'movie', 'Top Gun: Maverick', 'Top Gun: Maverick', NULL,
 'After thirty years, Maverick is still pushing the envelope as a top naval aviator, but must confront ghosts of his past when he leads TOP GUN''s elite graduates on a specialized mission.',
 'Feel the need.',
 2022, 'PG-13', 8.3, 8160000, 361743, 'tt1745960',
 (strftime('%s','now') - 1209600) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(13, 1, 'movie', 'Oppenheimer', 'Oppenheimer', NULL,
 'The story of American scientist J. Robert Oppenheimer and his role in the development of the atomic bomb during World War II.',
 'The world forever changes.',
 2023, 'R', 8.6, 11040000, 872585, 'tt15398776',
 (strftime('%s','now') - 604800) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(14, 1, 'movie', 'Everything Everywhere All at Once', 'Everything Everywhere All at Once', NULL,
 'An aging Chinese immigrant is swept up in an insane adventure in which she alone can save the world by exploring other universes connecting with the lives she could have led.',
 'The universe is so much bigger than you realize.',
 2022, 'R', 8.1, 7920000, 545611, 'tt6710474',
 (strftime('%s','now') - 2592000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(15, 1, 'movie', 'The Silence of the Lambs', 'Silence of the Lambs, The', NULL,
 'A young F.B.I. cadet must receive the help of an incarcerated and manipulative cannibal killer to help catch another serial killer.',
 'To enter the mind of a killer she must challenge the mind of a madman.',
 1991, 'R', 8.6, 7380000, 274, 'tt0102926',
 (strftime('%s','now') - 12960000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ─────────────────────────────────────────────────────────────────────────────
-- TV Shows (library 2, items 16-18)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO items
    (id, library_id, kind, title, sort_title, summary, tagline, year,
     rating_age, rating_audience, tmdb_id, imdb_id, added_at, updated_at)
VALUES
(16, 2, 'show', 'Breaking Bad', 'Breaking Bad',
 'A high school chemistry teacher dying of cancer teams with a former student to secure a future for his family by manufacturing and selling methamphetamine.',
 'All bad things must come to an end.',
 2008, 'TV-MA', 9.5, 1396, 'tt0903747',
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(17, 2, 'show', 'Stranger Things', 'Stranger Things',
 'When a young boy vanishes, a small town uncovers a mystery involving secret experiments, terrifying supernatural forces, and one strange little girl.',
 'Every ending has a beginning.',
 2016, 'TV-14', 8.7, 66732, 'tt4574334',
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(18, 2, 'show', 'The Last of Us', 'Last of Us, The',
 'Joel, a hardened survivor, is hired to smuggle Ellie, a 14-year-old girl, out of an oppressive quarantine zone. What starts as a small job soon becomes a brutal, heartbreaking journey.',
 'When you''re lost in the darkness, look for the light.',
 2023, 'TV-MA', 8.8, 100088, 'tt3581920',
 (strftime('%s','now') - 259200) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ─────────────────────────────────────────────────────────────────────────────
-- Anime (library 3, items 19-20)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO items
    (id, library_id, kind, title, sort_title, summary, tagline, year,
     rating_age, rating_audience, tmdb_id, tvdb_id, added_at, updated_at)
VALUES
(19, 3, 'show', 'Attack on Titan', 'Attack on Titan',
 'Centuries ago, mankind was slaughtered to near extinction by monstrous humanoid creatures called titans. Surviving humans now live in fear behind enormous walls.',
 'The last bastion of humanity.',
 2013, 'TV-MA', 9.1, 44217, 267440,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(20, 3, 'show', 'Demon Slayer: Kimetsu no Yaiba', 'Demon Slayer: Kimetsu no Yaiba',
 'A family is attacked by demons and only two members survive — Tanjiro and his sister Nezuko, who is turning into a demon. Tanjiro sets out to become a demon slayer to avenge his family and cure his sister.',
 'Become the blade that destroys evil.',
 2019, 'TV-MA', 8.7, 85937, 362680,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ─────────────────────────────────────────────────────────────────────────────
-- Item genres
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO item_genres (item_id, genre_id) VALUES
-- The Dark Knight: Action, Crime, Drama
(1, 1), (1, 5), (1, 6),
-- Inception: Action, Adventure, Science Fiction
(2, 1), (2, 2), (2, 12),
-- The Shawshank Redemption: Drama
(3, 6),
-- The Matrix: Action, Science Fiction
(4, 1), (4, 12),
-- Interstellar: Adventure, Drama, Science Fiction
(5, 2), (5, 6), (5, 12),
-- Pulp Fiction: Crime, Drama, Thriller
(6, 5), (6, 6), (6, 13),
-- The Godfather: Crime, Drama
(7, 5), (7, 6),
-- Fight Club: Drama, Thriller, Mystery
(8, 6), (8, 13), (8, 10),
-- Goodfellas: Crime, Drama
(9, 5), (9, 6),
-- Dune Part One: Action, Adventure, Science Fiction
(10, 1), (10, 2), (10, 12),
-- Avengers Endgame: Action, Adventure, Science Fiction
(11, 1), (11, 2), (11, 12),
-- Top Gun Maverick: Action, Drama
(12, 1), (12, 6),
-- Oppenheimer: Drama, History
(13, 6), (13, 8),
-- Everything Everywhere All at Once: Action, Adventure, Science Fiction, Comedy
(14, 1), (14, 2), (14, 12), (14, 4),
-- The Silence of the Lambs: Crime, Drama, Horror, Thriller
(15, 5), (15, 6), (15, 9), (15, 13),
-- Breaking Bad: Crime, Drama, Thriller
(16, 5), (16, 6), (16, 13),
-- Stranger Things: Drama, Fantasy, Horror, Mystery
(17, 6), (17, 7), (17, 9), (17, 10),
-- The Last of Us: Drama, Action, Science Fiction
(18, 6), (18, 1), (18, 12),
-- Attack on Titan: Action, Adventure, Animation, Drama
(19, 1), (19, 2), (19, 3), (19, 6),
-- Demon Slayer: Action, Adventure, Animation, Fantasy
(20, 1), (20, 2), (20, 3), (20, 7);

-- ─────────────────────────────────────────────────────────────────────────────
-- Poster images (picsum.photos — deterministic, never 404)
-- ─────────────────────────────────────────────────────────────────────────────
-- Each item gets a portrait poster (300×450) and a landscape backdrop (1280×720).
-- Seed strings ensure the same photo appears on every demo instance.
INSERT OR IGNORE INTO images (item_id, kind, source, source_url, is_primary) VALUES
-- Movies
(1,  'poster',   'demo', 'https://picsum.photos/seed/cfp1/300/450',   1),
(1,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb1/1280/720',  1),
(2,  'poster',   'demo', 'https://picsum.photos/seed/cfp2/300/450',   1),
(2,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb2/1280/720',  1),
(3,  'poster',   'demo', 'https://picsum.photos/seed/cfp3/300/450',   1),
(3,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb3/1280/720',  1),
(4,  'poster',   'demo', 'https://picsum.photos/seed/cfp4/300/450',   1),
(4,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb4/1280/720',  1),
(5,  'poster',   'demo', 'https://picsum.photos/seed/cfp5/300/450',   1),
(5,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb5/1280/720',  1),
(6,  'poster',   'demo', 'https://picsum.photos/seed/cfp6/300/450',   1),
(6,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb6/1280/720',  1),
(7,  'poster',   'demo', 'https://picsum.photos/seed/cfp7/300/450',   1),
(7,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb7/1280/720',  1),
(8,  'poster',   'demo', 'https://picsum.photos/seed/cfp8/300/450',   1),
(8,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb8/1280/720',  1),
(9,  'poster',   'demo', 'https://picsum.photos/seed/cfp9/300/450',   1),
(9,  'backdrop', 'demo', 'https://picsum.photos/seed/cfb9/1280/720',  1),
(10, 'poster',   'demo', 'https://picsum.photos/seed/cfp10/300/450',  1),
(10, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb10/1280/720', 1),
(11, 'poster',   'demo', 'https://picsum.photos/seed/cfp11/300/450',  1),
(11, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb11/1280/720', 1),
(12, 'poster',   'demo', 'https://picsum.photos/seed/cfp12/300/450',  1),
(12, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb12/1280/720', 1),
(13, 'poster',   'demo', 'https://picsum.photos/seed/cfp13/300/450',  1),
(13, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb13/1280/720', 1),
(14, 'poster',   'demo', 'https://picsum.photos/seed/cfp14/300/450',  1),
(14, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb14/1280/720', 1),
(15, 'poster',   'demo', 'https://picsum.photos/seed/cfp15/300/450',  1),
(15, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb15/1280/720', 1),
-- TV Shows
(16, 'poster',   'demo', 'https://picsum.photos/seed/cfp16/300/450',  1),
(16, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb16/1280/720', 1),
(17, 'poster',   'demo', 'https://picsum.photos/seed/cfp17/300/450',  1),
(17, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb17/1280/720', 1),
(18, 'poster',   'demo', 'https://picsum.photos/seed/cfp18/300/450',  1),
(18, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb18/1280/720', 1),
-- Anime
(19, 'poster',   'demo', 'https://picsum.photos/seed/cfp19/300/450',  1),
(19, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb19/1280/720', 1),
(20, 'poster',   'demo', 'https://picsum.photos/seed/cfp20/300/450',  1),
(20, 'backdrop', 'demo', 'https://picsum.photos/seed/cfb20/1280/720', 1);

-- ─────────────────────────────────────────────────────────────────────────────
-- People (cast + directors)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO people (id, name, tmdb_id, photo_url) VALUES
(1,  'Christopher Nolan',    525,   'https://picsum.photos/seed/cfactor525/100/100'),
(2,  'Christian Bale',       3894,  'https://picsum.photos/seed/cfactor3894/100/100'),
(3,  'Heath Ledger',         1810,  'https://picsum.photos/seed/cfactor1810/100/100'),
(4,  'Aaron Eckhart',        10835, 'https://picsum.photos/seed/cfactor10835/100/100'),
(5,  'Leonardo DiCaprio',    6193,  'https://picsum.photos/seed/cfactor6193/100/100'),
(6,  'Joseph Gordon-Levitt', 24045, 'https://picsum.photos/seed/cfactor24045/100/100'),
(7,  'Tim Robbins',          7060,  'https://picsum.photos/seed/cfactor7060/100/100'),
(8,  'Morgan Freeman',       192,   'https://picsum.photos/seed/cfactor192/100/100'),
(9,  'Keanu Reeves',         6384,  'https://picsum.photos/seed/cfactor6384/100/100'),
(10, 'Laurence Fishburne',   2975,  'https://picsum.photos/seed/cfactor2975/100/100'),
(11, 'Matthew McConaughey', 10160,  'https://picsum.photos/seed/cfactor10160/100/100'),
(12, 'John Travolta',        8891,  'https://picsum.photos/seed/cfactor8891/100/100'),
(13, 'Samuel L. Jackson',    2231,  'https://picsum.photos/seed/cfactor2231/100/100'),
(14, 'Brad Pitt',            287,   'https://picsum.photos/seed/cfactor287/100/100'),
(15, 'Edward Norton',        819,   'https://picsum.photos/seed/cfactor819/100/100'),
(16, 'Al Pacino',            1158,  'https://picsum.photos/seed/cfactor1158/100/100'),
(17, 'Marlon Brando',        3021,  'https://picsum.photos/seed/cfactor3021/100/100'),
(18, 'Robert De Niro',       380,   'https://picsum.photos/seed/cfactor380/100/100'),
(19, 'Ray Liotta',           12793, 'https://picsum.photos/seed/cfactor12793/100/100'),
(20, 'Bryan Cranston',       17419, 'https://picsum.photos/seed/cfactor17419/100/100'),
(21, 'Aaron Paul',           84497, 'https://picsum.photos/seed/cfactor84497/100/100'),
(22, 'Winona Ryder',         13205, 'https://picsum.photos/seed/cfactor13205/100/100'),
(23, 'David Harbour',        26963, 'https://picsum.photos/seed/cfactor26963/100/100'),
(24, 'Pedro Pascal',         37625, 'https://picsum.photos/seed/cfactor37625/100/100'),
(25, 'Bella Ramsey',         1241322,'https://picsum.photos/seed/cfactor1241322/100/100');

-- ─────────────────────────────────────────────────────────────────────────────
-- Credits
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO item_credits (item_id, person_id, role, character_name, sort_order) VALUES
-- The Dark Knight
(1, 1,  'Director', NULL,         0),
(1, 2,  'Actor',    'Bruce Wayne', 1),
(1, 3,  'Actor',    'Joker',       2),
(1, 4,  'Actor',    'Harvey Dent', 3),
-- Inception
(2, 1,  'Director', NULL,               0),
(2, 5,  'Actor',    'Dom Cobb',         1),
(2, 6,  'Actor',    'Arthur',           2),
-- The Shawshank Redemption
(3, 7,  'Actor',    'Andy Dufresne',    1),
(3, 8,  'Actor',    'Ellis Boyd Red',   2),
-- The Matrix
(4, 9,  'Actor',    'Neo',              1),
(4, 10, 'Actor',    'Morpheus',         2),
-- Interstellar
(5, 1,  'Director', NULL,               0),
(5, 11, 'Actor',    'Cooper',           1),
-- Pulp Fiction
(6, 12, 'Actor',    'Vincent Vega',     1),
(6, 13, 'Actor',    'Jules Winnfield',  2),
(6, 14, 'Actor',    'Butch Coolidge',   3),
-- The Godfather
(7, 17, 'Actor',    'Don Vito Corleone', 1),
(7, 16, 'Actor',    'Michael Corleone', 2),
-- Fight Club
(8, 14, 'Actor',    'Tyler Durden',     1),
(8, 15, 'Actor',    'Narrator',         2),
-- Goodfellas
(9, 18, 'Actor',    'Jimmy Conway',     1),
(9, 19, 'Actor',    'Henry Hill',       2),
-- Breaking Bad
(16, 20, 'Actor',   'Walter White',     1),
(16, 21, 'Actor',   'Jesse Pinkman',    2),
-- Stranger Things
(17, 22, 'Actor',   'Joyce Byers',      1),
(17, 23, 'Actor',   'Jim Hopper',       2),
-- The Last of Us
(18, 24, 'Actor',   'Joel Miller',      1),
(18, 25, 'Actor',   'Ellie Williams',   2);

-- ─────────────────────────────────────────────────────────────────────────────
-- Seasons
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO seasons (id, show_id, season_number, title, summary, tmdb_id) VALUES
-- Breaking Bad
(1,  16, 1, 'Season 1', 'Walter White, a chemistry teacher, is diagnosed with terminal lung cancer and turns to a life of crime.', 3572),
(2,  16, 2, 'Season 2', 'Tensions rise between Walt and Jesse as their drug empire grows and the DEA closes in.', 3573),
-- Stranger Things
(3,  17, 1, 'Season 1', 'The disappearance of a young boy sparks a chain of events in the small town of Hawkins, Indiana.', 77680),
(4,  17, 2, 'Season 2', 'It is now the fall of 1984 and the gang is back together again, beginning high school.', 77681),
-- The Last of Us
(5,  18, 1, 'Season 1', 'Twenty years after the outbreak, the hardened survivor Joel is hired to smuggle Ellie out of a quarantine zone.', 144593),
-- Attack on Titan
(6,  19, 1, 'Season 1', 'Eren Yaeger and his companions join the military to fight against the man-eating Titans that have forced humanity behind massive walls.', 51285),
(7,  19, 2, 'Season 2', 'Eren and his fellow soldiers discover more secrets about the Titans and the people behind the walls.', 60592),
-- Demon Slayer
(8,  20, 1, 'Season 1', 'Tanjiro Kamado, a kind-hearted boy who sells charcoal for a living, finds his family slaughtered by a demon.', 114801),
(9,  20, 2, 'Season 2', 'Tanjiro and his friends investigate the mysterious disappearances of people in the Entertainment District.', 182038),
(10, 20, 3, 'Season 3', 'Tanjiro heads to the Swordsmith Village to have his broken sword repaired.', 211265);

-- ─────────────────────────────────────────────────────────────────────────────
-- Episodes
-- ─────────────────────────────────────────────────────────────────────────────

-- ── Breaking Bad Season 1 (7 episodes) ──────────────────────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(1,  1, 1, 'Pilot',
 'Walter White, a chemistry teacher with a terminal cancer diagnosis, partners with former student Jesse Pinkman to cook methamphetamine.',
 CAST(strftime('%s','2008-01-20') AS INTEGER) * 1000, 3480000, 62085,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(2,  1, 2, 'Cat''s in the Bag',
 'Walt and Jesse must figure out how to deal with the two people they have taken captive.',
 CAST(strftime('%s','2008-01-27') AS INTEGER) * 1000, 2880000, 62086,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(3,  1, 3, '...And the Bag''s in the River',
 'Walt cleans up the aftermath of a chemical spill, and Jesse reaches an agreement with Emilio''s cousin.',
 CAST(strftime('%s','2008-02-10') AS INTEGER) * 1000, 2880000, 62087,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(4,  1, 4, 'Cancer Man',
 'Walt struggles with a moral dilemma. Jesse tries to reconnect with his parents.',
 CAST(strftime('%s','2008-02-17') AS INTEGER) * 1000, 2880000, 62088,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(5,  1, 5, 'Gray Matter',
 'Walt and Skyler attend a former colleague''s party. Jesse struggles to clean up his act.',
 CAST(strftime('%s','2008-02-24') AS INTEGER) * 1000, 2880000, 62089,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(6,  1, 6, 'Crazy Handful of Nothin''',
 'Walt and Jesse find a new distributor for their product — a drug dealer named Tuco.',
 CAST(strftime('%s','2008-03-02') AS INTEGER) * 1000, 2880000, 62090,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(7,  1, 7, 'A No-Rough-Stuff-Type Deal',
 'Walt and Jesse plan a major new heist to obtain a key ingredient for their meth.',
 CAST(strftime('%s','2008-03-09') AS INTEGER) * 1000, 2880000, 62091,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ── Breaking Bad Season 2 (5 sample episodes) ───────────────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(8,  2, 1, 'Seven Thirty-Seven',
 'Walt and Jesse must deal with Tuco, who has become increasingly violent and erratic.',
 CAST(strftime('%s','2009-03-08') AS INTEGER) * 1000, 2880000, 62093,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(9,  2, 2, 'Grilled',
 'Walt and Jesse are held captive at Tuco''s hideout with his uncle Hector.',
 CAST(strftime('%s','2009-03-15') AS INTEGER) * 1000, 2880000, 62094,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(10, 2, 3, 'Bit by a Dead Bee',
 'Walt and Jesse deal with the aftermath of their encounter with Tuco.',
 CAST(strftime('%s','2009-03-22') AS INTEGER) * 1000, 2880000, 62095,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(11, 2, 4, 'Down',
 'Jesse hits rock bottom after losing everything, while Walt focuses on the business.',
 CAST(strftime('%s','2009-03-29') AS INTEGER) * 1000, 2880000, 62096,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(12, 2, 5, 'Breakage',
 'Walt and Jesse organize a new distribution network by recruiting local drug dealers.',
 CAST(strftime('%s','2009-04-05') AS INTEGER) * 1000, 2880000, 62097,
 (strftime('%s','now') - 9504000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ── Stranger Things Season 1 (8 episodes) ───────────────────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(13, 3, 1, 'The Vanishing of Will Byers',
 'On his way home from a friend''s house, young Will sees something terrifying. Nearby, a peculiar girl emerges from the woods.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198600,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(14, 3, 2, 'The Weirdo on Maple Street',
 'Lucas, Mike and Dustin try to talk to the girl they found in the woods. Joyce and Hopper search for Will.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198601,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(15, 3, 3, 'Holly, Jolly',
 'An anguished Joyce makes a discovery. Elsewhere, Eleven experiences a flashback and remembers the creature she escaped from.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198602,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(16, 3, 4, 'The Body',
 'Refusing to believe Will is dead, Joyce tries to connect with her son. The boys give Eleven a makeover.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198603,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(17, 3, 5, 'The Flea and the Acrobat',
 'Hopper breaks into the lab while the boys get a lesson from Mr. Clarke on the nature of the Upside Down.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198604,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(18, 3, 6, 'The Monster',
 'Eleven''s connection to the creature grows stronger. A battered Hopper strikes a deal with the lab''s ruthless director.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198605,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(19, 3, 7, 'The Bathtub',
 'Eleven makes telepathic contact with Will. Nancy and Jonathan prepare to fight the creature.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198606,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(20, 3, 8, 'The Upside Down',
 'Dr. Brenner''s agents close in on the lab as Eleven faces the Monster.',
 CAST(strftime('%s','2016-07-15') AS INTEGER) * 1000, 2940000, 1198607,
 (strftime('%s','now') - 6048000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ── The Last of Us Season 1 (9 episodes, recently added) ────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(21, 5, 1, 'When You''re Lost in the Darkness',
 'Twenty years after a fungal outbreak ravages civilization, Joel and his partner Tess are hired to smuggle Ellie out of a quarantine zone.',
 CAST(strftime('%s','2023-01-15') AS INTEGER) * 1000, 4980000, 2102451,
 (strftime('%s','now') - 259200) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(22, 5, 2, 'Infected',
 'Ellie gets closer to Joel and Tess as they traverse the ruins of a Boston hotel. The trio encounter terrifying Infected.',
 CAST(strftime('%s','2023-01-22') AS INTEGER) * 1000, 3480000, 2102452,
 (strftime('%s','now') - 259200) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(23, 5, 3, 'Long, Long Time',
 'Joel and Ellie encounter survivalist Bill in his fortified town. A tender story of love plays out across the years.',
 CAST(strftime('%s','2023-01-29') AS INTEGER) * 1000, 4740000, 2102453,
 (strftime('%s','now') - 172800) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(24, 5, 4, 'Please Hold to My Hand',
 'Joel and Ellie navigate the dangers of Kansas City, where a violent rebel uprising has taken control.',
 CAST(strftime('%s','2023-02-05') AS INTEGER) * 1000, 3480000, 2102454,
 (strftime('%s','now') - 172800) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(25, 5, 5, 'Endure and Survive',
 'Henry and Sam find themselves allied with Joel and Ellie. A reckoning is coming.',
 CAST(strftime('%s','2023-02-10') AS INTEGER) * 1000, 3960000, 2102455,
 (strftime('%s','now') - 86400) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(26, 5, 6, 'Kin',
 'Months later, Joel and Ellie reach Wyoming. A reunion and a difficult question.',
 CAST(strftime('%s','2023-02-19') AS INTEGER) * 1000, 3960000, 2102456,
 (strftime('%s','now') - 86400) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(27, 5, 7, 'Left Behind',
 'Ellie recounts the last time she saw her best friend Riley, while Joel struggles to survive.',
 CAST(strftime('%s','2023-02-24') AS INTEGER) * 1000, 3360000, 2102457,
 CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(28, 5, 8, 'When We Are in Need',
 'Joel and Ellie encounter a threatening religious community in the mountains.',
 CAST(strftime('%s','2023-03-03') AS INTEGER) * 1000, 3480000, 2102458,
 CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(29, 5, 9, 'Look for the Light',
 'Joel and Ellie near the end of their long journey. A final, devastating choice.',
 CAST(strftime('%s','2023-03-12') AS INTEGER) * 1000, 4380000, 2102459,
 CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ── Attack on Titan Season 1 (8 sample episodes) ────────────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(30, 6, 1, 'To You, in 2000 Years: The Fall of Shiganshina, Part 1',
 'Young Eren Yeager lives in the walled city of Shiganshina, until it is attacked by Colossal and Armored Titans.',
 CAST(strftime('%s','2013-04-07') AS INTEGER) * 1000, 1440000, 63062,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(31, 6, 2, 'That Day: The Fall of Shiganshina, Part 2',
 'In the aftermath of the Titan attack, the three children join the military to fight the Titans.',
 CAST(strftime('%s','2013-04-14') AS INTEGER) * 1000, 1440000, 63063,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(32, 6, 3, 'A Dim Light Amid Despair: Humanity''s Comeback, Part 1',
 'Years later, Eren, Mikasa, and Armin join the military cadet corps.',
 CAST(strftime('%s','2013-04-21') AS INTEGER) * 1000, 1440000, 63064,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(33, 6, 4, 'The Night of the Closing Ceremony: Humanity''s Comeback, Part 2',
 'The cadets graduate and choose which branch of the military to join.',
 CAST(strftime('%s','2013-04-28') AS INTEGER) * 1000, 1440000, 63065,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(34, 6, 5, 'First Battle: The Struggle for Trost, Part 1',
 'Titans breach Trost District''s gate. Eren and his comrades face their first real battle.',
 CAST(strftime('%s','2013-05-05') AS INTEGER) * 1000, 1440000, 63066,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(35, 6, 6, 'The World the Girl Saw: The Struggle for Trost, Part 2',
 'As the battle for Trost rages, Mikasa recalls how she first met Eren.',
 CAST(strftime('%s','2013-05-12') AS INTEGER) * 1000, 1440000, 63067,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(36, 6, 7, 'Small Blade: The Struggle for Trost, Part 3',
 'The cadets find themselves cornered with a critically low gas supply.',
 CAST(strftime('%s','2013-05-19') AS INTEGER) * 1000, 1440000, 63068,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(37, 6, 8, 'I Can Hear His Heartbeat: The Struggle for Trost, Part 4',
 'A shocking reveal changes the course of the battle for Trost.',
 CAST(strftime('%s','2013-05-26') AS INTEGER) * 1000, 1440000, 63069,
 (strftime('%s','now') - 7776000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ── Demon Slayer Season 1 (8 sample episodes) ───────────────────────────────
INSERT OR IGNORE INTO episodes (id, season_id, episode_number, title, summary, air_date, duration_ms, tmdb_id, added_at, updated_at) VALUES
(38, 8, 1, 'Cruelty',
 'Tanjiro returns home to find his family slaughtered by a demon. Only his sister Nezuko survived — as a demon herself.',
 CAST(strftime('%s','2019-04-06') AS INTEGER) * 1000, 1440000, 1964941,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(39, 8, 2, 'Trainer Sakonji Urokodaki',
 'Tanjiro sets out to become a Demon Slayer, guided by the masked demon hunter Sakonji Urokodaki.',
 CAST(strftime('%s','2019-04-13') AS INTEGER) * 1000, 1440000, 1964942,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(40, 8, 3, 'Sabito and Makomo',
 'Tanjiro trains on a boulder and meets two mysterious children in the forest.',
 CAST(strftime('%s','2019-04-20') AS INTEGER) * 1000, 1440000, 1964943,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(41, 8, 4, 'Final Selection',
 'Tanjiro enters Final Selection, where he must survive seven days in a forest filled with demons.',
 CAST(strftime('%s','2019-04-27') AS INTEGER) * 1000, 1440000, 1964944,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(42, 8, 5, 'My Own Steel',
 'After passing Final Selection, Tanjiro receives his Nichirin Sword and his first mission.',
 CAST(strftime('%s','2019-05-04') AS INTEGER) * 1000, 1440000, 1964945,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(43, 8, 6, 'Swordsman Accompanying a Demon',
 'Tanjiro encounters the demon Swamp Demon and a kidnapped girl.',
 CAST(strftime('%s','2019-05-11') AS INTEGER) * 1000, 1440000, 1964946,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(44, 8, 7, 'Muzan Kibutsuji',
 'Tanjiro crosses paths with Muzan Kibutsuji, the demon who killed his family.',
 CAST(strftime('%s','2019-05-18') AS INTEGER) * 1000, 1440000, 1964947,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),

(45, 8, 8, 'The Smell of Enchanting Blood',
 'Tanjiro and Nezuko face the twin demons assigned by Muzan to kill them.',
 CAST(strftime('%s','2019-05-25') AS INTEGER) * 1000, 1440000, 1964948,
 (strftime('%s','now') - 4320000) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);

-- Episode stills (thumb images) for recently-added episodes
INSERT OR IGNORE INTO images (episode_id, kind, source, source_url, is_primary) VALUES
(21, 'thumb', 'demo', 'https://picsum.photos/seed/cfe21/480/270', 1),
(22, 'thumb', 'demo', 'https://picsum.photos/seed/cfe22/480/270', 1),
(23, 'thumb', 'demo', 'https://picsum.photos/seed/cfe23/480/270', 1),
(24, 'thumb', 'demo', 'https://picsum.photos/seed/cfe24/480/270', 1),
(25, 'thumb', 'demo', 'https://picsum.photos/seed/cfe25/480/270', 1),
(26, 'thumb', 'demo', 'https://picsum.photos/seed/cfe26/480/270', 1),
(27, 'thumb', 'demo', 'https://picsum.photos/seed/cfe27/480/270', 1),
(28, 'thumb', 'demo', 'https://picsum.photos/seed/cfe28/480/270', 1),
(29, 'thumb', 'demo', 'https://picsum.photos/seed/cfe29/480/270', 1);

-- ─────────────────────────────────────────────────────────────────────────────
-- Trending cache — populates the Top 10 rails
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR REPLACE INTO trending_cache (source, media_kind, rank, tmdb_id, title, poster_path, fetched_at) VALUES
('tmdb', 'movie', 1,  155,    'The Dark Knight',                    'https://picsum.photos/seed/cfp1/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 2,  27205,  'Inception',                          'https://picsum.photos/seed/cfp2/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 3,  278,    'The Shawshank Redemption',           'https://picsum.photos/seed/cfp3/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 4,  603,    'The Matrix',                         'https://picsum.photos/seed/cfp4/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 5,  157336, 'Interstellar',                       'https://picsum.photos/seed/cfp5/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 6,  680,    'Pulp Fiction',                       'https://picsum.photos/seed/cfp6/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 7,  238,    'The Godfather',                      'https://picsum.photos/seed/cfp7/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 8,  550,    'Fight Club',                         'https://picsum.photos/seed/cfp8/300/450',   CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 9,  872585, 'Oppenheimer',                        'https://picsum.photos/seed/cfp13/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'movie', 10, 361743, 'Top Gun: Maverick',                  'https://picsum.photos/seed/cfp12/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'show',  1,  1396,   'Breaking Bad',                       'https://picsum.photos/seed/cfp16/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'show',  2,  100088, 'The Last of Us',                     'https://picsum.photos/seed/cfp18/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'show',  3,  66732,  'Stranger Things',                    'https://picsum.photos/seed/cfp17/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'show',  4,  44217,  'Attack on Titan',                    'https://picsum.photos/seed/cfp19/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000),
('tmdb', 'show',  5,  85937,  'Demon Slayer: Kimetsu no Yaiba',     'https://picsum.photos/seed/cfp20/300/450',  CAST(strftime('%s','now') AS INTEGER) * 1000);

-- ─────────────────────────────────────────────────────────────────────────────
-- Play state (user 1 — Continue Watching + watch history)
-- ─────────────────────────────────────────────────────────────────────────────
-- Movies in progress
INSERT OR REPLACE INTO play_state (user_id, item_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at) VALUES
-- The Dark Knight: 40% through
(1, 1,  NULL, 3664000,  9156000,  0, 0, (strftime('%s','now') - 7200) * 1000),
-- Interstellar: 55% through
(1, 5,  NULL, 5577000,  10140000, 0, 0, (strftime('%s','now') - 172800) * 1000),
-- Dune Part One: 25% through
(1, 10, NULL, 2340000,  9360000,  0, 0, (strftime('%s','now') - 259200) * 1000),
-- Oppenheimer: 60% through
(1, 13, NULL, 6624000,  11040000, 0, 0, (strftime('%s','now') - 86400) * 1000),
-- Watched movies
(1, 2,  NULL, 8880000,  8880000,  1, 2, (strftime('%s','now') - 2592000) * 1000),
(1, 7,  NULL, 10560000, 10560000, 1, 1, (strftime('%s','now') - 5184000) * 1000),
(1, 11, NULL, 10860000, 10860000, 1, 1, (strftime('%s','now') - 3888000) * 1000);

-- TV episodes: Breaking Bad — S1E1 and S1E2 watched, S1E3 in progress
INSERT OR REPLACE INTO play_state (user_id, item_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at) VALUES
(1, NULL, 1,  2880000,  2880000, 1, 1, (strftime('%s','now') - 6480000) * 1000),
(1, NULL, 2,  2880000,  2880000, 1, 1, (strftime('%s','now') - 6393600) * 1000),
(1, NULL, 3,  1440000,  2880000, 0, 0, (strftime('%s','now') - 3600) * 1000);

-- The Last of Us — S1E1 in progress (recently started)
INSERT OR REPLACE INTO play_state (user_id, item_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at) VALUES
(1, NULL, 21, 900000, 4980000, 0, 0, (strftime('%s','now') - 1800) * 1000);

-- ─────────────────────────────────────────────────────────────────────────────
-- Upcoming air_dates for the calendar view.
-- air_date is epoch MILLISECONDS (midnight UTC).  Set a handful of episodes
-- to future dates so the Calendar page shows upcoming episodes.
-- ─────────────────────────────────────────────────────────────────────────────
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+3 days'))  AS INTEGER) * 1000 WHERE id = 19;  -- Stranger Things S1E7
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+5 days'))  AS INTEGER) * 1000 WHERE id = 29;  -- The Last of Us S1E9 (finale)
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+7 days'))  AS INTEGER) * 1000 WHERE id = 10;  -- Breaking Bad S2E3
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+8 days'))  AS INTEGER) * 1000 WHERE id = 44;  -- Demon Slayer S1E7
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+10 days')) AS INTEGER) * 1000 WHERE id = 20;  -- Stranger Things S1E8 (finale)
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+12 days')) AS INTEGER) * 1000 WHERE id = 37;  -- Attack on Titan S1E8
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+14 days')) AS INTEGER) * 1000 WHERE id = 11;  -- Breaking Bad S2E4
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+15 days')) AS INTEGER) * 1000 WHERE id = 45;  -- Demon Slayer S1E8
UPDATE episodes SET air_date = CAST(strftime('%s', date('now', '+21 days')) AS INTEGER) * 1000 WHERE id = 12;  -- Breaking Bad S2E5

-- ─────────────────────────────────────────────────────────────────────────────
-- My List (user 1)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO user_my_list (user_id, item_id, added_at) VALUES
(1, 3,  (strftime('%s','now') - 604800) * 1000),   -- Shawshank Redemption
(1, 4,  (strftime('%s','now') - 432000) * 1000),   -- The Matrix
(1, 6,  (strftime('%s','now') - 864000) * 1000),   -- Pulp Fiction
(1, 14, (strftime('%s','now') - 172800) * 1000),   -- Everything Everywhere
(1, 17, (strftime('%s','now') - 259200) * 1000),   -- Stranger Things
(1, 19, (strftime('%s','now') - 345600) * 1000);   -- Attack on Titan

-- ─────────────────────────────────────────────────────────────────────────────
-- media_files — one dummy row per item/episode so items pass the
-- has_active_files_clause() filter in list_items() and appear in browse.
-- Paths follow real scanner conventions but no actual files are required.
-- removed_at omitted (NULL) → treated as active.
-- ─────────────────────────────────────────────────────────────────────────────
INSERT OR IGNORE INTO media_files (item_id, episode_id, path, size_bytes, mtime_ms, container, duration_ms, bit_rate, width, height, scanned_at) VALUES
-- Movies (item_id 1-15)
(1, NULL, '/media/movies/the-dark-knight.mkv', 8500000000, 1704067200000, 'mkv', 9120000, 25000, 1920, 1080, 1704067200),
(2, NULL, '/media/movies/inception.mkv', 7800000000, 1704067200000, 'mkv', 8880000, 22000, 1920, 1080, 1704067200),
(3, NULL, '/media/movies/the-shawshank-redemption.mkv', 6200000000, 1704067200000, 'mkv', 8520000, 18000, 1920, 1080, 1704067200),
(4, NULL, '/media/movies/the-matrix.mkv', 5900000000, 1704067200000, 'mkv', 8160000, 17000, 1920, 1080, 1704067200),
(5, NULL, '/media/movies/interstellar.mkv', 9100000000, 1704067200000, 'mkv', 10200000, 24000, 1920, 1080, 1704067200),
(6, NULL, '/media/movies/pulp-fiction.mkv', 5800000000, 1704067200000, 'mkv', 9120000, 16000, 1920, 1080, 1704067200),
(7, NULL, '/media/movies/the-godfather.mkv', 7200000000, 1704067200000, 'mkv', 10560000, 18000, 1920, 1080, 1704067200),
(8, NULL, '/media/movies/fight-club.mkv', 6100000000, 1704067200000, 'mkv', 8280000, 17000, 1920, 1080, 1704067200),
(9, NULL, '/media/movies/goodfellas.mkv', 6800000000, 1704067200000, 'mkv', 9000000, 19000, 1920, 1080, 1704067200),
(10, NULL, '/media/movies/dune-part-one.mkv', 8900000000, 1704067200000, 'mkv', 9360000, 24000, 3840, 2160, 1704067200),
(11, NULL, '/media/movies/avengers-endgame.mkv', 9500000000, 1704067200000, 'mkv', 10920000, 26000, 1920, 1080, 1704067200),
(12, NULL, '/media/movies/top-gun-maverick.mkv', 7400000000, 1704067200000, 'mkv', 8040000, 22000, 1920, 1080, 1704067200),
(13, NULL, '/media/movies/oppenheimer.mkv', 9200000000, 1704067200000, 'mkv', 11160000, 24000, 1920, 1080, 1704067200),
(14, NULL, '/media/movies/everything-everywhere.mkv', 6500000000, 1704067200000, 'mkv', 7920000, 19000, 1920, 1080, 1704067200),
(15, NULL, '/media/movies/silence-of-the-lambs.mkv', 5700000000, 1704067200000, 'mkv', 7080000, 16000, 1920, 1080, 1704067200),
-- Breaking Bad episodes 1-12
(NULL, 1, '/media/shows/breaking-bad/s01e01.mkv', 1200000000, 1704067200000, 'mkv', 3480000, 8000, 1920, 1080, 1704067200),
(NULL, 2, '/media/shows/breaking-bad/s01e02.mkv', 1100000000, 1704067200000, 'mkv', 3000000, 8000, 1920, 1080, 1704067200),
(NULL, 3, '/media/shows/breaking-bad/s01e03.mkv', 1150000000, 1704067200000, 'mkv', 3120000, 8000, 1920, 1080, 1704067200),
(NULL, 4, '/media/shows/breaking-bad/s01e04.mkv', 1180000000, 1704067200000, 'mkv', 3180000, 8000, 1920, 1080, 1704067200),
(NULL, 5, '/media/shows/breaking-bad/s01e05.mkv', 1090000000, 1704067200000, 'mkv', 2940000, 8000, 1920, 1080, 1704067200),
(NULL, 6, '/media/shows/breaking-bad/s01e06.mkv', 1130000000, 1704067200000, 'mkv', 3060000, 8000, 1920, 1080, 1704067200),
(NULL, 7, '/media/shows/breaking-bad/s01e07.mkv', 1160000000, 1704067200000, 'mkv', 3120000, 8000, 1920, 1080, 1704067200),
(NULL, 8, '/media/shows/breaking-bad/s02e01.mkv', 1200000000, 1704067200000, 'mkv', 3240000, 8000, 1920, 1080, 1704067200),
(NULL, 9, '/media/shows/breaking-bad/s02e02.mkv', 1170000000, 1704067200000, 'mkv', 3060000, 8000, 1920, 1080, 1704067200),
(NULL, 10, '/media/shows/breaking-bad/s02e03.mkv', 1180000000, 1704067200000, 'mkv', 3120000, 8000, 1920, 1080, 1704067200),
(NULL, 11, '/media/shows/breaking-bad/s02e04.mkv', 1150000000, 1704067200000, 'mkv', 3000000, 8000, 1920, 1080, 1704067200),
(NULL, 12, '/media/shows/breaking-bad/s02e05.mkv', 1200000000, 1704067200000, 'mkv', 3180000, 8000, 1920, 1080, 1704067200),
-- Stranger Things episodes 13-20
(NULL, 13, '/media/shows/stranger-things/s01e01.mkv', 1800000000, 1704067200000, 'mkv', 3240000, 12000, 1920, 1080, 1704067200),
(NULL, 14, '/media/shows/stranger-things/s01e02.mkv', 1700000000, 1704067200000, 'mkv', 3000000, 12000, 1920, 1080, 1704067200),
(NULL, 15, '/media/shows/stranger-things/s01e03.mkv', 1750000000, 1704067200000, 'mkv', 3060000, 12000, 1920, 1080, 1704067200),
(NULL, 16, '/media/shows/stranger-things/s01e04.mkv', 1680000000, 1704067200000, 'mkv', 2940000, 12000, 1920, 1080, 1704067200),
(NULL, 17, '/media/shows/stranger-things/s01e05.mkv', 1720000000, 1704067200000, 'mkv', 3000000, 12000, 1920, 1080, 1704067200),
(NULL, 18, '/media/shows/stranger-things/s01e06.mkv', 1760000000, 1704067200000, 'mkv', 3060000, 12000, 1920, 1080, 1704067200),
(NULL, 19, '/media/shows/stranger-things/s01e07.mkv', 1800000000, 1704067200000, 'mkv', 3120000, 12000, 1920, 1080, 1704067200),
(NULL, 20, '/media/shows/stranger-things/s01e08.mkv', 2100000000, 1704067200000, 'mkv', 4800000, 12000, 1920, 1080, 1704067200),
-- The Last of Us episodes 21-29
(NULL, 21, '/media/shows/the-last-of-us/s01e01.mkv', 2400000000, 1704067200000, 'mkv', 5040000, 14000, 1920, 1080, 1704067200),
(NULL, 22, '/media/shows/the-last-of-us/s01e02.mkv', 2200000000, 1704067200000, 'mkv', 4680000, 14000, 1920, 1080, 1704067200),
(NULL, 23, '/media/shows/the-last-of-us/s01e03.mkv', 2600000000, 1704067200000, 'mkv', 5400000, 14000, 1920, 1080, 1704067200),
(NULL, 24, '/media/shows/the-last-of-us/s01e04.mkv', 2100000000, 1704067200000, 'mkv', 4440000, 14000, 1920, 1080, 1704067200),
(NULL, 25, '/media/shows/the-last-of-us/s01e05.mkv', 2300000000, 1704067200000, 'mkv', 4800000, 14000, 1920, 1080, 1704067200),
(NULL, 26, '/media/shows/the-last-of-us/s01e06.mkv', 2000000000, 1704067200000, 'mkv', 4200000, 14000, 1920, 1080, 1704067200),
(NULL, 27, '/media/shows/the-last-of-us/s01e07.mkv', 2150000000, 1704067200000, 'mkv', 4560000, 14000, 1920, 1080, 1704067200),
(NULL, 28, '/media/shows/the-last-of-us/s01e08.mkv', 2050000000, 1704067200000, 'mkv', 4320000, 14000, 1920, 1080, 1704067200),
(NULL, 29, '/media/shows/the-last-of-us/s01e09.mkv', 2800000000, 1704067200000, 'mkv', 5880000, 14000, 1920, 1080, 1704067200),
-- Attack on Titan episodes 30-37
(NULL, 30, '/media/anime/attack-on-titan/s01e01.mkv', 900000000, 1704067200000, 'mkv', 1440000, 10000, 1920, 1080, 1704067200),
(NULL, 31, '/media/anime/attack-on-titan/s01e02.mkv', 880000000, 1704067200000, 'mkv', 1380000, 10000, 1920, 1080, 1704067200),
(NULL, 32, '/media/anime/attack-on-titan/s01e03.mkv', 870000000, 1704067200000, 'mkv', 1380000, 10000, 1920, 1080, 1704067200),
(NULL, 33, '/media/anime/attack-on-titan/s01e04.mkv', 890000000, 1704067200000, 'mkv', 1440000, 10000, 1920, 1080, 1704067200),
(NULL, 34, '/media/anime/attack-on-titan/s01e05.mkv', 860000000, 1704067200000, 'mkv', 1380000, 10000, 1920, 1080, 1704067200),
(NULL, 35, '/media/anime/attack-on-titan/s01e06.mkv', 880000000, 1704067200000, 'mkv', 1380000, 10000, 1920, 1080, 1704067200),
(NULL, 36, '/media/anime/attack-on-titan/s01e07.mkv', 895000000, 1704067200000, 'mkv', 1440000, 10000, 1920, 1080, 1704067200),
(NULL, 37, '/media/anime/attack-on-titan/s01e08.mkv', 875000000, 1704067200000, 'mkv', 1380000, 10000, 1920, 1080, 1704067200),
-- Demon Slayer episodes 38-45
(NULL, 38, '/media/anime/demon-slayer/s01e01.mkv', 950000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 39, '/media/anime/demon-slayer/s01e02.mkv', 930000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 40, '/media/anime/demon-slayer/s01e03.mkv', 940000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 41, '/media/anime/demon-slayer/s01e04.mkv', 920000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 42, '/media/anime/demon-slayer/s01e05.mkv', 960000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 43, '/media/anime/demon-slayer/s01e06.mkv', 935000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 44, '/media/anime/demon-slayer/s01e07.mkv', 945000000, 1704067200000, 'mkv', 1440000, 11000, 1920, 1080, 1704067200),
(NULL, 45, '/media/anime/demon-slayer/s01e08.mkv', 1100000000, 1704067200000, 'mkv', 1500000, 11000, 1920, 1080, 1704067200);

-- ─────────────────────────────────────────────────────────────────────────────
-- FTS index backfill (keep search working after direct SQL insert)
-- ─────────────────────────────────────────────────────────────────────────────
INSERT INTO items_fts(rowid, title, original_title, summary, cast_names)
SELECT
    i.id,
    i.title,
    COALESCE(i.original_title, ''),
    COALESCE(i.summary, ''),
    COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = i.id),
        ''
    )
FROM items i
WHERE NOT EXISTS (SELECT 1 FROM items_fts WHERE items_fts.rowid = i.id);

-- Done.
SELECT 'Demo seed complete. Open http://localhost:3001 to explore ChimpFlix.' AS message;
