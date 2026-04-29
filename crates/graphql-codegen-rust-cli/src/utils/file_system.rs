use std::fs;
use std::io;
use std::path::Path;

pub async fn mkdirp(path: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(path)
}

pub async fn read_file(path: impl AsRef<Path>) -> io::Result<String> {
    fs::read_to_string(path)
}

pub async fn write_file(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    fs::write(path, contents)
}

pub async fn unlink_file(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_file(path)
}
