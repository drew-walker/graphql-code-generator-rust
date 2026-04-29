pub fn debug_log(message: impl AsRef<str>) {
    eprintln!("{}", message.as_ref());
}
