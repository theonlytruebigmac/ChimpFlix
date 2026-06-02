# Settings/Admin redesign + feature program — operator runtime-test checklist

Branch: `redesign/settings-admin-ia` (uncommitted — you commit). `cargo check --workspace`
and `next build` are both green, but **sqlx here is runtime-checked**, so `cargo check`
does NOT validate SQL. This checklist is how you confirm the migrations + queries
actually work before merge. Recommend testing against a **copy/backup** of the DB first
(one feature performs a destructive library-wide delete).

> Note: `cargo check --all-targets` has **pre-existing, unrelated** `MalClient` test errors
> (confirmed on a clean baseline). The production gate is `cargo check --workspace` (green).
> Decisions captured: **watch-party** = deferred to its own project (shown as an honest
> "coming soon" row). **kids-safe** = built lightweight/fail-open (see §9).

## 0. Boot — migrations apply
- [ ] Start the server; confirm migrations **phase98–phase107** apply with no error
      (live DB was at phase97). Tail `server.log` for migration failures.
- [ ] phase105 seeds the new-content ledger: on first boot it pre-marks existing content
      (~829 movies + ~5,568 episodes here) so the **first scan does NOT blast the
      back-catalogue** as "new". Confirm no mass notification on the next scan.

## 1. Styling (all 19 pages)
- [ ] Every page under `/settings` renders in the console design: sidebar with **nav icons +
      active accent bar**, You/Server context switch, cards/setting-rows/pills/stat-tiles/
      tabs, **no page titles**, content full-width centred.
- [ ] Avatar menu shows **Settings only** (no separate "Admin"); the **Server** tab inside
      settings is owner-gated.

## 2. Discord per-user webhook (integrations)
- [ ] Integrations → Discord tile: **Connect**, paste a webhook URL, Save → shows Connected.
- [ ] Trigger a notifiable event → a Discord embed arrives.
- [ ] Break the URL / network and confirm `server.log` does **not** print the webhook token
      on failure. **Disconnect** clears it. A non-Discord URL is rejected (400).

## 3. Admin controls
- [ ] **Invite-only**: General → turn "Allow new sign-ups" off → self-registration is
      refused; an **invite-based** signup still works.
- [ ] **Account lock**: Users drawer → Lock a non-owner → that user can't log in (gate is
      after password check); the **owner cannot be locked**; "Locked" filter counts/filters.
- [ ] General shows **Version** + **Data directory**; Overview Library tile shows
      **movies vs episodes** split; Network shows a persisted **"reachable · checked Xm ago"**.

## 4. Data layer
- [ ] Overview **Now Playing** + Logs **Audit** show real **usernames + media titles**
      (not `#id`).
- [ ] Activity hero shows **Minutes watched / ≈ N hours**; **Top Users** ranked by watch
      time. Numbers are sane (watch-time = finishes-or-furthest-position, not inflated).

## 5. Ops (maintenance + logs)
- [ ] Maintenance → Bulk ops → **whole-library**: Mark watched / unwatched / Re-scan, and
      **Delete** (requires typing the exact library name; audit-logged; deletes only that
      library's content, not the library row). Verify on a **copy** first.
- [ ] Logs → Audit: **Action** + **date-range** filters; **Export CSV** (open it — a
      `=`/`+`/`-`/`@`-leading user-agent must be quoted, not executed). Logs: **module
      filter** + **Export**.

## 6. Media-A (transcoding)
- [ ] **Re-probe** button refreshes the detected-hardware chips without a restart.
- [ ] **Burn-in subtitles** OFF (default) → text subs still use the WebVTT sidecar
      (playback unchanged). ON → text subs are burned into a transcode.
- [ ] **Two-pass loudnorm** defaults **ON** (preserves prior behavior when loudness
      measurements exist). Toggling OFF forces single-pass.

## 7. Media-B (libraries / optimized / webhooks)
- [ ] Library drawer: **edit name + paths** (save shows a "run a scan" hint), **Scans**
      tab lists scan history, stats show **runtime / with-poster % / missing-IDs**.
- [ ] Optimized versions: a running encode shows a **live progress bar**; **Cancel** stops
      ffmpeg + removes the partial file; finished rows keep Delete.
- [ ] Admin → Notifications → Webhooks: each row shows a **last-delivery** pill
      (Pending / 2xx ok / failed) + relative time.

## 8. Notifications cluster
- [ ] `/settings/notifications`: quiet-hours as **HH:MM** in your **profile timezone**
      (set the timezone); confirm an email is held back inside the window (bell still records).
- [ ] Add a **new movie** to a library → users with access + "New movies" enabled get **one**
      notification. Add **several episodes** of one show → **one** "N new episodes of <Show>"
      (not one per episode); only users who **watch that show** are notified.
- [ ] Per-kind in-app/email/discord toggles work; **security (2FA) alerts always deliver**;
      owner-only kinds (Job failures / New signups) show only for owners.

## 9. Home & visibility
- [ ] Rail **enable/disable + reorder** persists and the home page reflects it; a user with
      no prefs sees the **default** home unchanged.
- [ ] Per-library **show/hide** still works.
- [ ] **Kids-safe**: it's **fail-open** — turning it ON does **not** blank the library
      (no item has an age-rating yet, so it's a no-op until ratings are populated; it then
      hides only explicitly-mature-rated titles). Confirm enabling it doesn't trigger a false
      "scan in progress" screen. *(Decision pending: leave as-is, populate ratings, or drop.)*

## 10. Tri-state per-library access (None / View / Full)
- [ ] Existing users still **browse + play** everything (all prior grants migrated to Full).
- [ ] Set a user to **View** on a library → they **see** it + metadata but **cannot play**:
      verify a 403 on **direct play, transcode/HLS, and cast** start. **Full** plays; **None**
      hides the library. Set via the Access **matrix**, **groups** (per-library level), and the
      **user drawer** Access tab.

---
Nothing is committed. After validating, commit on `redesign/settings-admin-ia` and merge.
