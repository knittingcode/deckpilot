//! Where the standard output and error of an agent this app starts go:
//! `<data dir>/logs/output.log` (`AgentPaths::output_log`), so what an
//! agent prints before its logging starts, or as it crashes (the runtime's
//! own messages, a panic's summary, a system library's warnings), is still
//! there afterwards. The app starts the agent with `--quiet-stderr`, so the
//! agent's log records go to its daily log files only and are not written
//! twice.
//!
//! The file is bounded: when the app starts an agent and the file has
//! reached [`MAX_OUTPUT_BYTES`], it becomes `output.1.log` (replacing the
//! one before) and a new one is begun. The file is readable by the user
//! only (on Unix, mode 0600; the data directory is 0700 as well).

use std::fs::{File, OpenOptions};
use std::io;

use deckpilot_ipc::AgentPaths;

/// The size from which the output file is replaced by a new one when an
/// agent starts.
pub const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;

/// Opens the output file for an agent about to start, appending, after
/// moving a file that has reached `max_bytes` to the previous one. The log
/// directory is created if needed.
pub fn open(paths: &AgentPaths, max_bytes: u64) -> io::Result<File> {
    paths.create_dirs()?;
    let current = paths.output_log();
    if std::fs::metadata(&current).is_ok_and(|m| m.is_file() && m.len() >= max_bytes) {
        // An agent still running (one that refused to start twice, an old
        // one) keeps writing to the file it has, now the previous one. If
        // the move fails, the file is appended to as it is.
        if let Err(error) = std::fs::rename(&current, paths.previous_output_log()) {
            eprintln!("cannot rotate the agent's output file: {error}");
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&current)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn temp_paths(name: &str) -> (std::path::PathBuf, AgentPaths) {
        let dir = std::env::temp_dir().join(format!(
            "deckpilot-agent-output-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = AgentPaths::in_dir(dir.join("data"));
        (dir, paths)
    }

    #[test]
    fn output_is_appended_across_starts() {
        let (dir, paths) = temp_paths("append");
        writeln!(open(&paths, 1024).unwrap(), "first").unwrap();
        writeln!(open(&paths, 1024).unwrap(), "second").unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.output_log()).unwrap(),
            "first\nsecond\n"
        );
        assert!(!paths.previous_output_log().exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.output_log())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_full_file_becomes_the_previous_one() {
        let (dir, paths) = temp_paths("rotate");
        open(&paths, 8).unwrap().write_all(b"old run\n").unwrap();
        std::fs::write(paths.previous_output_log(), "oldest\n").unwrap();
        writeln!(open(&paths, 8).unwrap(), "new run").unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.output_log()).unwrap(),
            "new run\n"
        );
        // Only one previous file is kept.
        assert_eq!(
            std::fs::read_to_string(paths.previous_output_log()).unwrap(),
            "old run\n"
        );
        // Below the limit, nothing moves.
        writeln!(open(&paths, 1024).unwrap(), "again").unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.output_log()).unwrap(),
            "new run\nagain\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
