#[derive(Debug, Clone, Default)]
pub struct Logger;

impl Logger {
    pub fn error(&self, message: impl AsRef<str>) {
        eprintln!("{}", message.as_ref());
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        eprintln!("{}", message.as_ref());
    }
}

pub fn get_logger() -> Logger {
    Logger
}
