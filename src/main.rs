use crate::config::{Config, ConfigError};

mod config;
mod engine;
mod loader;

fn main() -> Result<(), ConfigError>
{
    Config::new()?.execute()
}
