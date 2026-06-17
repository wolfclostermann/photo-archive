mod config;
mod importer;
mod menu;
mod scanner;
mod storage;

use anyhow::Result;
use config::{Config, Tier};
use inquire::{Confirm, MultiSelect, Text};
use std::thread;

fn main() -> Result<()> {
    let config = Config::load();
    ensure_hot_dirs(&config)?;
    main_menu(&config)
}

fn ensure_hot_dirs(config: &Config) -> Result<()> {
    let Some(hot) = &config.hot else { return Ok(()) };

    if !storage::remote_exists(&hot.shoots_remote) {
        println!(
            "Hot storage directory does not exist: {}",
            hot.shoots_remote
        );
        let create = Confirm::new(&format!(
            "Create it on {}?",
            hot.backend.label()
        ))
        .with_default(true)
        .prompt()?;
        if create {
            storage::create_remote_dir(&hot.shoots_remote)?;
            println!("Created.");
        } else {
            anyhow::bail!("hot storage directory is required to continue");
        }
    }

    Ok(())
}

fn main_menu(config: &Config) -> Result<()> {
    loop {
        let choice = match menu::select(
            "Photoshoot Manager",
            &[
                "Browse & download photoshoots",
                "Import from card",
                "Sync all photos up",
                "Generate missing previews",
                "Purge local copies",
                "Lightroom library sync",
                "Quit",
            ],
        )? {
            menu::Choice::Quit => break,
            menu::Choice::Back => continue,
            menu::Choice::Select(s) => s,
        };

        match choice.as_str() {
            "Browse & download photoshoots" => {
                if let Err(e) = browse_menu(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Import from card" => {
                if let Err(e) = import_from_card(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Sync all photos up" => {
                if let Err(e) = sync_all_up(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Generate missing previews" => {
                if let Err(e) = generate_missing_previews(config) {
                    eprintln!("Error: {e}");
                }
            }
            "Purge local copies" => {
                if let Err(e) = purge_local_menu(config) {
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

fn list_all_shoots(config: &Config) -> Result<Vec<storage::Shoot>> {
    let hot_shoots = match &config.hot {
        Some(bc) => storage::list_shoots_from(&bc.shoots_remote, &Tier::Hot)?,
        None => vec![],
    };
    let cold_shoots = match &config.cold {
        Some(bc) => storage::list_shoots_from(&bc.shoots_remote, &Tier::Cold)?,
        None => vec![],
    };
    Ok(storage::merge_shoot_lists(hot_shoots, cold_shoots))
}

fn browse_menu(config: &Config) -> Result<()> {
    println!("Fetching photoshoots...");
    let mut shoots = list_all_shoots(config)?;

    if shoots.is_empty() {
        println!("No photoshoots found.");
        return Ok(());
    }

    println!("Fetching sizes and metadata...");
    let size_handles: Vec<_> = shoots
        .iter()
        .map(|s| {
            let path = s.active_remote().map(|r| r.to_string());
            thread::spawn(move || path.and_then(|p| storage::fetch_shoot_size(&p).ok()))
        })
        .collect();

    let meta_handles: Vec<_> = shoots
        .iter()
        .map(|s| {
            let path = s.active_remote().map(|r| r.to_string());
            thread::spawn(move || path.and_then(|p| storage::fetch_metadata(&p)))
        })
        .collect();

    for (shoot, handle) in shoots.iter_mut().zip(size_handles) {
        shoot.size_bytes = handle.join().ok().flatten();
    }
    for (shoot, handle) in shoots.iter_mut().zip(meta_handles) {
        shoot.metadata = handle.join().ok().flatten();
    }

    loop {
        let mut options: Vec<String> = shoots.iter().map(|s| s.display_name()).collect();
        options.push("← Back".into());

        let choice = match menu::select("Select a photoshoot:", &options)? {
            menu::Choice::Quit | menu::Choice::Back => break,
            menu::Choice::Select(s) if s == "← Back" => break,
            menu::Choice::Select(s) => s,
        };

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
fn shoot_menu(config: &Config, shoot: &storage::Shoot) -> Result<Option<storage::Metadata>> {
    println!();
    println!("  {}", shoot.name);
    if let Some(m) = &shoot.metadata {
        if !m.model.is_empty()    { println!("  Model:    {}", m.model); }
        if !m.location.is_empty() { println!("  Location: {}", m.location); }
        if !m.notes.is_empty()    { println!("  Notes:    {}", m.notes); }
    }
    let tier_info = match (&shoot.hot_path, &shoot.cold_path) {
        (Some(_), Some(_)) => format!(
            "Hot ({}) + Cold ({})",
            config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("?"),
            config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("?"),
        ),
        (Some(_), None) => format!(
            "Hot ({}) only",
            config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("?"),
        ),
        (None, Some(_)) => format!(
            "Cold ({}) only",
            config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("?"),
        ),
        (None, None) => "Not archived".into(),
    };
    println!("  Storage:  {}", tier_info);
    println!();

    let hot_label  = config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("Hot");
    let cold_label = config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("Cold");

    let in_hot  = shoot.hot_path.is_some()  && config.hot.is_some();
    let in_cold = shoot.cold_path.is_some() && config.cold.is_some();

    loop {
        let mut options: Vec<String> = vec![
            "Download".into(),
            "View previews".into(),
            "Generate previews".into(),
            "Purge local copy".into(),
            "Edit metadata".into(),
        ];

        if config.cold.is_some() && !in_cold {
            options.push(format!("Archive to cold storage ({})", cold_label));
        }
        if config.hot.is_some() && !in_hot {
            options.push(format!("Restore to hot storage ({})", hot_label));
        }
        if in_hot && in_cold {
            options.push(format!("Remove from hot storage ({})", hot_label));
        }

        options.push("Migrate to split structure".into());
        options.push("Delete".into());
        options.push("← Back".into());

        let choice = match menu::select("Options:", &options)? {
            menu::Choice::Quit | menu::Choice::Back => break,
            menu::Choice::Select(s) => s,
        };

        match choice.as_str() {
            "Download" => {
                if let Err(e) = download_menu(config, shoot) {
                    eprintln!("Error: {e}");
                }
            }
            "View previews" => {
                if let Some(pr) = shoot.previews_remote() {
                    if let Err(e) = storage::browse_previews(&shoot.name, &pr) {
                        eprintln!("Error: {e}");
                    }
                } else {
                    println!("  Shoot has no remote path.");
                }
            }
            "Generate previews" => {
                let local = shoot.local_path(&config.local_shoots);
                if let (Some(active), Some(backend)) = (shoot.active_remote(), shoot.active_backend(config)) {
                    let upload_remote = format!("{}/previews", active);
                    match storage::generate_and_upload_previews(shoot, &local, &upload_remote, backend) {
                        Ok(_) => println!("Previews uploaded."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                } else {
                    println!("  Shoot has no remote path.");
                }
            }
            "Purge local copy" => {
                if let Err(e) = purge_single(config, shoot) {
                    eprintln!("Error: {e}");
                }
            }
            "Edit metadata" => {
                match edit_metadata(shoot) {
                    Ok(meta) => return Ok(Some(meta)),
                    Err(e) => eprintln!("Error saving metadata: {e}"),
                }
            }
            "Migrate to split structure" => {
                let local_path = shoot.local_path(&config.local_shoots);
                let local = if local_path.exists() { Some(local_path.as_path()) } else { None };
                let hot_pair  = shoot.hot_path.as_deref()
                    .zip(config.hot.as_ref().map(|bc| &bc.backend));
                let cold_pair = shoot.cold_path.as_deref()
                    .zip(config.cold.as_ref().map(|bc| &bc.backend));
                let mut remotes: Vec<(&str, &config::Backend)> = vec![];
                if let Some(p) = hot_pair  { remotes.push(p); }
                if let Some(p) = cold_pair { remotes.push(p); }
                match storage::migrate_shoot_to_split(local, &remotes) {
                    Ok(_)  => println!("Done."),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            "Delete" => {
                if shoot_delete_menu(config, shoot)? {
                    break; // shoot gone, exit menu
                }
            }
            s if s.starts_with("Archive to cold storage") => {
                if let (Some(hot_path), Some(cold_bc)) = (&shoot.hot_path, &config.cold) {
                    let cold_path = format!(
                        "{}/{}/{}",
                        cold_bc.shoots_remote, shoot.year, shoot.name
                    );
                    println!("Archiving {} to {}...", shoot.name, cold_bc.backend.label());
                    match storage::copy_remote_to_remote(hot_path, &cold_path, &cold_bc.backend, true) {
                        Ok(_) => println!("Archived. Shoot is now in both hot and cold storage."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
            }
            s if s.starts_with("Restore to hot storage") => {
                if let (Some(cold_path), Some(hot_bc)) = (&shoot.cold_path, &config.hot) {
                    let hot_path = format!(
                        "{}/{}/{}",
                        hot_bc.shoots_remote, shoot.year, shoot.name
                    );
                    println!("Restoring {} to {}...", shoot.name, hot_bc.backend.label());
                    match storage::copy_remote_to_remote(cold_path, &hot_path, &hot_bc.backend, false) {
                        Ok(_) => println!("Restored. Shoot is now in both hot and cold storage."),
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
            }
            s if s.starts_with("Remove from hot storage") => {
                if let (Some(hot_path), Some(cold_path)) = (&shoot.hot_path, &shoot.cold_path) {
                    println!("  Verifying {} is fully archived in cold storage...", shoot.name);
                    match storage::verify_remote_synced(hot_path, cold_path) {
                        Ok(true) => {
                            let confirm = Confirm::new(&format!(
                                "Remove {} from {}? Files remain in cold storage.",
                                shoot.name, hot_label
                            ))
                            .with_default(false)
                            .prompt()?;
                            if confirm {
                                match storage::delete_remote(hot_path) {
                                    Ok(_) => {
                                        println!("Removed from {}.", hot_label);
                                        break; // shoot presence changed, exit menu
                                    }
                                    Err(e) => eprintln!("Error: {e}"),
                                }
                            }
                        }
                        Ok(false) => {
                            println!("  Not fully archived to cold storage — archive it first before removing from hot.");
                        }
                        Err(e) => eprintln!("  Verification failed: {e}"),
                    }
                }
            }
            _ => break,
        }
    }

    Ok(None)
}

fn shoot_delete_menu(config: &Config, shoot: &storage::Shoot) -> Result<bool> {
    let in_hot  = shoot.hot_path.is_some()  && config.hot.is_some();
    let in_cold = shoot.cold_path.is_some() && config.cold.is_some();

    let hot_label  = config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("Hot");
    let cold_label = config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("Cold");

    if in_hot && in_cold {
        let options = vec![
            format!("Delete from {} (hot)", hot_label),
            format!("Delete from {} (cold)", cold_label),
            "Delete from both".to_string(),
            "← Back".to_string(),
        ];
        let choice = match menu::select("Delete from:", &options)? {
            menu::Choice::Quit | menu::Choice::Back => return Ok(false),
            menu::Choice::Select(s) if s == "← Back" => return Ok(false),
            menu::Choice::Select(s) => s,
        };

        match choice.as_str() {
            s if s.starts_with(&format!("Delete from {} (hot)", hot_label)) => {
                if confirm_delete(shoot, hot_label)? {
                    storage::delete_remote(shoot.hot_path.as_deref().unwrap())?;
                    println!("  {} deleted from {}.", shoot.name, hot_label);
                }
            }
            s if s.starts_with(&format!("Delete from {} (cold)", cold_label)) => {
                if confirm_delete(shoot, cold_label)? {
                    storage::delete_remote(shoot.cold_path.as_deref().unwrap())?;
                    println!("  {} deleted from {}.", shoot.name, cold_label);
                }
            }
            "Delete from both" => {
                println!("\n  WARNING: This permanently deletes {} from ALL storage.", shoot.name);
                let first = Confirm::new("Are you sure you want to delete from both hot and cold?")
                    .with_default(false)
                    .prompt()?;
                if first {
                    let second = Confirm::new(&format!(
                        "Final confirmation — permanently delete {} from all storage?",
                        shoot.name
                    ))
                    .with_default(false)
                    .prompt()?;
                    if second {
                        if let Some(path) = &shoot.hot_path {
                            storage::delete_remote(path)?;
                        }
                        if let Some(path) = &shoot.cold_path {
                            storage::delete_remote(path)?;
                        }
                        println!("  {} deleted from all storage.", shoot.name);
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
    } else {
        let tier_label = if in_hot { hot_label } else { cold_label };
        let remote = shoot.hot_path.as_deref().or(shoot.cold_path.as_deref()).unwrap();

        println!("\n  WARNING: This permanently deletes {} from {}.", shoot.name, tier_label);
        println!("  Local files are not affected.");
        if confirm_delete(shoot, tier_label)? {
            storage::delete_remote(remote)?;
            println!("  {} deleted from {}.", shoot.name, tier_label);
            return Ok(true);
        }
    }

    Ok(false)
}

fn confirm_delete(shoot: &storage::Shoot, tier_label: &str) -> Result<bool> {
    let first = Confirm::new(&format!(
        "Are you sure you want to delete {} from {}?",
        shoot.name, tier_label
    ))
    .with_default(false)
    .prompt()?;
    if !first { return Ok(false); }

    let second = Confirm::new(&format!(
        "Final confirmation — permanently delete {} from {}?",
        shoot.name, tier_label
    ))
    .with_default(false)
    .prompt()?;
    Ok(second)
}

fn download_menu(config: &Config, shoot: &storage::Shoot) -> Result<()> {
    // Prefer hot, fall back to cold
    let (remote_path, backend) = if let (Some(path), Some(bc)) = (&shoot.hot_path, &config.hot) {
        (path.as_str(), &bc.backend)
    } else if let (Some(path), Some(bc)) = (&shoot.cold_path, &config.cold) {
        println!("  Note: shoot is not in hot storage — downloading from cold ({}).", bc.backend.label());
        (path.as_str(), &bc.backend)
    } else {
        anyhow::bail!("shoot has no remote path");
    };

    let local = shoot.local_path(&config.local_shoots);
    let status = storage::check_local_status(&local, remote_path);
    let status_str = match &status {
        storage::LocalStatus::NotDownloaded => "not downloaded",
        storage::LocalStatus::Synced        => "synced locally",
        storage::LocalStatus::OutOfSync     => "out of sync",
    };
    println!("  Local status: {}", status_str);

    if let storage::LocalStatus::Synced = status {
        let proceed = Confirm::new("Already synced. Download again anyway?")
            .with_default(false)
            .prompt()?;
        if !proceed { return Ok(()); }
    }

    let choice = match menu::select(
        "Download:",
        &["RAW only (.CR2)", "JPEG only (.jpg)", "Both", "← Back"],
    )? {
        menu::Choice::Quit | menu::Choice::Back => return Ok(()),
        menu::Choice::Select(s) => s,
    };

    let filter = match choice.as_str() {
        "RAW only (.CR2)"  => storage::DownloadFilter::RawOnly,
        "JPEG only (.jpg)" => storage::DownloadFilter::JpegOnly,
        "Both"             => storage::DownloadFilter::Both,
        _                  => return Ok(()),
    };

    println!("Downloading to: {}", local.display());
    storage::download_shoot(remote_path, &local, filter, backend)?;
    println!("Download complete.");
    Ok(())
}

fn edit_metadata(shoot: &storage::Shoot) -> Result<storage::Metadata> {
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

    let metadata = storage::Metadata { model, location, notes };

    // Write to all remotes the shoot exists on
    if let Some(path) = &shoot.hot_path {
        storage::save_metadata(path, &metadata)?;
    }
    if let Some(path) = &shoot.cold_path {
        storage::save_metadata(path, &metadata)?;
    }
    println!("Metadata saved.");

    Ok(metadata)
}

fn sync_all_up(config: &Config) -> Result<()> {
    let mut any_error = false;

    if let Some(hot) = &config.hot {
        println!("Syncing to {} (hot)...", hot.backend.label());
        if let Err(e) = storage::sync_up(&config.local_shoots, &hot.shoots_remote, &hot.backend) {
            eprintln!("Error syncing to {}: {e}", hot.backend.label());
            any_error = true;
        }
    }

    if let Some(cold) = &config.cold {
        println!("Syncing to {} (cold)...", cold.backend.label());
        if let Err(e) = storage::sync_up(&config.local_shoots, &cold.shoots_remote, &cold.backend) {
            eprintln!("Error syncing to {}: {e}", cold.backend.label());
            any_error = true;
        }
    }

    if !any_error {
        println!("Sync complete.");
    }
    Ok(())
}

fn generate_missing_previews(config: &Config) -> Result<()> {
    println!("Fetching shoot list...");
    let shoots = list_all_shoots(config)?;

    if shoots.is_empty() {
        println!("No photoshoots found.");
        return Ok(());
    }

    println!("Checking for missing previews...");
    let missing: Vec<&storage::Shoot> = shoots
        .iter()
        .filter(|s| {
            let has_local = s.local_path(&config.local_shoots).exists();
            let has_previews = s.previews_remote()
                .map(|pr| storage::previews_exist(&pr))
                .unwrap_or(false);
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
        let local = shoot.local_path(&config.local_shoots);
        if let (Some(active), Some(backend)) = (shoot.active_remote(), shoot.active_backend(config)) {
            let upload_remote = format!("{}/previews", active);
            match storage::generate_and_upload_previews(shoot, &local, &upload_remote, backend) {
                Ok(_) => println!("Previews uploaded."),
                Err(e) => eprintln!("  Failed: {e}"),
            }
        }
    }

    println!("\nDone.");
    Ok(())
}

fn purge_single(config: &Config, shoot: &storage::Shoot) -> Result<()> {
    let local = shoot.local_path(&config.local_shoots);
    if !local.exists() {
        println!("  {} is not downloaded locally.", shoot.name);
        return Ok(());
    }

    // Prefer verifying against cold (the permanent archive), fall back to hot
    let verify_remote = shoot.cold_path.as_deref().or_else(|| shoot.hot_path.as_deref());
    let remote = match verify_remote {
        Some(r) => r,
        None => anyhow::bail!("shoot has no remote path to verify against"),
    };

    let tier_label = if shoot.cold_path.is_some() {
        config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("cold")
    } else {
        config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("hot")
    };

    println!("  Verifying {} is fully synced to {}...", shoot.name, tier_label);
    match storage::verify_local_synced(&local, remote) {
        Ok(true) => {}
        Ok(false) => {
            anyhow::bail!("local copy is not fully synced to {} — upload it first before purging", tier_label);
        }
        Err(e) => {
            anyhow::bail!("sync check failed: {e}");
        }
    }

    println!("  Verified. Local path: {}", local.display());
    let confirm = Confirm::new("Delete local copy? This cannot be undone.")
        .with_default(false)
        .prompt()?;

    if confirm {
        storage::purge_local(&local)?;
        println!("  Deleted.");
    }
    Ok(())
}

fn purge_local_menu(config: &Config) -> Result<()> {
    println!("Fetching shoot list...");
    let shoots = list_all_shoots(config)?;

    let local_shoots: Vec<&storage::Shoot> = shoots
        .iter()
        .filter(|s| s.local_path(&config.local_shoots).exists())
        .collect();

    if local_shoots.is_empty() {
        println!("No shoots are downloaded locally.");
        return Ok(());
    }

    // Verify against cold first (permanent archive), fall back to hot
    let verify_label = if config.cold.is_some() {
        config.cold.as_ref().map(|bc| bc.backend.label()).unwrap_or("cold")
    } else {
        config.hot.as_ref().map(|bc| bc.backend.label()).unwrap_or("hot")
    };

    println!("Verifying {} local shoot(s) against {}...", local_shoots.len(), verify_label);

    let mut synced: Vec<&storage::Shoot> = vec![];
    let mut skipped = 0usize;

    for shoot in &local_shoots {
        print!("  {} ... ", shoot.name);
        let _ = std::io::Write::flush(&mut std::io::stdout());

        let remote = shoot.cold_path.as_deref().or_else(|| shoot.hot_path.as_deref());
        let result = match remote {
            Some(r) => storage::verify_local_synced(&shoot.local_path(&config.local_shoots), r),
            None => Ok(false),
        };

        match result {
            Ok(true) => {
                println!("synced");
                synced.push(shoot);
            }
            Ok(false) => {
                println!("NOT fully synced — skipping");
                skipped += 1;
            }
            Err(e) => {
                println!("check failed ({e}) — skipping");
                skipped += 1;
            }
        }
    }

    if synced.is_empty() {
        println!("\nNo fully-synced local shoots to purge.");
        return Ok(());
    }

    println!("\nThe following {} shoot(s) are fully synced:", synced.len());
    for s in &synced {
        println!("  {}", s.name);
    }
    if skipped > 0 {
        println!("  ({skipped} shoot(s) not fully synced will be left untouched)");
    }

    let confirm = Confirm::new(&format!(
        "Permanently delete {} local shoot(s)? This cannot be undone.",
        synced.len()
    ))
    .with_default(false)
    .prompt()?;

    if !confirm {
        return Ok(());
    }

    for shoot in &synced {
        print!("  Deleting {} ... ", shoot.name);
        match storage::purge_local(&shoot.local_path(&config.local_shoots)) {
            Ok(_) => println!("done"),
            Err(e) => println!("FAILED: {e}"),
        }
    }

    println!("Purge complete.");
    Ok(())
}

fn import_from_card(config: &Config) -> Result<()> {
    let all_drives = scanner::find_external_drives();
    let cards: Vec<_> = all_drives
        .into_iter()
        .filter(|d| d.path.join("DCIM").exists())
        .collect();

    if cards.is_empty() {
        println!("No camera cards found (no external drives with a DCIM folder).");
        return Ok(());
    }

    let drive = if cards.len() == 1 {
        println!("Found card: {}", cards[0].display_name());
        cards.into_iter().next().unwrap()
    } else {
        let names: Vec<String> = cards.iter().map(|d| d.display_name()).collect();
        let name = match menu::select("Select card:", &names)? {
            menu::Choice::Quit | menu::Choice::Back => return Ok(()),
            menu::Choice::Select(s) => s,
        };
        cards.into_iter().find(|d| d.display_name() == name).unwrap()
    };

    println!("Scanning {}...", drive.display_name());
    let shoots = match scanner::scan_for_shoots(&drive) {
        Ok(ps) => ps,
        Err(e) => {
            eprintln!("Error: {e}");
            return Ok(());
        }
    };

    if shoots.is_empty() {
        println!("No image files found on this card.");
        return Ok(());
    }

    const IMPORT_ALL: &str = "← Import all";
    let mut options: Vec<String> = vec![IMPORT_ALL.to_string()];
    options.extend(shoots.iter().map(|p| p.display_name()));

    let selections = MultiSelect::new("Select shoots to import:", options).prompt()?;

    let selected: Vec<&scanner::Shoot> = if selections.iter().any(|s| s == IMPORT_ALL) {
        shoots.iter().collect()
    } else {
        shoots
            .iter()
            .filter(|p| selections.contains(&p.display_name()))
            .collect()
    };

    if selected.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    println!("\nWill import:");
    for p in &selected {
        let dest = config.local_shoots
            .join(p.date.format("%Y").to_string())
            .join(p.date.format("%Y-%m-%d").to_string());
        println!("  {}  →  {}", p.date, dest.display());
    }
    println!();

    let confirm = Confirm::new("Proceed with import?")
        .with_default(true)
        .prompt()?;
    if !confirm {
        return Ok(());
    }

    let mut total_copied = 0usize;
    let mut total_skipped = 0usize;
    for shoot in &selected {
        println!("\n{}", shoot.display_name());
        match importer::import_shoot(shoot, &config.local_shoots) {
            Ok(result) => {
                println!(
                    "  Done — {} copied, {} already present.",
                    result.copied, result.skipped
                );
                total_copied += result.copied;
                total_skipped += result.skipped;
            }
            Err(e) => eprintln!("  Error: {e}"),
        }
    }

    println!("\nImport complete — {} copied, {} skipped.", total_copied, total_skipped);

    let delete = Confirm::new("Delete imported files from card?")
        .with_default(false)
        .prompt()?;
    if !delete {
        return Ok(());
    }

    println!("Verifying...");
    let mut any_missing = false;
    for shoot in &selected {
        let missing = importer::verify_import(shoot, &config.local_shoots)?;
        if !missing.is_empty() {
            eprintln!(
                "  {} file(s) failed to verify for {}:",
                missing.len(),
                shoot.date
            );
            for f in &missing {
                eprintln!("    {}", f.display());
            }
            any_missing = true;
        }
    }

    if any_missing {
        eprintln!("Deletion aborted — not all files verified.");
        return Ok(());
    }

    println!("Verified. Deleting from card...");
    for shoot in &selected {
        match importer::delete_from_card(shoot) {
            Ok(n) => println!("  Deleted {} file(s) for {}.", n, shoot.date),
            Err(e) => eprintln!("  Error deleting {}: {e}", shoot.date),
        }
    }
    println!("Done.");

    Ok(())
}

fn lightroom_menu(config: &Config) -> Result<()> {
    // Prefer hot for Lightroom (working library); fall back to cold if only cold is configured
    let bc = match config.hot.as_ref().or(config.cold.as_ref()) {
        Some(bc) => bc,
        None => {
            println!("No backend configured.");
            return Ok(());
        }
    };

    let tier = if config.hot.is_some() { "hot" } else { "cold" };

    loop {
        let choice = match menu::select(
            &format!("Lightroom Library Sync ({}, {}):", bc.backend.label(), tier),
            &[
                "Sync up  (local → remote)",
                "Sync down  (remote → local)",
                "← Back",
            ],
        )? {
            menu::Choice::Quit | menu::Choice::Back => break,
            menu::Choice::Select(s) => s,
        };

        match choice.as_str() {
            "Sync up  (local → remote)" => {
                println!("Syncing Lightroom library to {}...", bc.backend.label());
                match storage::sync_lightroom_up(&config.local_lightroom, &bc.lightroom_remote, &bc.backend) {
                    Ok(_) => println!("Done."),
                    Err(e) => eprintln!("Error: {e}"),
                }
            }
            "Sync down  (remote → local)" => {
                println!(
                    "Warning: this will overwrite your local Lightroom library with the {} version.",
                    bc.backend.label()
                );
                let confirm = Confirm::new("Are you sure?")
                    .with_default(false)
                    .prompt()?;
                if confirm {
                    match storage::sync_lightroom_down(&bc.lightroom_remote, &config.local_lightroom, &bc.backend) {
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
