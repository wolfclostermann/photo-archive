use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum Tier {
    Hot,
    Cold,
}

#[derive(Debug, Clone)]
pub enum Backend {
    OneDrive,
    B2,
}

impl Backend {
    #[allow(dead_code)]
    pub fn tier(&self) -> Tier {
        match self {
            Backend::OneDrive => Tier::Hot,
            Backend::B2 => Tier::Cold,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Backend::OneDrive => "OneDrive",
            Backend::B2 => "Backblaze B2",
        }
    }

    pub fn needs_b2_chunk_flag(&self) -> bool {
        matches!(self, Backend::B2)
    }
}

#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub backend: Backend,
    pub shoots_remote: String,
    pub lightroom_remote: String,
}

pub struct Config {
    pub hot: Option<BackendConfig>,
    pub cold: Option<BackendConfig>,
    pub local_shoots: PathBuf,
    pub local_lightroom: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let _ = dotenvy::dotenv();
        let home = dirs::home_dir().expect("cannot find home directory");

        let hot = env::var("ONEDRIVE_REMOTE").ok().map(|shoots_remote| {
            let lightroom_remote = env::var("ONEDRIVE_LIGHTROOM_REMOTE")
                .expect("ONEDRIVE_LIGHTROOM_REMOTE must be set when ONEDRIVE_REMOTE is set");
            BackendConfig { backend: Backend::OneDrive, shoots_remote, lightroom_remote }
        });

        let cold = env::var("B2_REMOTE").ok().map(|shoots_remote| {
            let lightroom_remote = env::var("B2_LIGHTROOM_REMOTE")
                .expect("B2_LIGHTROOM_REMOTE must be set when B2_REMOTE is set");
            BackendConfig { backend: Backend::B2, shoots_remote, lightroom_remote }
        });

        if hot.is_none() && cold.is_none() {
            panic!(
                "No remote backends configured. \
                Set ONEDRIVE_REMOTE and/or B2_REMOTE in .env"
            );
        }

        Self {
            hot,
            cold,
            local_shoots: env::var("LOCAL_SHOOTS")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join("Pictures/Shoots")),
            local_lightroom: env::var("LOCAL_LIGHTROOM")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join("Pictures/Lightroom")),
        }
    }
}
