use std::env::args;

#[derive(Debug, Clone, Copy)]
pub enum ConfigError
{
    NoFileProvided,
    FileReadError,
}

pub struct Config
{
    filename: String,
}

impl Config
{
    pub fn new() -> Result<Self, ConfigError>
    {
        let mut args = args().skip(1); // Skip the executable name itself
        let filename = args.next().ok_or(ConfigError::NoFileProvided)?;

        Ok(Self {
            filename
        })
    }

    pub fn execute(&self) -> Result<(), ConfigError>
    {
        // Load file
        let contents = std::fs::read(&self.filename).map_err(|_| ConfigError::FileReadError)?;

        // Init Required systems


        Ok(())
    }
}
