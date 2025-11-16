use std::env::args;

#[derive(Debug, Clone, Copy)]
enum ConfigError
{
    NoFileProvided,
}

struct Config
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
}
