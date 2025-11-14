use std::path::Path;

mod assembler;

const FILE_PATTERN: &str = r"^.*\.test$";

fn tmp(path: &Path) -> datatest_stable::Result<()>
{
    Ok(())
}

datatest_stable::harness! {
    { test = tmp, root = "./tests/programs", pattern = FILE_PATTERN },
}
