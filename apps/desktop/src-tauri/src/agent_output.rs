//! Where the standard output and error of an agent this app starts go:
//! `<data dir>/logs/output.log` (`AgentPaths::output_log`), so what an
//! agent prints before its logging starts, or as it crashes (the runtime's
//! own messages, a panic's summary, a system library's warnings), is still
//! there afterwards. The app starts the agent with `--quiet-stderr`, so the
//! agent's log records go to its daily log files only and are not written
//! twice.
//!
//! The file is bounded: when the app starts an agent and the file has
//! reached `MAX_OUTPUT_LOG_BYTES`, it becomes `output.1.log` (replacing the
//! one before) and a new one is begun. While it runs, the agent empties the
//! file once it passes that size (`deckpilot_agent::output`). The file is
//! readable by the user only (on Unix, mode 0600; the data directory is
//! 0700 as well). It is opened for appending, so an agent that is still
//! stopping and the next one never write over each other's output.

use std::fs::{File, OpenOptions};
use std::io;
use std::process::Stdio;

use deckpilot_ipc::AgentPaths;

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
    let file = options.open(&current)?;
    #[cfg(unix)]
    {
        // Also for a file that existed with another mode. A file system
        // that refuses the change still gets the output.
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
            eprintln!("cannot make the agent's output file private: {error}");
        }
    }
    Ok(file)
}

/// The agent's standard output and error: both the output file, or nowhere
/// if it cannot be opened (the agent starts in any case).
pub fn streams(paths: io::Result<AgentPaths>, max_bytes: u64) -> (Stdio, Stdio) {
    let file = paths
        .and_then(|paths| open(&paths, max_bytes))
        .and_then(|file| Ok((file.try_clone()?, file)));
    match file {
        Ok((stdout, stderr)) => (stdout.into(), stderr.into()),
        Err(error) => {
            eprintln!("the agent's output is not kept: {error}");
            (Stdio::null(), Stdio::null())
        }
    }
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

    /// An agent that is still stopping and the next one write to the same
    /// file at once: nothing is written over.
    #[test]
    fn two_writers_keep_each_others_output() {
        let (dir, paths) = temp_paths("two");
        let mut old = open(&paths, 1024).unwrap();
        let mut new = open(&paths, 1024).unwrap();
        writeln!(old, "old one stops").unwrap();
        writeln!(new, "new one starts").unwrap();
        writeln!(old, "old one's last words").unwrap();
        assert_eq!(
            std::fs::read_to_string(paths.output_log()).unwrap(),
            "old one stops\nnew one starts\nold one's last words\n"
        );
        drop((old, new));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_file_becomes_private() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, paths) = temp_paths("mode");
        paths.create_dirs().unwrap();
        std::fs::write(paths.output_log(), "").unwrap();
        std::fs::set_permissions(paths.output_log(), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        drop(open(&paths, 1024).unwrap());
        let mode = std::fs::metadata(paths.output_log())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A child's standard output and error both land in the file, and one
    /// that cannot be opened (a directory in its place) still lets the
    /// child start, with its output going nowhere.
    #[cfg(unix)]
    #[test]
    fn a_started_program_writes_both_streams_to_the_file() {
        let (dir, paths) = temp_paths("streams");
        let (stdout, stderr) = streams(Ok(paths.clone()), 1024);
        let status = std::process::Command::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .stdout(stdout)
            .stderr(stderr)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(paths.output_log()).unwrap(),
            "out\nerr\n"
        );

        std::fs::remove_file(paths.output_log()).unwrap();
        std::fs::create_dir(paths.output_log()).unwrap();
        let (stdout, stderr) = streams(Ok(paths.clone()), 1024);
        let status = std::process::Command::new("sh")
            .args(["-c", "echo out; echo err >&2"])
            .stdout(stdout)
            .stderr(stderr)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
