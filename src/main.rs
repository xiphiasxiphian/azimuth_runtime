use crate::config::{Config, ConfigError};

mod config;
mod loader;
mod engine;

fn main() -> Result<(), ConfigError>
{
    Config::new()?.execute()
}
