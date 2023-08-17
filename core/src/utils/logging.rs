use log::warn;

pub trait LoggingExit {
    fn warn_if_none(self, msg: impl AsRef<str>) -> Self;
}

impl<T> LoggingExit for Option<T> {
    fn warn_if_none(self, msg: impl AsRef<str>) -> Self {
        if self.is_none() {
            warn!("{}", msg.as_ref())
        }
        self
    }
}
