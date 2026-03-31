use std::path::PathBuf;

pub struct Config {
    pub pictures_remote: String,
    pub lightroom_remote: String,
    pub local_pictures: PathBuf,
    pub local_lightroom: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs::home_dir().expect("cannot find home directory");
        Self {
            pictures_remote: "b2:wlta-photography/Pictures".into(),
            lightroom_remote: "b2:wlta-photography/Pictures/Lightroom".into(),
            local_pictures: home.join("Pictures"),
            local_lightroom: home.join("Pictures/Lightroom"),
        }
    }
}
