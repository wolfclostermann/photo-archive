mod b2;
mod config;

use anyhow::Result;
use config::Config;
use inquire::{Confirm, Select};
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

    // Fetch all shoot sizes in parallel
    println!("Fetching sizes...");
    let handles: Vec<_> = shoots
        .iter()
        .map(|s| {
            let path = s.remote_path.clone();
            thread::spawn(move || b2::fetch_shoot_size(&path))
        })
        .collect();

    for (shoot, handle) in shoots.iter_mut().zip(handles) {
        shoot.size_bytes = handle.join().ok().and_then(|r| r.ok());
    }

    loop {
        let mut options: Vec<String> = shoots.iter().map(|s| s.display_name()).collect();
        options.push("← Back".into());

        let choice = Select::new("Select a photoshoot:", options.clone()).prompt()?;

        if choice == "← Back" {
            break;
        }

        if let Some(shoot) = shoots.iter().find(|s| s.display_name() == choice) {
            let shoot = shoot.clone();
            if let Err(e) = shoot_menu(config, &shoot) {
                eprintln!("Error: {e}");
            }
        }
    }

    Ok(())
}

fn shoot_menu(config: &Config, shoot: &b2::Shoot) -> Result<()> {
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
            return Ok(());
        }
    }

    let choice = Select::new(
        "Download options:",
        vec![
            "Download RAW only (.CR2)",
            "Download JPEG only (.jpg)",
            "Download both",
            "← Back",
        ],
    )
    .prompt()?;

    let filter = match choice {
        "Download RAW only (.CR2)" => b2::DownloadFilter::RawOnly,
        "Download JPEG only (.jpg)" => b2::DownloadFilter::JpegOnly,
        "Download both" => b2::DownloadFilter::Both,
        _ => return Ok(()),
    };

    let local = shoot.local_path(config);
    println!("Downloading to: {}", local.display());
    b2::download_shoot(shoot, config, filter)?;
    println!("Download complete.");

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
