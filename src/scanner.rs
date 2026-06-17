use std::collections::HashMap;
use std::path::{Path, PathBuf};
use chrono::NaiveDate;
use walkdir::WalkDir;
use anyhow::Result;

const IMAGE_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "nef", "arw", "raf", "rw2", "dng", "orf",
    "jpg", "jpeg",
];

pub struct Drive {
    pub name: String,
    pub path: PathBuf,
}

impl Drive {
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.name, self.path.display())
    }
}

pub struct Shoot {
    pub date: NaiveDate,
    pub files: Vec<PathBuf>,
}

impl Shoot {
    pub fn shot_count(&self) -> usize {
        self.files
            .iter()
            .filter_map(|f| f.file_stem())
            .map(|s| s.to_string_lossy().to_uppercase())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    pub fn display_name(&self) -> String {
        let mut exts: Vec<String> = self
            .files
            .iter()
            .filter_map(|f| f.extension())
            .map(|e| e.to_string_lossy().to_uppercase())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        exts.sort();
        format!(
            "{}  —  {} shots  ({})",
            self.date,
            self.shot_count(),
            exts.join(", ")
        )
    }
}

/// Returns all external drives visible under /Volumes, excluding the boot volume.
pub fn find_external_drives() -> Vec<Drive> {
    let volumes = Path::new("/Volumes");
    if !volumes.exists() {
        return vec![];
    }

    let boot_dev = dev_id("/");

    let mut drives = vec![];
    if let Ok(entries) = std::fs::read_dir(volumes) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if boot_dev.is_some() && dev_id(&path) == boot_dev {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            drives.push(Drive { name, path });
        }
    }
    drives.sort_by(|a, b| a.name.cmp(&b.name));
    drives
}

fn dev_id(path: impl AsRef<Path>) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path.as_ref()).ok().map(|m| m.dev())
}

/// Scans a drive's DCIM tree and groups image files by date into shoots.
pub fn scan_for_shoots(drive: &Drive) -> Result<Vec<Shoot>> {
    let dcim = drive.path.join("DCIM");
    if !dcim.exists() {
        anyhow::bail!("No DCIM directory found on this drive");
    }

    let mut by_date: HashMap<NaiveDate, Vec<PathBuf>> = HashMap::new();

    for entry in WalkDir::new(&dcim).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path().to_path_buf();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        if let Some(date) = image_date(&path) {
            by_date.entry(date).or_default().push(path);
        }
    }

    if by_date.is_empty() {
        anyhow::bail!("No image files found in DCIM");
    }

    let mut shoots: Vec<Shoot> = by_date
        .into_iter()
        .map(|(date, mut files)| {
            files.sort();
            Shoot { date, files }
        })
        .collect();

    shoots.sort_by_key(|s| s.date);
    Ok(shoots)
}

fn image_date(path: &Path) -> Option<NaiveDate> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    let dt: chrono::DateTime<chrono::Local> = mtime.into();
    Some(dt.date_naive())
}
