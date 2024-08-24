use super::konst::CONFIG_FILENAME;

pub struct Config {
    pub name: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: CONFIG_FILENAME.to_owned(),
        }
    }
}
