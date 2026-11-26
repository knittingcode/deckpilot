//! The agent's standard output and error when the configuration app
//! started it (`--quiet-stderr`): both go to `logs/output.log`, which the
//! app opens and moves aside when it has grown too long, before each start.
//! A run of the agent can last weeks, so the agent bounds the file as it
//! runs as well: once it holds more than [`MAX_RUN_OUTPUT`], it is emptied
//! and [`TRUNCATED_NOTE`] is written at its start ([`spawn_cap`]). The
//! newest output, which explains a crash, is what stays.
//!
//! On Windows the file's handles also stop being inherited by the programs
//! the agent starts (`deckpilot_apps::native_windows`).

use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::time::Duration;

/// The most a run of the agent keeps in its output file.
pub const MAX_RUN_OUTPUT: u64 = 1024 * 1024;
/// How often the output file's size is checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);
/// The first line of an output file that was emptied.
pub const TRUNCATED_NOTE: &str =
    "[deckpilot-agent: earlier output of this run was removed, it had reached 1 MiB]\n";

/// Bounds standard output and error for the rest of the run, if standard
/// error is a regular file (one the app opened), on a thread of its own.
pub fn spawn_cap() {
    #[cfg(windows)]
    deckpilot_apps::native_windows::keep_std_handles_from_children();
    let Some(mut file) = stderr_file() else {
        return;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return;
    }
    let started = std::thread::Builder::new()
        .name("output-cap".into())
        .spawn(move || {
            loop {
                std::thread::sleep(CHECK_EVERY);
                let _ = cap(&mut file, MAX_RUN_OUTPUT);
            }
        });
    if let Err(error) = started {
        tracing::warn!(%error, "cannot bound the agent's output file");
    }
}

/// A new handle to standard error's file. Standard output is the same file
/// when the app starts the agent.
fn stderr_file() -> Option<File> {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        std::io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        std::io::stderr()
            .as_handle()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
}

/// Empties `file` if it holds more than `limit` bytes and writes
/// [`TRUNCATED_NOTE`]; returns whether it did. The position is reset as
/// well: the app opens the file for appending on Unix, where the position
/// does not matter, but not on Windows, where every handle to the file
/// shares it.
pub fn cap(file: &mut File, limit: u64) -> io::Result<bool> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() <= limit {
        return Ok(false);
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(TRUNCATED_NOTE.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn contents(file: &mut File) -> String {
        let mut text = String::new();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn a_file_over_the_limit_is_emptied_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut writer = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        writer.write_all(b"0123456789").unwrap();
        // At the limit: kept.
        assert!(!cap(&mut writer.try_clone().unwrap(), 10).unwrap());
        writer.write_all(b"abc").unwrap();
        // Over it: emptied through another handle, as the agent's thread
        // does; the next write continues after the note.
        assert!(cap(&mut writer.try_clone().unwrap(), 10).unwrap());
        writer.write_all(b"last words\n").unwrap();
        assert_eq!(
            contents(&mut writer),
            format!("{TRUNCATED_NOTE}last words\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_appending_file_continues_after_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut writer = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writer.write_all(&[b'x'; 64]).unwrap();
        assert!(cap(&mut writer.try_clone().unwrap(), 32).unwrap());
        writer.write_all(b"panic: ...\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{TRUNCATED_NOTE}panic: ...\n")
        );
    }
}
