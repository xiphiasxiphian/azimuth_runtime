pub trait ScopeMethods
{
    fn scope<F, R>(self, f: F) -> R
    where
        F: FnOnce(&Self) -> R;

    fn scope_mut<F, R>(self, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R;

    fn also<F>(self, f: F) -> Self
    where
        F: FnOnce(&Self);

    fn also_mut<F>(self, f: F) -> Self
    where
        F: FnOnce(&mut Self);
}

impl<T> ScopeMethods for T
{
    fn scope<F, R>(self, f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        f(&self)
    }

    fn scope_mut<F, R>(mut self, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        f(&mut self)
    }

    fn also<F>(self, f: F) -> T
    where
        F: FnOnce(&Self),
    {
        f(&self);
        self
    }

    fn also_mut<F>(mut self, f: F) -> T
    where
        F: FnOnce(&mut Self),
    {
        f(&mut self);
        self
    }
}

#[macro_export]
macro_rules! guard {
    ($check:expr) => {
        if !($check)
        {
            return None;
        }
    };
    ($check:expr, $ret:expr) => {
        if !($check)
        {
            return Err($ret);
        }
    };
}
