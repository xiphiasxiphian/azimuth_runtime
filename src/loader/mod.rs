use crate::loader::runnable::Runnable;

mod parser;
pub mod runnable;

pub struct Loader;

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl Loader
{
    pub fn from_file(filename: &str) -> Option<Self>
    {
        todo!()
    }

    pub fn get_entry_point(&self) -> Option<Runnable>
    {
        todo!()
    }
}
