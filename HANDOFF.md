# Handoff: OneDrive hot-storage folder is moving

_Written 2026-10-14 during a OneDrive reorganisation._

## What changed

The shoots folder on OneDrive is being renamed as part of a wider tidy-up:

| Before | After |
|---|---|
| `onedrive:Pictures/Photosets` (holds `2025/`, `2026/YYYY-MM-DD/`) | `onedrive:Pictures/Shoots` |
| `onedrive:Pictures/Model Shoots/Tash` | `onedrive:Pictures/Shoots/Tash` |

The move is server-side, so file contents, checksums and the `YYYY/YYYY-MM-DD` layout are unchanged.
Only the path is different. `Tash/` sits beside the year folders, but `list_shoots_from`
(`src/storage.rs`) only matches `YYYY/YYYY-MM-DD` directories, so the app ignores it.

## What needs updating

1. **`.env` on the Mac** (gitignored, not in this repo). Set:
   ```
   ONEDRIVE_REMOTE=onedrive:Pictures/Shoots
   ```
   It most likely points at `onedrive:Pictures/Photosets` today. Check the real value first.
   If it pointed somewhere else, this move doesn't affect it.
2. **`ONEDRIVE_LIGHTROOM_REMOTE`**: no Lightroom folder was found anywhere in OneDrive during the
   reorganisation (only the phone's `Pictures/Samsung Gallery/Pictures/AdobeLightroom`, which is
   unrelated). Confirm where this points. If the folder doesn't exist yet, `Pictures/Lightroom`
   fits the new layout.
3. **`.env.example`**: still shows `onedrive:Photos/Shoots`. Consider changing it to
   `onedrive:Pictures/Shoots` to match.

## Watch out for

- If you run the app before updating `.env`, `main.rs` will find the old remote missing and offer
  to **create** it, and the prompt defaults to Yes. That would make a new empty `Pictures/Photosets`, and every shoot would show as
  not on hot storage. Decline, and fix `.env` first.
- Don't run a sync or purge until the hot remote lists the expected shoots again. A purge verifies
  against hot/cold copies, and it would refuse or misjudge while the path is wrong.

Delete this file once `.env` is updated and the app lists shoots from the new path.
