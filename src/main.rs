mod b2;
mod config;

use anyhow::Result;
use config::Config;
use inquire::{Confirm, Select, Text};
use std::thread;

fn main() -> Result<()> {
    let config = Config::default();
    main_menu(&config)
}

fn main_menu(config: &Config) -> Result<()> {
    loop {
        let choice = Select::new(
            "Photo Archive Manager",
            vec![
                "Browse & download photoshoots",
                "Sync all photos to B2",
                "Lightroom library sync",
                "Quit",
            ],
        )
        .prompt()?;

        match choice {
            "Browse & download photoshoots" => {
                if let Err(e) = browse_menu(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Sync all photos to B2" => {
                if let Err(e) = b2::sync_photos_up(config) {
                    eprintln!("Error: {e}");
                } else {
                    println!("Sync complete.");
                }
            }
            "Lightroom library sync" => {
                if let Err(e) = lightroom_menu(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Quit" => break,
            _ => {}
        }
    }
    Ok(())
}

fn browse_menu(config: &Config) -> Result<()> {
    println!("Fetching photoshoots from B2...");
    let mut shoots = b2::list_shoots(config)?;

    if shoots.is_empty() {
        println!("No photoshoots found.");
        return Ok(());
    }

    // Fetch sizes and metadata in parallel
    println!("Fetching sizes and metadata...");
    let size_handles: Vec<_> = shoots
        .iter()
        .map(|s| {
            let path = s.remote_path.clone();
            thread::spawn(move || b2::fetch_shoot_size(&path))
        })
        .collect();

    let meta_handles: Vec<_> = shoots
        .iter()
        .map(|s| {
            let path = s.remote_path.clone();
            thread::spawn(move || b2::fetch_metadata(&path))
        })
        .collect();

    for (shoot, handle) in shoots.iter_mut().zip(size_handles) {
        shoot.size_bytes = handle.join().ok().and_then(|r| r.ok());
    }
    for (shoot, handle) in shoots.iter_mut().zip(meta_handles) {
        shoot.metadata = handle.join().ok().flatten();
    }

    loop {
        let mut options: Vec<String> = shoots.iter().map(|s| s.display_name()).collect();
        options.push("← Back".into());

        let choice = Select::new("Select a photoshoot:", options.clone()).prompt()?;

        if choice == "← Back" {
            break;
        }

        if let Some(idx) = shoots.iter().position(|s| s.display_name() == choice) {
            let updated = shoot_menu(config, &shoots[idx])?;
            if let Some(meta) = updated {
                shoots[idx].metadata = Some(meta);
            }
        }
    }

    Ok(())
}

/// Returns updated metadata if it was edited, so the list can reflect the change immediately.
fn shoot_menu(config: &Config, shoot: &b2::Shoot) -> Result<Option<b2::Metadata>> {
    let status = b2::check_local_status(shoot, config);
    let status_str = match &status {
        b2::LocalStatus::NotDownloaded => "not downloaded",
        b2::LocalStatus::Synced => "synced locally",
        b2::LocalStatus::OutOfSync => "out of sync",
    };

    println!("\n{} — {}", shoot.name, status_str);

    if let b2::LocalStatus::Synced = status {
        let proceed = Confirm::new("Already synced. Download again anyway?")
            .with_default(false)
            .prompt()?;
        if !proceed {
            let choice = Select::new(
                "Options:",
                vec!["Generate & upload previews", "Browse previews", "Edit metadata", "← Back"],
            )
            .prompt()?;
            match choice {
                "Generate & upload previews" => {
                    match b2::generate_and_upload_previews(shoot, config) {
                        Ok(_) => println!("Previews uploaded."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
                "Browse previews" => {
                    if let Err(e) = b2::browse_previews(shoot) {
                        eprintln!("Error: {e}");
                    }
                }
                "Edit metadata" => return edit_metadata(shoot).map(Some),
                _ => {}
            }
            return Ok(None);
        }
    }

    let choice = Select::new(
        "Options:",
        vec![
            "Download RAW only (.CR2)",
            "Download JPEG only (.jpg)",
            "Download both",
            "Generate & upload previews",
            "Browse previews",
            "Edit metadata",
            "← Back",
        ],
    )
    .prompt()?;

    match choice {
        "Generate & upload previews" => {
            match b2::generate_and_upload_previews(shoot, config) {
                Ok(_) => println!("Previews uploaded."),
                Err(e) => eprintln!("Error: {e}"),
            }
            return Ok(None);
        }
        "Browse previews" => {
            if let Err(e) = b2::browse_previews(shoot) {
                eprintln!("Error: {e}");
            }
            return Ok(None);
        }
        "Edit metadata" => return edit_metadata(shoot).map(Some),
        "← Back" => return Ok(None),
        _ => {}
    }

    let filter = match choice {
        "Download RAW only (.CR2)" => b2::DownloadFilter::RawOnly,
        "Download JPEG only (.jpg)" => b2::DownloadFilter::JpegOnly,
        _ => b2::DownloadFilter::Both,
    };

    let local = shoot.local_path(config);
    println!("Downloading to: {}", local.display());
    b2::download_shoot(shoot, config, filter)?;
    println!("Download complete.");

    Ok(None)
}

fn edit_metadata(shoot: &b2::Shoot) -> Result<b2::Metadata> {
    let existing = shoot.metadata.clone().unwrap_or_default();

    let model = Text::new("Model:")
        .with_initial_value(&existing.model)
        .prompt()?;

    let location = Text::new("Location:")
        .with_initial_value(&existing.location)
        .prompt()?;

    let notes = Text::new("Notes:")
        .with_initial_value(&existing.notes)
        .prompt()?;

    let metadata = b2::Metadata { model, location, notes };
    b2::save_metadata(&shoot.remote_path, &metadata)?;
    println!("Metadata saved.");

    Ok(metadata)
}

fn lightroom_menu(config: &Config) -> Result<()> {
    loop {
        let choice = Select::new(
            "Lightroom Library Sync:",
            vec![
                "Sync up to B2  (local → remote)",
                "Sync down from B2  (remote → local)",
                "← Back",
            ],
        )
        .prompt()?;

        match choice {
            "Sync up to B2  (local → remote)" => {
                println!("Syncing Lightroom library to B2...");
                match b2::sync_lightroom_up(config) {
                    Ok(_) => println!("Done."),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            "Sync down from B2  (remote → local)" => {
                println!("Warning: this will overwrite your local Lightroom library with the B2 version.");
                let confirm = Confirm::new("Are you sure?")
                    .with_default(false)
                    .prompt()?;
                if confirm {
                    match b2::sync_lightroom_down(config) {
                        Ok(_) => println!("Done."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
            }
            _ => break,
        }
    }
    Ok(())
}
