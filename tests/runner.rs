use std::{fs::File, io::Write, path::Path};

use assert_cmd::Command;
use constcat::concat;

mod assembler;

const FILE_PATTERN: &str = r"^.*\.test$";

const TEST_BASE: &str = "./tests";
const PROGRAM_PATH: &str = concat!(TEST_BASE, "/programs");
const COMPILED_PATH: &str = concat!(TEST_BASE, "/compiled");

const COMPILED_FILE_EXTENSION: &str = "azc";

fn test(path: &Path) -> datatest_stable::Result<()>
{
    let suffix = path.strip_prefix(Path::new(PROGRAM_PATH))?;
    let mut bytecode_path = Path::new(COMPILED_PATH).join(suffix);
    bytecode_path.set_extension(COMPILED_FILE_EXTENSION);

    // Check whether to (re)compile
    if !bytecode_path.exists() || bytecode_path.metadata()?.modified()? < path.metadata()?.modified()?
    {
        let string = std::fs::read_to_string(path)?;

        let mut bytes: Vec<u8> = vec![];
        assembler::assemble(string.as_str(), &mut bytes)?;

        _ = std::fs::create_dir_all(bytecode_path.parent().unwrap());
        let mut file = File::create(&bytecode_path)?;
        file.write_all(&bytes)?;
    }

    Ok(())
}

datatest_stable::harness! {
    { test = test, root = PROGRAM_PATH, pattern = FILE_PATTERN },
}
