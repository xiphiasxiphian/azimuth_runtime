use crate::config::{Config, ConfigError};

mod config;
mod engine;

fn main() -> Result<(), ConfigError>
{
    Config::new()?.execute()
}
