# photo-archive

A terminal UI for managing photography archives on Backblaze B2, built in Rust.

## Features

**Browse & download**
- Lists all photoshoots stored in B2, with sizes and shoot metadata, newest first
- Per-shoot metadata: model, location, notes — stored as `shoot.json` alongside the photos in B2
- Download RAW only (.CR2), JPEG only, or both
- Checks local sync status before downloading

**Previews**
- Generate JPEG previews (1024px, via `sips`) from local files and upload to B2
- Prefers JPEG source over CR2 when both exist, to avoid RAW decode overhead
- Browse previews: downloads from B2 to a temp directory and opens in Finder
- Bulk option to generate previews for all shoots that are missing them

**Sync**
- Sync all local photos up to B2
- Lightroom library sync: up (local → B2) or down (B2 → local)

**Purge local copies**
- Verifies every local file exists on B2 with a matching checksum before deleting anything
- Per-shoot or bulk purge of fully-synced shoots
- Bulk purge skips any shoot not fully verified — they are left untouched
- Deletion is done via the filesystem only; rclone is never used to delete

**Delete from B2**
- Per-shoot only — no bulk delete
- Requires two explicit confirmations before proceeding
- Local files are never affected

All transfers use [rclone](https://rclone.org) and are fully resumable.

## Prerequisites

- [rclone](https://rclone.org) installed and configured with a `b2` remote
- Rust (install via [rustup](https://rustup.rs))
- macOS (preview generation uses `/usr/bin/sips`)

## Setup

```bash
# Configure rclone B2 remote (one-time)
rclone config create b2 b2 account <keyID> key <applicationKey>

# Build
cargo build --release

# Run
./target/release/photo-archive
```

## B2 structure

```
b2:your-bucket/
  Pictures/
    2025/
      2025-08-20/
        *.CR2
        *.jpg
        shoot.json       ← shoot metadata (model, location, notes)
        previews/        ← generated JPEG previews
      2025-06-02/
      ...
    Lightroom/           ← Lightroom catalog and library
```

Local mirror lives at `~/Pictures/`, maintaining the same folder structure. Preview files and `shoot.json` are B2-only and excluded from local downloads.

## Configuration

Copy `.env.example` to `.env` and set your bucket name and local paths. Values are read at startup; no rebuild needed.
