use std::env;
use std::path::PathBuf;

pub struct Config {
    pub photosets_remote: String,
    pub lightroom_remote: String,
    pub local_photosets: PathBuf,
    pub local_lightroom: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let _ = dotenvy::dotenv();
        let home = dirs::home_dir().expect("cannot find home directory");
        Self {
            photosets_remote: env::var("B2_PHOTOSETS_REMOTE")
                .expect("B2_PHOTOSETS_REMOTE must be set in .env or environment"),
            lightroom_remote: env::var("B2_LIGHTROOM_REMOTE")
                .expect("B2_LIGHTROOM_REMOTE must be set in .env or environment"),
            local_photosets: env::var("LOCAL_PHOTOSETS")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join("Pictures/Photosets")),
            local_lightroom: env::var("LOCAL_LIGHTROOM")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join("Pictures/Lightroom")),
        }
    }
}
