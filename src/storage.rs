use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::config::{Backend, Config, Tier};

#[derive(Debug, Deserialize)]
struct RcloneLsEntry {
    #[serde(rename = "IsDir")]
    is_dir: bool,
    #[serde(rename = "Path")]
    path: String,
}

#[derive(Debug, Deserialize)]
struct RcloneSizeOutput {
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone)]
pub struct Shoot {
    pub name: String,
    pub year: String,
    pub hot_path: Option<String>,
    pub cold_path: Option<String>,
    pub size_bytes: Option<u64>,
    pub metadata: Option<Metadata>,
}

impl Shoot {
    /// Preferred remote for read operations: hot if available, else cold.
    pub fn active_remote(&self) -> Option<&str> {
        self.hot_path.as_deref().or(self.cold_path.as_deref())
    }

    /// Backend associated with active_remote.
    pub fn active_backend<'a>(&self, config: &'a Config) -> Option<&'a Backend> {
        if self.hot_path.is_some() {
            config.hot.as_ref().map(|bc| &bc.backend)
        } else {
            config.cold.as_ref().map(|bc| &bc.backend)
        }
    }

    pub fn previews_remote(&self) -> Option<String> {
        self.active_remote().map(|r| format!("{}/previews", r))
    }

    pub fn local_path(&self, local_shoots: &Path) -> std::path::PathBuf {
        local_shoots.join(&self.year).join(&self.name)
    }

    pub fn display_name(&self) -> String {
        let badge = match (&self.hot_path, &self.cold_path) {
            (Some(_), Some(_)) => "[H+C]",
            (Some(_), None)    => "[H]  ",
            (None, Some(_))    => "[C]  ",
            (None, None)       => "     ",
        };

        let meta = self.metadata.as_ref().map(|m| {
            let parts: Vec<&str> = [m.model.as_str(), m.location.as_str()]
                .iter()
                .copied()
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(" · ")
        });

        let name_and_meta = match meta.as_deref() {
            Some(m) if !m.is_empty() => format!("{}  {}", self.name, m),
            _ => self.name.clone(),
        };

        match self.size_bytes {
            Some(b) => format!("{}  {}  ({})", badge, name_and_meta, format_bytes(b)),
            None    => format!("{}  {}", badge, name_and_meta),
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_073_741_824;
    const MB: u64 = 1_048_576;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn remote_exists(remote: &str) -> bool {
    Command::new("rclone")
        .args(["lsd", remote, "--max-depth", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn create_remote_dir(remote: &str) -> Result<()> {
    let status = Command::new("rclone")
        .args(["mkdir", remote])
        .status()
        .context("failed to run rclone mkdir")?;
    if !status.success() {
        anyhow::bail!("failed to create remote directory");
    }
    Ok(())
}

/// Lists shoots from a single remote, tagging each entry with the given tier path field.
pub fn list_shoots_from(remote: &str, tier: &Tier) -> Result<Vec<Shoot>> {
    let output = Command::new("rclone")
        .args(["lsjson", "--dirs-only", "--recursive", remote])
        .output()
        .context("failed to run rclone lsjson")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("rclone lsjson failed: {}", stderr);
    }

    let entries: Vec<RcloneLsEntry> = serde_json::from_slice(&output.stdout)
        .context("failed to parse rclone lsjson output")?;

    let shoots = entries
        .into_iter()
        .filter(|e| {
            if !e.is_dir { return false; }
            let mut parts = e.path.splitn(2, '/');
            let year = parts.next().unwrap_or("");
            let name = parts.next().unwrap_or("");
            year.len() == 4 && year.chars().all(|c| c.is_ascii_digit())
                && name.len() == 10 && name.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() }
                })
        })
        .map(|e| {
            let (year, name) = e.path.split_once('/').unwrap();
            let remote_path = format!("{}/{}", remote, e.path);
            let (hot_path, cold_path) = match tier {
                Tier::Hot  => (Some(remote_path), None),
                Tier::Cold => (None, Some(remote_path)),
            };
            Shoot {
                name: name.to_string(),
                year: year.to_string(),
                hot_path,
                cold_path,
                size_bytes: None,
                metadata: None,
            }
        })
        .collect();

    Ok(shoots)
}

/// Merges shoot lists from hot and cold backends into a single deduplicated list.
pub fn merge_shoot_lists(hot: Vec<Shoot>, cold: Vec<Shoot>) -> Vec<Shoot> {
    let mut map: HashMap<String, Shoot> = HashMap::new();
    for shoot in hot {
        map.insert(shoot.name.clone(), shoot);
    }
    for cold_shoot in cold {
        map.entry(cold_shoot.name.clone())
            .and_modify(|existing| {
                existing.cold_path = cold_shoot.cold_path.clone();
            })
            .or_insert(cold_shoot);
    }
    let mut shoots: Vec<Shoot> = map.into_values().collect();
    shoots.sort_by(|a, b| b.name.cmp(&a.name));
    shoots
}

pub fn fetch_shoot_size(remote_path: &str) -> Result<u64> {
    let output = Command::new("rclone")
        .args(["size", "--json", remote_path])
        .output()
        .context("failed to run rclone size")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("rclone size failed: {}", stderr);
    }

    let size: RcloneSizeOutput =
        serde_json::from_slice(&output.stdout).context("failed to parse rclone size output")?;

    Ok(size.bytes)
}

pub fn fetch_metadata(remote_path: &str) -> Option<Metadata> {
    let json_path = format!("{}/shoot.json", remote_path);
    let output = Command::new("rclone")
        .args(["cat", &json_path])
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }

    serde_json::from_slice(&output.stdout).ok()
}

pub fn save_metadata(remote_path: &str, metadata: &Metadata) -> Result<()> {
    let json = serde_json::to_string_pretty(metadata)?;
    let tmp = std::env::temp_dir().join("shoot-manager-shoot.json");
    std::fs::write(&tmp, json)?;

    let dest = format!("{}/shoot.json", remote_path);
    let status = Command::new("rclone")
        .args(["copyto", tmp.to_str().unwrap(), &dest])
        .status()
        .context("failed to run rclone copyto")?;

    if !status.success() {
        anyhow::bail!("failed to save metadata");
    }
    Ok(())
}

/// Scans a shoot's local directory (including raw/ and jpeg/ subdirs) for photo source files.
/// Prefers JPEG over CR2 for the same filename stem to avoid RAW decode overhead.
fn collect_photo_sources(local: &Path) -> Result<HashMap<String, std::path::PathBuf>> {
    let mut sources: HashMap<String, std::path::PathBuf> = HashMap::new();
    let search_dirs = [local.to_path_buf(), local.join("raw"), local.join("jpeg")];
    for dir in &search_dirs {
        if !dir.exists() { continue; }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() { continue; }
            let ext = path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !matches!(ext.as_str(), "cr2" | "jpg" | "jpeg") { continue; }
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let is_jpeg = matches!(ext.as_str(), "jpg" | "jpeg");
            if is_jpeg || !sources.contains_key(&stem) {
                sources.insert(stem, path);
            }
        }
    }
    Ok(sources)
}

/// Generates JPEG previews from local shoot files and uploads them to the given remote.
/// Prefers JPEG source over CR2 for the same filename stem to avoid RAW decode overhead.
pub fn generate_and_upload_previews(
    shoot: &Shoot,
    local: &Path,
    upload_remote: &str,
    backend: &Backend,
) -> Result<()> {
    if !local.exists() {
        anyhow::bail!("shoot is not downloaded locally — download it first");
    }

    let preview_dir = std::env::temp_dir()
        .join("shoot-manager-previews")
        .join(&shoot.name);
    std::fs::create_dir_all(&preview_dir)?;

    let sources = collect_photo_sources(local)?;

    if sources.is_empty() {
        anyhow::bail!("no CR2 or JPEG files found in {}", local.display());
    }

    let total = sources.len();
    println!("Generating {} previews...", total);

    let mut generated = 0usize;
    let mut first_error: Option<String> = None;

    for (stem, src) in &sources {
        let out = preview_dir.join(format!("{}.jpg", stem));
        print!("\r  {}/{}", generated + 1, total);
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let result = Command::new("/usr/bin/sips")
            .args([
                "-s", "format", "jpeg",
                "--resampleHeightWidthMax", "1024",
                src.to_str().unwrap(),
                "--out", out.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .context("failed to run sips")?;

        if result.status.success() {
            generated += 1;
        } else if first_error.is_none() {
            first_error = Some(String::from_utf8_lossy(&result.stderr).trim().to_string());
        }
    }
    println!("\r  {}/{} done", generated, total);

    if generated == 0 {
        let msg = first_error.unwrap_or_else(|| "unknown error".into());
        anyhow::bail!(
            "sips failed to generate any previews.\nFirst error: {}\n\
            If this is a permissions error, grant Terminal Full Disk Access in\n\
            System Settings → Privacy & Security → Full Disk Access.",
            msg
        );
    }

    println!("Uploading previews...");
    let mut args = vec![
        "copy".to_string(),
        preview_dir.to_str().unwrap().to_string(),
        upload_remote.to_string(),
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
    ];
    if backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }
    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("failed to upload previews");
    }

    Ok(())
}

pub fn previews_exist(previews_remote: &str) -> bool {
    Command::new("rclone")
        .args(["ls", previews_remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Downloads previews to a temp directory and opens it in Finder.
pub fn browse_previews(shoot_name: &str, previews_remote: &str) -> Result<()> {
    let check = Command::new("rclone")
        .args(["ls", previews_remote])
        .stderr(Stdio::null())
        .output()
        .context("failed to run rclone ls")?;

    if check.stdout.is_empty() {
        anyhow::bail!("no previews found for this shoot — generate them first");
    }

    let preview_dir = std::env::temp_dir()
        .join("shoot-manager-previews")
        .join(shoot_name);
    std::fs::create_dir_all(&preview_dir)?;

    println!("Downloading previews...");
    let status = Command::new("rclone")
        .args([
            "copy",
            previews_remote,
            preview_dir.to_str().unwrap(),
            "--progress",
            "--transfers", "4",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("failed to download previews");
    }

    Command::new("open")
        .arg(preview_dir.to_str().unwrap())
        .spawn()
        .context("failed to open Finder")?;

    Ok(())
}

/// Verifies every local file exists on the remote with a matching checksum (one-way: local→remote).
pub fn verify_local_synced(local: &Path, remote: &str) -> Result<bool> {
    if !local.exists() {
        return Ok(false);
    }

    let output = Command::new("rclone")
        .args([
            "check",
            local.to_str().unwrap(),
            remote,
            "--one-way",
            "--exclude", "shoot.json",
            "--exclude", "previews/**",
            "--exclude", ".DS_Store",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("failed to run rclone check")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let errors: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("ERROR") || l.contains("not found"))
            .collect();
        if !errors.is_empty() {
            anyhow::bail!("{}", errors.join("\n"));
        }
        return Ok(false);
    }

    Ok(true)
}

/// Verifies every file in src exists in dst with a matching checksum (one-way: src→dst).
pub fn verify_remote_synced(src: &str, dst: &str) -> Result<bool> {
    let output = Command::new("rclone")
        .args([
            "check",
            src,
            dst,
            "--one-way",
            "--exclude", "previews/**",
            "--exclude", ".DS_Store",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("failed to run rclone check")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let errors: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("ERROR") || l.contains("not found"))
            .collect();
        if !errors.is_empty() {
            anyhow::bail!("{}", errors.join("\n"));
        }
        return Ok(false);
    }

    Ok(true)
}

/// Permanently deletes a remote path. Local files are not touched.
pub fn delete_remote(remote_path: &str) -> Result<()> {
    let status = Command::new("rclone")
        .args(["purge", remote_path])
        .status()
        .context("failed to run rclone purge")?;

    if !status.success() {
        anyhow::bail!("rclone purge failed");
    }
    Ok(())
}

/// Deletes a local directory using the filesystem only. No rclone involved.
pub fn purge_local(local: &Path) -> Result<()> {
    std::fs::remove_dir_all(local)
        .with_context(|| format!("failed to delete {}", local.display()))
}

pub enum LocalStatus {
    NotDownloaded,
    Synced,
    OutOfSync,
}

pub fn check_local_status(local: &Path, remote: &str) -> LocalStatus {
    if !local.exists() {
        return LocalStatus::NotDownloaded;
    }

    let status = Command::new("rclone")
        .args([
            "check",
            "--one-way",
            "--quiet",
            remote,
            local.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() => LocalStatus::Synced,
        _ => LocalStatus::OutOfSync,
    }
}

pub enum DownloadFilter {
    RawOnly,
    JpegOnly,
    Both,
}

#[derive(Debug, PartialEq)]
pub enum ShootLayout {
    Flat,
    Split,
}

/// Returns Split if the local shoot directory already has raw/ or jpeg/ subdirectories.
pub fn detect_local_layout(local: &Path) -> ShootLayout {
    if local.join("raw").exists() || local.join("jpeg").exists() {
        ShootLayout::Split
    } else {
        ShootLayout::Flat
    }
}


/// Reorganises a shoot from a flat layout into raw/ and jpeg/ subdirectories.
/// Migrates the local copy first (if present), then each remote tier.
/// Already-split tiers are skipped so the operation is idempotent.
pub fn migrate_shoot_to_split(
    local: Option<&Path>,
    remotes: &[(&str, &Backend)],
) -> Result<()> {
    let mut any_work = false;

    if let Some(local_path) = local {
        if local_path.exists() && detect_local_layout(local_path) == ShootLayout::Flat {
            println!("  Migrating local copy...");
            let raw_dir  = local_path.join("raw");
            let jpeg_dir = local_path.join("jpeg");

            let mut to_move: Vec<(std::path::PathBuf, std::path::PathBuf)> = vec![];
            for entry in std::fs::read_dir(local_path)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_file() { continue; }
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let dest_dir = match ext.as_str() {
                    "cr2"        => &raw_dir,
                    "jpg" | "jpeg" => &jpeg_dir,
                    _ => continue,
                };
                let filename = path.file_name().unwrap().to_owned();
                to_move.push((path, dest_dir.join(filename)));
            }

            let mut needed_dirs: std::collections::HashSet<std::path::PathBuf> =
                std::collections::HashSet::new();
            for (_, dst) in &to_move {
                needed_dirs.insert(dst.parent().unwrap().to_path_buf());
            }
            for dir in &needed_dirs {
                std::fs::create_dir_all(dir)?;
            }
            for (src, dst) in to_move {
                std::fs::rename(&src, &dst)
                    .with_context(|| format!("failed to move {}", src.display()))?;
            }
            println!("  Local copy migrated.");
            any_work = true;
        } else if local_path.exists() {
            println!("  Local copy already uses split layout, skipping.");
        }
    }

    for (remote, backend) in remotes {
        // List direct children of the shoot directory (non-recursive, files and dirs).
        let output = Command::new("rclone")
            .args(["lsjson", remote])
            .stderr(Stdio::null())
            .output()
            .context("failed to list remote files")?;
        if !output.status.success() {
            anyhow::bail!("failed to list files on {}", remote);
        }
        let entries: Vec<RcloneLsEntry> = serde_json::from_slice(&output.stdout)
            .context("failed to parse rclone lsjson")?;

        // Collect root-level photo files and where they should go.
        // Dirs (raw/, jpeg/, previews/) are is_dir=true and filtered out.
        let to_move: Vec<(String, &'static str)> = entries.iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| {
                let ext = std::path::Path::new(&e.path)
                    .extension()?
                    .to_str()?
                    .to_ascii_lowercase();
                let subdir = match ext.as_str() {
                    "cr2"          => "raw",
                    "jpg" | "jpeg" => "jpeg",
                    _ => return None,
                };
                Some((e.path.clone(), subdir))
            })
            .collect();

        if to_move.is_empty() {
            println!("  Remote already uses split layout, skipping.");
            continue;
        }

        // Use per-file moveto so source and destination never overlap (rclone move
        // refuses when dst is a subdirectory of src). moveto is a server-side rename
        // on both OneDrive and B2, so no bytes are transferred.
        let total      = to_move.len();
        let to_move    = Arc::new(to_move);
        let next_idx   = Arc::new(AtomicUsize::new(0));
        let completed  = Arc::new(AtomicUsize::new(0));
        let first_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let handles: Vec<_> = (0..8usize.min(total)).map(|_| {
            let to_move   = Arc::clone(&to_move);
            let next_idx  = Arc::clone(&next_idx);
            let completed = Arc::clone(&completed);
            let first_err = Arc::clone(&first_err);
            let remote    = remote.to_string();
            let needs_b2  = backend.needs_b2_chunk_flag();
            thread::spawn(move || {
                loop {
                    if first_err.lock().unwrap().is_some() { break; }
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    if idx >= to_move.len() { break; }
                    let (filename, subdir) = &to_move[idx];
                    let src = format!("{}/{}", remote, filename);
                    let dst = format!("{}/{}/{}", remote, subdir, filename);
                    let mut args = vec!["moveto".to_string(), src, dst];
                    if needs_b2 { args.extend(["--b2-chunk-size".into(), "96M".into()]); }
                    match Command::new("rclone").args(&args)
                        .stdout(Stdio::null()).stderr(Stdio::piped()).output()
                    {
                        Ok(o) if o.status.success() => {
                            completed.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(o) => {
                            let mut err = first_err.lock().unwrap();
                            if err.is_none() {
                                *err = Some(format!("failed to move {}: {}",
                                    filename, String::from_utf8_lossy(&o.stderr).trim()));
                            }
                            break;
                        }
                        Err(e) => {
                            let mut err = first_err.lock().unwrap();
                            if err.is_none() { *err = Some(format!("failed to spawn rclone: {e}")); }
                            break;
                        }
                    }
                }
            })
        }).collect();

        loop {
            let done   = completed.load(Ordering::Relaxed);
            let pct    = done * 100 / total;
            let filled = done * 20  / total;
            let bar    = format!("[{}{}]", "=".repeat(filled), " ".repeat(20 - filled));
            print!("\r  {} {:>3}%  ({}/{})", bar, pct, done, total);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            if done >= total || first_err.lock().unwrap().is_some() { break; }
            thread::sleep(Duration::from_millis(100));
        }
        for handle in handles { handle.join().ok(); }
        if let Some(err) = first_err.lock().unwrap().take() {
            println!();
            anyhow::bail!("{}", err);
        }
        println!("\r{:<60}", format!("  {} files moved.", total));
        any_work = true;
    }

    if !any_work {
        println!("  Shoot is already using split structure everywhere.");
    }

    Ok(())
}

pub fn download_shoot(
    remote_path: &str,
    local: &Path,
    filter: DownloadFilter,
    backend: &Backend,
) -> Result<()> {
    let local_str = local.to_str().unwrap().to_string();

    let mut args = vec![
        "copy".to_string(),
        remote_path.to_string(),
        local_str,
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
        "--exclude".to_string(), "shoot.json".to_string(),
        "--exclude".to_string(), "previews/**".to_string(),
    ];

    if backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }

    match filter {
        DownloadFilter::RawOnly => {
            args.extend(["--include".into(), "*.CR2".into(), "--include".into(), "*.cr2".into()]);
        }
        DownloadFilter::JpegOnly => {
            args.extend([
                "--include".into(), "*.jpg".into(),
                "--include".into(), "*.JPG".into(),
                "--include".into(), "*.jpeg".into(),
                "--include".into(), "*.JPEG".into(),
            ]);
        }
        DownloadFilter::Both => {}
    }

    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone copy")?;

    if !status.success() {
        anyhow::bail!("rclone copy failed");
    }
    Ok(())
}

/// Copies all files from one remote path to another. Use dst_backend for upload flags.
/// Pass exclude_previews=true when archiving to cold storage (previews aren't useful there).
pub fn copy_remote_to_remote(
    src: &str,
    dst: &str,
    dst_backend: &Backend,
    exclude_previews: bool,
) -> Result<()> {
    let mut args = vec![
        "copy".to_string(),
        src.to_string(),
        dst.to_string(),
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
    ];
    if dst_backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }
    if exclude_previews {
        args.extend(["--exclude".into(), "previews/**".into()]);
    }
    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone copy")?;

    if !status.success() {
        anyhow::bail!("rclone copy failed");
    }
    Ok(())
}

/// Syncs local shoots directory up to a remote.
pub fn sync_up(local: &Path, remote: &str, backend: &Backend) -> Result<()> {
    let mut args = vec![
        "copy".to_string(),
        local.to_str().unwrap().to_string(),
        remote.to_string(),
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
    ];
    if backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }
    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone copy failed");
    }
    Ok(())
}

pub fn sync_lightroom_up(local: &Path, remote: &str, backend: &Backend) -> Result<()> {
    let mut args = vec![
        "sync".to_string(),
        local.to_str().unwrap().to_string(),
        remote.to_string(),
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
    ];
    if backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }
    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone sync failed");
    }
    Ok(())
}

pub fn sync_lightroom_down(remote: &str, local: &Path, backend: &Backend) -> Result<()> {
    let mut args = vec![
        "sync".to_string(),
        remote.to_string(),
        local.to_str().unwrap().to_string(),
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
    ];
    if backend.needs_b2_chunk_flag() {
        args.extend(["--b2-chunk-size".into(), "96M".into()]);
    }
    let status = Command::new("rclone")
        .args(&args)
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone sync failed");
    }
    Ok(())
}
