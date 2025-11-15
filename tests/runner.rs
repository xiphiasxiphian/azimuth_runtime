use std::{fs::File, io::{Write}, path::Path};

mod assembler;

const FILE_PATTERN: &str = r"^.*\.test$";

fn test(path: &Path) -> datatest_stable::Result<()>
{
    let suffix = path.strip_prefix(Path::new("./tests/programs"))?;
    let mut bytecode_path = Path::new("./tests/compiled").join(suffix);
    bytecode_path.set_extension("azc");

    // Check whether to (re)compiles
    if !bytecode_path.exists() || bytecode_path.metadata()?.modified()? < path.metadata()?.modified()?
    {
        let string = std::fs::read_to_string(path)?;

        let mut bytes: Vec<u8> = vec![];
        assembler::assemble(string.as_str(), &mut bytes)?;

        _ = std::fs::create_dir_all(&(bytecode_path.parent().unwrap()));
        let mut file = File::create(&bytecode_path)?;
        file.write_all(&bytes)?;
    }

    Ok(())
}

datatest_stable::harness! {
    { test = test, root = "./tests/programs", pattern = FILE_PATTERN },
}
