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
                "Generate missing previews",
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
            "Generate missing previews" => {
                if let Err(e) = generate_missing_previews(config) {
                    eprintln!("Error: {e}");
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
    // Display shoot info and metadata
    println!();
    println!("  {}", shoot.name);
    if let Some(m) = &shoot.metadata {
        if !m.model.is_empty()    { println!("  Model:    {}", m.model); }
        if !m.location.is_empty() { println!("  Location: {}", m.location); }
        if !m.notes.is_empty()    { println!("  Notes:    {}", m.notes); }
    }
    println!();

    loop {
        let choice = Select::new(
            "Options:",
            vec![
                "Download",
                "View previews",
                "Generate previews",
                "Edit metadata",
                "← Back",
            ],
        )
        .prompt()?;

        match choice {
            "Download" => {
                if let Err(e) = download_menu(config, shoot) {
                    eprintln!("Error: {e}");
                }
            }
            "View previews" => {
                if let Err(e) = b2::browse_previews(shoot) {
                    eprintln!("Error: {e}");
                }
            }
            "Generate previews" => {
                match b2::generate_and_upload_previews(shoot, config) {
                    Ok(_) => println!("Previews uploaded."),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            "Edit metadata" => {
                if let Ok(meta) = edit_metadata(shoot) {
                    return Ok(Some(meta));
                }
            }
            _ => break,
        }
    }

    Ok(None)
}

fn download_menu(config: &Config, shoot: &b2::Shoot) -> Result<()> {
    let status = b2::check_local_status(shoot, config);
    let status_str = match &status {
        b2::LocalStatus::NotDownloaded => "not downloaded",
        b2::LocalStatus::Synced        => "synced locally",
        b2::LocalStatus::OutOfSync     => "out of sync",
    };
    println!("  Local status: {}", status_str);

    if let b2::LocalStatus::Synced = status {
        let proceed = Confirm::new("Already synced. Download again anyway?")
            .with_default(false)
            .prompt()?;
        if !proceed { return Ok(()); }
    }

    let choice = Select::new(
        "Download:",
        vec!["RAW only (.CR2)", "JPEG only (.jpg)", "Both", "← Back"],
    )
    .prompt()?;

    let filter = match choice {
        "RAW only (.CR2)"  => b2::DownloadFilter::RawOnly,
        "JPEG only (.jpg)" => b2::DownloadFilter::JpegOnly,
        "Both"             => b2::DownloadFilter::Both,
        _                  => return Ok(()),
    };

    let local = shoot.local_path(config);
    println!("Downloading to: {}", local.display());
    b2::download_shoot(shoot, config, filter)?;
    println!("Download complete.");
    Ok(())
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

fn generate_missing_previews(config: &Config) -> Result<()> {
    println!("Fetching shoot list...");
    let shoots = b2::list_shoots(config)?;

    if shoots.is_empty() {
        println!("No photoshoots found.");
        return Ok(());
    }

    // Check which shoots are missing previews and have local files to generate from
    println!("Checking for missing previews...");
    let missing: Vec<&b2::Shoot> = shoots
        .iter()
        .filter(|s| {
            let has_local = s.local_path(config).exists();
            let has_previews = b2::previews_exist(s);
            has_local && !has_previews
        })
        .collect();

    if missing.is_empty() {
        println!("All locally available shoots already have previews.");
        return Ok(());
    }

    println!("{} shoot(s) need previews:", missing.len());
    for s in &missing {
        println!("  {}", s.name);
    }

    for (i, shoot) in missing.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, missing.len(), shoot.name);
        match b2::generate_and_upload_previews(shoot, config) {
            Ok(_) => println!("Previews uploaded."),
            Err(e) => eprintln!("  Failed: {e}"),
        }
    }

    println!("\nDone.");
    Ok(())
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
