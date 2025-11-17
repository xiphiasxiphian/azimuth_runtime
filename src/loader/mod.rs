pub mod runnable;

use crate::loader::runnable::{Directive, Runnable};

pub struct Loader
{
    runnables: Vec<Runnable>,
}

// This is a temporary solution that just statically loads the
// entire file at once.
// In the future this will happen dynamically where required.
impl Loader
{
    pub fn from_file(filename: &str) -> Option<Self>
    {
        // Again there is definitely a better way of doing this that doesn't
        // involve spamming mutable variables everywhere
        // But this entire system is going to get rewritten as some point
        // and I just want it working right now.

        let contents = std::fs::read(filename).ok()?;
        let mut runnables: Vec<Runnable> = vec![];

        let mut remaining: &[u8] = &contents;
        while let [_, ..] = remaining
        {
            let (runnable, rem) = Runnable::from_bytes(remaining)?;
            runnables.push(runnable);
            remaining = rem;
        }


        Some(Self {
            runnables
        })
    }

    pub fn get_entry_point(&self) -> Option<&Runnable>
    {
        self.runnables.iter().find(|x| x.directives().contains(&Directive::Start))
    }
}
