use crate::config::{Config, ConfigError};

mod config;
mod engine;
mod loader;
mod memory;

fn main() -> Result<(), ConfigError>
{
    Config::new()?.execute()
}
