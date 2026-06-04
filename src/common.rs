pub trait ScopeMethods
{
    fn scope<F, R>(self, func: F) -> R
    where
        F: FnOnce(&Self) -> R;

    fn scope_mut<F, R>(self, func: F) -> R
    where
        F: FnOnce(&mut Self) -> R;

    fn also<F>(self, func: F) -> Self
    where
        F: FnOnce(&Self);

    fn also_mut<F>(self, func: F) -> Self
    where
        F: FnOnce(&mut Self);
}

impl<T> ScopeMethods for T
{
    fn scope<F, R>(self, func: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        func(&self)
    }

    fn scope_mut<F, R>(mut self, func: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        func(&mut self)
    }

    fn also<F>(self, func: F) -> T
    where
        F: FnOnce(&Self),
    {
        func(&self);
        self
    }

    fn also_mut<F>(mut self, func: F) -> T
    where
        F: FnOnce(&mut Self),
    {
        func(&mut self);
        self
    }
}

pub trait VecSet<T>
{
    fn set(&mut self, elem: T, index: usize) -> Option<T>;
}

impl<T> VecSet<T> for Vec<T>
{
    fn set(&mut self, elem: T, index: usize) -> Option<T>
    {
        (index < self.len())
            .then(|| {
                std::mem::replace(&mut self[index], elem)
            })
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
