use anyhow::{Context, Result};
use serde::Deserialize;
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

#[derive(Debug, Clone)]
pub struct Shoot {
    pub name: String,
    pub year: String,
    pub remote_path: String,
    pub size_bytes: Option<u64>,
}

impl Shoot {
    pub fn display_name(&self) -> String {
        match self.size_bytes {
            Some(bytes) => format!("{}  ({})", self.name, format_bytes(bytes)),
            None => self.name.clone(),
        }
    }

    pub fn local_path(&self, config: &Config) -> std::path::PathBuf {
        config.local_pictures.join(&self.year).join(&self.name)
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
        "--transfers".to_string(),
        "4".to_string(),
        "--b2-chunk-size".to_string(),
        "96M".to_string(),
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
            "--transfers",
            "4",
            "--b2-chunk-size",
            "96M",
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
            "--transfers",
            "4",
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
            "--transfers",
            "4",
        ])
        .status()
        .context("failed to run rclone")?;

    if !status.success() {
        anyhow::bail!("rclone sync failed");
    }
    Ok(())
}
