use std::env::args;

use crate::{
    engine::{Runner, RunnerError, stack::Stack},
    loader::Loader,
};

#[derive(Debug, Clone)]
pub enum ConfigError
{
    NoFileProvided,
    FileReadError,
    UnknownFlag(String),
    MissingOperand(String),
    InvalidOperand(String),
    LoaderInitError,
    StackInitError,
    RunnerError(RunnerError),
}

// List of optional flags that can be passed in as arguments
struct Flags
{
    stack_size: usize,
}

impl Flags
{
    const DEFAULT_STACK_SIZE: usize = 1024;
}

// Config the defaults for all the optional parameters
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
    filename: String, // name of the compiled for to execute
    flags: Flags, // Optional flags
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
                arg_ @ "--maxstack" =>
                {
                    let operand = args.next().ok_or(ConfigError::MissingOperand(arg_.into()))?;
                    flags.stack_size = operand.parse().map_err(|_| ConfigError::InvalidOperand(operand))?;
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
        let loader = Loader::from_file(&self.filename).map_err(|_| ConfigError::LoaderInitError)?;

        // Init Stack
        let mut stack = Stack::new(self.flags.stack_size);

        // Init Heap: TODO

        // Pass information to runner
        let mut runner = Runner::new(&mut stack, &loader);

        runner.run().map_err(ConfigError::RunnerError)
    }
}
