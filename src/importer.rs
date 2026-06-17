use std::path::{Path, PathBuf};
use chrono::NaiveDate;
use anyhow::Result;
use crate::scanner::Shoot;

const JPEG_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

fn subfolder_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    match ext.as_deref() {
        Some(e) if JPEG_EXTENSIONS.contains(&e) => "jpeg",
        _ => "raw",
    }
}

fn shoot_path(base: &Path, date: &NaiveDate) -> PathBuf {
    base.join(date.format("%Y").to_string())
        .join(date.format("%Y-%m-%d").to_string())
}

pub struct ImportResult {
    pub copied: usize,
    pub skipped: usize,
}

/// Copies all files in a shoot into `base_dir/YYYY/YYYY-MM-DD/{raw,jpeg}/`.
/// Files that already exist with matching sizes are skipped.
pub fn import_shoot(shoot: &Shoot, base_dir: &Path) -> Result<ImportResult> {
    let dest_dir = shoot_path(base_dir, &shoot.date);
    let total = shoot.files.len();
    let mut copied = 0usize;
    let mut skipped = 0usize;

    for (i, src) in shoot.files.iter().enumerate() {
        let filename = src
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("file has no name: {}", src.display()))?;
        let sub = dest_dir.join(subfolder_for(src));
        std::fs::create_dir_all(&sub)?;
        let dest = sub.join(filename);

        if dest.exists() {
            let src_size = std::fs::metadata(src)?.len();
            let dest_size = std::fs::metadata(&dest)?.len();
            if src_size == dest_size {
                skipped += 1;
                continue;
            }
        }

        print!(
            "  [{}/{}] {} ...",
            i + 1,
            total,
            filename.to_string_lossy()
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::fs::copy(src, &dest)?;
        println!(" done");
        copied += 1;
    }

    Ok(ImportResult { copied, skipped })
}

/// Verifies every source file is present at the destination with a matching size.
pub fn verify_import(shoot: &Shoot, base_dir: &Path) -> Result<Vec<PathBuf>> {
    let dest_dir = shoot_path(base_dir, &shoot.date);
    let mut missing = vec![];

    for src in &shoot.files {
        let filename = match src.file_name() {
            Some(n) => n,
            None => continue,
        };
        let dest = dest_dir.join(subfolder_for(src)).join(filename);

        let ok = dest.exists() && {
            let src_size = std::fs::metadata(src)?.len();
            let dest_size = std::fs::metadata(&dest)?.len();
            src_size == dest_size
        };

        if !ok {
            missing.push(src.clone());
        }
    }

    Ok(missing)
}

/// Deletes all source files in the shoot from the card.
pub fn delete_from_card(shoot: &Shoot) -> Result<usize> {
    let mut deleted = 0usize;
    for src in &shoot.files {
        std::fs::remove_file(src)?;
        deleted += 1;
    }
    Ok(deleted)
}
