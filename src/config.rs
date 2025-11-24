use std::env::args;

use crate::{engine::stack::Stack, loader::Loader};

#[derive(Debug, Clone)]
pub enum ConfigError
{
    NoFileProvided,
    FileReadError,
    UnknownFlag(String),
    MissingOperand(String),
    InvalidOperand(String),
}

struct Flags
{
    stack_size: usize,
}

impl Flags
{
    const DEFAULT_STACK_SIZE: usize = 1024;
}

impl Default for Flags
{
    fn default() -> Self
    {
        Self {
            stack_size: Self::DEFAULT_STACK_SIZE,
        }
    }
}

pub struct Config
{
    filename: String,
    flags: Flags,
}

impl Config
{
    pub fn new() -> Result<Self, ConfigError>
    {
        let mut args = args().skip(1); // Skip the executable name itself
        let mut flags = Flags::default();
        let mut filename: Option<String> = None;

        while let Some(arg) = args.next()
        {
            match arg.as_str()
            {
                a @ "--maxstack" =>
                {
                    let operand = args.next().ok_or(ConfigError::MissingOperand(a.into()))?;
                    flags.stack_size = operand
                        .parse()
                        .map_err(|_| ConfigError::InvalidOperand(operand))?;
                }
                _file =>
                {
                    filename
                        .replace(arg)
                        .map_or(Ok(()), |x| Err(ConfigError::UnknownFlag(x)))?;
                }
            }
        }

        Ok(Self {
            filename: filename.ok_or(ConfigError::NoFileProvided)?,
            flags,
        })
    }

    pub fn execute(&self) -> Result<(), ConfigError>
    {
        // Load file

        // -- Init Required systems --

        // Init Loader (WIP)
        let loader = Loader::from_file(&self.filename);

        // Init Stack
        let stack = Stack::new(self.flags.stack_size);

        // Init Heap

        // Pass information to runner

        todo!();
        Ok(())
    }
}
