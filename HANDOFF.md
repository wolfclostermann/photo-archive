# Session handoff

**Branch**: `feature/hot-cold-storage`
**Paused**: 2026-06-16

## Done this session

- Added `src/storage.rs` (renamed from `src/b2.rs`) — fully backend-agnostic rclone wrapper
  - `Shoot` struct now has `hot_path: Option<String>` and `cold_path: Option<String>` instead of a single `remote_path`
  - `display_name()` shows tier badge: `[H+C]`, `[H]`, `[C]`
  - `active_remote()` / `active_backend()` helpers prefer hot, fall back to cold
  - New: `list_shoots_from(remote, tier)`, `merge_shoot_lists()`, `copy_remote_to_remote()`, `verify_remote_synced()`, `remote_exists()`, `create_remote_dir()`
  - `--b2-chunk-size 96M` conditionalized via `Backend::needs_b2_chunk_flag()`
  - `delete_from_b2` renamed to `delete_remote`
- Updated `src/config.rs` — `Backend` enum (`OneDrive`=hot, `B2`=cold), `Tier` enum, `BackendConfig` struct, `Config::load()` replaces `Default` impl; reads `ONEDRIVE_*` and `B2_*` env vars independently
- Updated `src/main.rs` — merged shoot listing from both tiers, presence-aware shoot menu, tier-transfer actions ("Archive to cold", "Restore to hot", "Remove from hot"), delete submenu per tier, `sync_all_up` pushes to all backends, lightroom defaults to hot
- Updated `.env.example` — added `ONEDRIVE_PHOTOSETS_REMOTE` / `ONEDRIVE_LIGHTROOM_REMOTE` with hot/cold section comments
- Added startup check `ensure_hot_dirs()` in `main()` — if hot storage remote doesn't exist, prompts to create it via `rclone mkdir`

## In progress / next steps

- User is configuring rclone for OneDrive — rclone config lives at `/Users/wolf/.config/rclone/rclone.conf`
- Once rclone is configured and `.env` has `ONEDRIVE_PHOTOSETS_REMOTE` set, the app should work end-to-end
- Consider whether to also run `ensure_hot_dirs` check for `lightroom_remote` on hot backend (currently only checks `photosets_remote`)
- No known bugs; build is clean (`cargo build` 0 warnings)

## Context to carry forward

- All storage operations go through rclone CLI — no direct API calls. OneDrive support is purely a matter of rclone config + env vars.
- `Backend::tier()` is intentionally `#[allow(dead_code)]` — it's the extensibility hook for future backends (AWS Glacier, Google Archive, etc.)
- When adding a new hot backend: add variant to `Backend`, wire env vars in `Config::load()`, add `needs_b2_chunk_flag()` return false for it
- Metadata (`shoot.json`) is written to ALL remotes a shoot exists on when editing, to keep tiers in sync
- Purge-local verification uses cold remote preferentially (cold = permanent archive); falls back to hot if cold not present
- Previews are uploaded to the hot/active remote only — not copied to cold (previews are for quick browsing, cold is archival)
