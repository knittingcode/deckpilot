//! The agent's standard output and error when the configuration app
//! started it (`--quiet-stderr`): both go to `logs/output.log`, which the
//! app opens for appending and moves aside before a start once it has
//! reached [`MAX_OUTPUT_LOG_BYTES`]. A run of the agent can last weeks, so
//! the agent bounds the file as it runs as well: every [`CHECK_EVERY`],
//! once the file holds more than that, it is emptied and
//! [`TRUNCATED_NOTE`] is written ([`spawn_cap`]). The newest output, which
//! explains a crash, is what stays.
//!
//! Every writer appends, so after the file was emptied the next write lands
//! at its start, with no gap. The file is emptied through a handle of its
//! own: on Unix a duplicate of standard error, on Windows the file opened
//! by its path for writing (a handle opened for appending cannot shorten a
//! file there).
//!
//! On Windows the standard handles also stop being inherited by the
//! programs the agent starts (`deckpilot_apps::native_windows`).

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

pub use deckpilot_ipc::paths::MAX_OUTPUT_LOG_BYTES;

/// How often the output file's size is checked.
pub const CHECK_EVERY: Duration = Duration::from_secs(30);
/// Written when the output file was emptied.
pub const TRUNCATED_NOTE: &str = "[deckpilot-agent: earlier output of this run was removed, the file had reached its size limit]\n";

/// Bounds standard output and error for the rest of the run, on a thread
/// of its own, if standard error is a regular file (`output`, which the
/// app opened).
pub fn spawn_cap(output: &Path) {
    #[cfg(windows)]
    deckpilot_apps::native_windows::keep_std_handles_from_children();
    if !stderr_file().is_some_and(|file| file.metadata().is_ok_and(|m| m.is_file())) {
        return;
    }
    // Unix empties the file through standard error, not by its path.
    #[cfg(unix)]
    let _ = output;
    #[cfg(not(unix))]
    let output = output.to_path_buf();
    let started = std::thread::Builder::new()
        .name("output-cap".into())
        .spawn(move || {
            #[cfg(unix)]
            let target = stderr_file();
            loop {
                std::thread::sleep(CHECK_EVERY);
                #[cfg(unix)]
                let file = target.as_ref().and_then(|file| file.try_clone().ok());
                #[cfg(not(unix))]
                let file = truncatable(&output);
                if let Some(file) = file {
                    let _ = cap(&file, MAX_OUTPUT_LOG_BYTES, &mut io::stderr());
                }
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
        io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        io::stderr()
            .as_handle()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
}

/// The output file opened so that it can be shortened (Windows).
#[cfg_attr(unix, allow(dead_code))]
fn truncatable(output: &Path) -> Option<File> {
    std::fs::OpenOptions::new().write(true).open(output).ok()
}

/// Empties `file` if it holds more than `limit` bytes, then writes
/// [`TRUNCATED_NOTE`] to `note` (standard error, which appends); returns
/// whether it did. Output written between the two comes before the note.
pub fn cap(file: &File, limit: u64, note: &mut impl Write) -> io::Result<bool> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() <= limit {
        return Ok(false);
    }
    file.set_len(0)?;
    note.write_all(TRUNCATED_NOTE.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As the app opens it for the agent.
    fn appending(path: &Path) -> File {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn a_file_over_the_limit_is_emptied_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut stderr = appending(&path);
        // A second writer, as an agent that is still stopping.
        let mut other = appending(&path);
        stderr.write_all(b"0123456789").unwrap();
        // At the limit: kept.
        assert!(!cap(&truncatable(&path).unwrap(), 10, &mut stderr).unwrap());
        other.write_all(b"abc").unwrap();
        // Over it: emptied through a handle of its own; every writer
        // continues at the new end, with no gap.
        assert!(cap(&truncatable(&path).unwrap(), 10, &mut stderr).unwrap());
        stderr.write_all(b"last words\n").unwrap();
        other.write_all(b"and more\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{TRUNCATED_NOTE}last words\nand more\n")
        );
    }

    /// Unix empties the file through a duplicate of standard error's
    /// appending handle.
    #[cfg(unix)]
    #[test]
    fn an_appending_handle_empties_it_on_unix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.log");
        let mut stderr = appending(&path);
        stderr.write_all(&[b'x'; 64]).unwrap();
        assert!(cap(&stderr.try_clone().unwrap(), 32, &mut stderr).unwrap());
        stderr.write_all(b"panic: ...\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{TRUNCATED_NOTE}panic: ...\n")
        );
    }
}
