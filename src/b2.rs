use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Command, Stdio};

use crate::config::Config;

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
    pub remote_path: String,
    pub size_bytes: Option<u64>,
    pub metadata: Option<Metadata>,
}

impl Shoot {
    pub fn display_name(&self) -> String {
        let meta = self.metadata.as_ref().map(|m| {
            let parts: Vec<&str> = [m.model.as_str(), m.location.as_str()]
                .iter()
                .copied()
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(" · ")
        });

        match (meta.as_deref(), self.size_bytes) {
            (Some(m), Some(b)) if !m.is_empty() => format!("{}  {}  ({})", self.name, m, format_bytes(b)),
            (Some(m), None) if !m.is_empty()    => format!("{}  {}", self.name, m),
            (_, Some(b))                         => format!("{}  ({})", self.name, format_bytes(b)),
            _                                    => self.name.clone(),
        }
    }

    pub fn local_path(&self, config: &Config) -> std::path::PathBuf {
        config.local_pictures.join(&self.year).join(&self.name)
    }

    pub fn previews_remote(&self) -> String {
        format!("{}/previews", self.remote_path)
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

pub fn list_shoots(config: &Config) -> Result<Vec<Shoot>> {
    let output = Command::new("rclone")
        .args(["lsjson", "--dirs-only", "--recursive", &config.pictures_remote])
        .output()
        .context("failed to run rclone lsjson")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("rclone lsjson failed: {}", stderr);
    }

    let entries: Vec<RcloneLsEntry> = serde_json::from_slice(&output.stdout)
        .context("failed to parse rclone lsjson output")?;

    // Keep only depth-2 paths matching "YYYY/YYYY-MM-DD"
    let mut shoots: Vec<Shoot> = entries
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
            let remote_path = format!("{}/{}", config.pictures_remote, e.path);
            Shoot {
                name: name.to_string(),
                year: year.to_string(),
                remote_path,
                size_bytes: None,
                metadata: None,
            }
        })
        .collect();

    shoots.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(shoots)
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
    let tmp = std::env::temp_dir().join("photo-archive-shoot.json");
    std::fs::write(&tmp, json)?;

    let dest = format!("{}/shoot.json", remote_path);
    let status = Command::new("rclone")
        .args(["copyto", tmp.to_str().unwrap(), &dest])
        .status()
        .context("failed to run rclone copyto")?;

    if !status.success() {
        anyhow::bail!("failed to save metadata to B2");
    }
    Ok(())
}

/// Generates JPEG previews from local shoot files and uploads them to B2.
/// Prefers JPEG source over CR2 for the same filename stem to avoid RAW decode overhead.
pub fn generate_and_upload_previews(shoot: &Shoot, config: &Config) -> Result<()> {
    let local = shoot.local_path(config);
    if !local.exists() {
        anyhow::bail!("shoot is not downloaded locally — download it first");
    }

    let preview_dir = std::env::temp_dir()
        .join("photo-archive-previews")
        .join(&shoot.name);
    std::fs::create_dir_all(&preview_dir)?;

    // Collect source files, preferring JPEG over CR2 per stem
    let mut sources: HashMap<String, std::path::PathBuf> = HashMap::new();
    for entry in std::fs::read_dir(&local)? {
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
        // Insert if not present, or upgrade CR2 entry to JPEG
        if is_jpeg || !sources.contains_key(&stem) {
            sources.insert(stem, path);
        }
    }

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

    // Upload previews to B2
    println!("Uploading previews to B2...");
    let status = Command::new("rclone")
        .args([
            "copy",
            preview_dir.to_str().unwrap(),
            &shoot.previews_remote(),
            "--progress",
            "--transfers", "4",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("failed to upload previews");
    }

    Ok(())
}

pub fn previews_exist(shoot: &Shoot) -> bool {
    Command::new("rclone")
        .args(["ls", &shoot.previews_remote()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Downloads previews from B2 to a local temp directory and opens it in Finder.
pub fn browse_previews(shoot: &Shoot) -> Result<()> {
    // Check previews exist on B2
    let check = Command::new("rclone")
        .args(["ls", &shoot.previews_remote()])
        .stderr(Stdio::null())
        .output()
        .context("failed to run rclone ls")?;

    if check.stdout.is_empty() {
        anyhow::bail!("no previews found for this shoot — generate them first");
    }

    let preview_dir = std::env::temp_dir()
        .join("photo-archive-previews")
        .join(&shoot.name);
    std::fs::create_dir_all(&preview_dir)?;

    println!("Downloading previews...");
    let status = Command::new("rclone")
        .args([
            "copy",
            &shoot.previews_remote(),
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

/// Verifies that every local file in the shoot exists on B2 with a matching checksum.
/// Uses --one-way so we check local→B2 only. B2 is never written to or deleted from here.
pub fn verify_local_synced(shoot: &Shoot, config: &Config) -> Result<bool> {
    let local = shoot.local_path(config);
    if !local.exists() {
        return Ok(false);
    }

    let output = Command::new("rclone")
        .args([
            "check",
            local.to_str().unwrap(),
            &shoot.remote_path,
            "--one-way",
            "--exclude", "shoot.json",
            "--exclude", "previews/**",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("failed to run rclone check")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Surface any meaningful error lines (rclone prints differences to stderr)
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

/// Permanently deletes a shoot from B2. Local files are not touched.
pub fn delete_from_b2(shoot: &Shoot) -> Result<()> {
    let status = Command::new("rclone")
        .args(["purge", &shoot.remote_path])
        .status()
        .context("failed to run rclone purge")?;

    if !status.success() {
        anyhow::bail!("rclone purge failed");
    }
    Ok(())
}

/// Deletes the local copy of a shoot using the filesystem only. No rclone involved.
pub fn purge_local(shoot: &Shoot, config: &Config) -> Result<()> {
    let local = shoot.local_path(config);
    std::fs::remove_dir_all(&local)
        .with_context(|| format!("failed to delete {}", local.display()))
}

pub enum LocalStatus {
    NotDownloaded,
    Synced,
    OutOfSync,
}

pub fn check_local_status(shoot: &Shoot, config: &Config) -> LocalStatus {
    let local = shoot.local_path(config);
    if !local.exists() {
        return LocalStatus::NotDownloaded;
    }

    let status = Command::new("rclone")
        .args([
            "check",
            "--one-way",
            "--quiet",
            &shoot.remote_path,
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

pub fn download_shoot(shoot: &Shoot, config: &Config, filter: DownloadFilter) -> Result<()> {
    let local = shoot.local_path(config);
    let local_str = local.to_str().unwrap().to_string();

    let mut args = vec![
        "copy".to_string(),
        shoot.remote_path.clone(),
        local_str,
        "--progress".to_string(),
        "--transfers".to_string(), "4".to_string(),
        "--b2-chunk-size".to_string(), "96M".to_string(),
        "--exclude".to_string(), "shoot.json".to_string(),
        "--exclude".to_string(), "previews/**".to_string(),
    ];

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

pub fn sync_photos_up(config: &Config) -> Result<()> {
    let status = Command::new("rclone")
        .args([
            "copy",
            config.local_pictures.to_str().unwrap(),
            &config.pictures_remote,
            "--progress",
            "--transfers", "4",
            "--b2-chunk-size", "96M",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone copy failed");
    }
    Ok(())
}

pub fn sync_lightroom_up(config: &Config) -> Result<()> {
    let status = Command::new("rclone")
        .args([
            "sync",
            config.local_lightroom.to_str().unwrap(),
            &config.lightroom_remote,
            "--progress",
            "--transfers", "4",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone sync failed");
    }
    Ok(())
}

pub fn sync_lightroom_down(config: &Config) -> Result<()> {
    let status = Command::new("rclone")
        .args([
            "sync",
            &config.lightroom_remote,
            config.local_lightroom.to_str().unwrap(),
            "--progress",
            "--transfers", "4",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone sync failed");
    }
    Ok(())
}
