# photo-archive

A terminal UI for managing photography archives on Backblaze B2, built in Rust.

## Features

- Browse photoshoots stored in B2, with sizes, newest first
- Check whether a shoot is already downloaded locally before pulling
- Download RAW only (.CR2), JPEG only, or both
- Sync local photos up to B2
- Sync Lightroom library up or down

All transfers are handled via [rclone](https://rclone.org) and are fully resumable — re-running any operation will skip already-synced files.

## Prerequisites

- [rclone](https://rclone.org) installed and configured with a `b2` remote pointing at `wlta-photography`
- Rust (install via [rustup](https://rustup.rs))

## Setup

```bash
# Configure rclone B2 remote (one-time)
rclone config create b2 b2 account <keyID> key <applicationKey>

# Build
cargo build --release

# Run
./target/release/photo-archive
```

## Structure

```
b2:wlta-photography/
  Pictures/
    2025/
      2025-08-20/   ← individual shoots
      2025-06-02/
      ...
    Lightroom/      ← Lightroom catalog and library
```

Local mirror lives at `~/Pictures/`, maintaining the same folder structure.

## Configuration

Paths and bucket name are in `src/config.rs`. Edit and rebuild if anything changes.
