//! The git remotes of a project repository's folder, for the app's "Link
//! remote" (Git Providers design §9.2, `listRepositoryRemotes`). The host
//! reads them, never a plugin: Git Providers has no `tools`, and the Git
//! plugin's `remote` lists names only.
//!
//! * **Read-only, local, no shell.** One `git remote -v` through the
//!   `tool.run` runner ([`super::execute`]): the user's own `git`
//!   ([`ToolHost::executable`]), the fixed hardening prefix, the cleared
//!   environment, the local read time limit. It never fetches, pulls or
//!   writes, and nothing here contacts a server.
//! * **Only the bound folder's own repository**, as for a plugin's run:
//!   the folder is resolved (links followed) and must be a directory, the
//!   work tree and the git directory must be it or inside it
//!   ([`super::work_tree_inside`]), and discovery stops at it.
//! * **No credentials leave.** An `https` URL loses its user name and
//!   password, an `ssh` one its password, both any query or fragment, and
//!   an SSH user name is kept only when it reads like an account (`git`),
//!   never like a token; a URL git does not hold in its canonical form is
//!   left out rather than rewritten; a URL that is still not one a project
//!   may store
//!   ([`RepositoryRemote::check`]: `http`, `file`, a path, `ext::`, a
//!   leftover password in the scp form) is left out, and so is a remote
//!   whose name is not a remote name. At most
//!   [`MAX_DETECTED_REMOTES`].

use std::path::Path;
use std::time::Instant;

use deckpilot_core::RepositoryRemote;
use deckpilot_ipc::connections::{DetectedRemote, MAX_DETECTED_REMOTES};
use deckpilot_plugin_abi::privileged::Tool;

use super::{Denial, ToolHost, codes, execute, git, work_tree_inside};

/// The remotes of the repository at `folder` (a bound folder of this
/// computer), until `deadline` at the latest.
pub(crate) fn detect(
    host: &ToolHost,
    folder: &Path,
    deadline: Instant,
) -> Result<Vec<DetectedRemote>, Denial> {
    let gone = || {
        Denial::new(
            codes::INVALID_CWD,
            "the repository's folder does not exist on this computer",
        )
    };
    // As a plugin's handle: the real folder, links followed.
    let folder = folder
        .is_absolute()
        .then(|| std::fs::canonicalize(folder).ok())
        .flatten()
        .filter(|f| f.is_dir())
        .ok_or_else(gone)?;
    let program = host.executable(Tool::Git)?;
    let plan = git::remote_urls(host, &folder, program);
    let never = || None;
    work_tree_inside(host, &plan, Some(deadline), &never)?;
    let listed = execute(&plan, Some(deadline), &never)?;
    if !listed.output.success() {
        return Err(Denial::new(
            codes::NOT_A_REPOSITORY,
            "git could not list the repository's remotes",
        ));
    }
    Ok(parse(&listed.output.stdout))
}

/// `git remote -v`'s lines (`<name>\t<url> (fetch)`, then `(push)`), as
/// remotes without credentials. The first entry of a name wins.
pub(crate) fn parse(stdout: &str) -> Vec<DetectedRemote> {
    let mut remotes: Vec<DetectedRemote> = Vec::new();
    for line in stdout.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let (url, push) = if let Some(url) = rest.strip_suffix(" (fetch)") {
            (url, false)
        } else if let Some(url) = rest.strip_suffix(" (push)") {
            (url, true)
        } else {
            continue;
        };
        let Some(url) = storable(name, url) else {
            continue;
        };
        if let Some(remote) = remotes.iter_mut().find(|r| r.name == name) {
            if push && remote.url != url && remote.push_url.is_none() {
                remote.push_url = Some(url);
            }
        } else if !push && remotes.len() < MAX_DETECTED_REMOTES {
            remotes.push(DetectedRemote {
                name: name.to_owned(),
                url,
                push_url: None,
            });
        }
    }
    remotes
}

/// `url` without credentials, if a project may store it as the remote
/// `name`'s URL.
fn storable(name: &str, url: &str) -> Option<String> {
    let clean = without_credentials(url)?;
    let remote = RepositoryRemote {
        name: name.to_owned(),
        remote_url: Some(clean.clone()),
        ..RepositoryRemote::default()
    };
    remote.check().is_ok().then_some(clean)
}

/// An `https` URL without its user name and password, an `ssh` one
/// without its password, either without a query or fragment; the scp form
/// (`git@host:path`) as it is (it has no password: the core refuses a
/// `user:password@` there). Anything else is `None`.
fn without_credentials(url: &str) -> Option<String> {
    // As git has it, never re-encoded into another URL.
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    if url.starts_with("https://") || url.starts_with("ssh://") {
        let mut parsed = url::Url::parse(url).ok()?;
        if parsed.as_str() != url {
            // Not canonical (case, `\`, an IDN, a default port): a
            // suggestion is git's own URL or nothing.
            return None;
        }
        if parsed.scheme() == "https" || !account(parsed.username()) {
            parsed.set_username("").ok()?;
        }
        parsed.set_password(None).ok()?;
        parsed.set_query(None);
        parsed.set_fragment(None);
        return Some(parsed.to_string());
    }
    if url.contains("://") {
        return None;
    }
    // The scp form needs its user: one that reads like a token is no
    // suggestion at all.
    let user = url.split_once('@').map_or("", |(user, _)| user);
    account(user).then(|| url.to_owned())
}

/// An SSH user name that names an account (`git`, `deploy`), not a token:
/// short, lower-case, and nothing shaped like a secret.
fn account(user: &str) -> bool {
    user.is_empty()
        || (user.len() <= 32
            && user.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && user
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
            && !deckpilot_plugin_store::looks_like_secret(user))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(name: &str, url: &str, push: Option<&str>) -> DetectedRemote {
        DetectedRemote {
            name: name.into(),
            url: url.into(),
            push_url: push.map(str::to_owned),
        }
    }

    #[test]
    fn remotes_are_listed_once_with_their_push_url_when_it_differs() {
        let out = parse(
            "origin\thttps://gitlab.com/shop/main.git (fetch)\n\
             origin\thttps://gitlab.com/shop/main.git (push)\n\
             upstream\tgit@github.com:acme/main.git (fetch)\n\
             upstream\tssh://git@github.com/acme/main-push.git (push)\n",
        );
        assert_eq!(
            out,
            vec![
                remote("origin", "https://gitlab.com/shop/main.git", None),
                remote(
                    "upstream",
                    "git@github.com:acme/main.git",
                    Some("ssh://git@github.com/acme/main-push.git")
                ),
            ]
        );
    }

    /// No user name or password of an `https` URL, no password of an `ssh`
    /// one, no query or fragment (a token in either) ever reaches the app.
    #[test]
    fn credentials_are_removed() {
        let out = parse(
            "a\thttps://user:pw@gitlab.com/shop/main.git (fetch)\n\
             b\thttps://oauth2@gitlab.com/shop/b.git?private_token=x#frag (fetch)\n\
             c\tssh://git:pw@code.example.com:2222/shop/c.git (fetch)\n\
             c\tssh://git:pw@code.example.com:2222/shop/c.git (push)\n",
        );
        assert_eq!(
            out,
            vec![
                remote("a", "https://gitlab.com/shop/main.git", None),
                remote("b", "https://gitlab.com/shop/b.git", None),
                remote("c", "ssh://git@code.example.com:2222/shop/c.git", None),
            ]
        );
        let text = format!("{out:?}");
        for leaked in ["pw", "oauth2", "private_token", "frag"] {
            assert!(!text.contains(leaked), "{leaked}: {text}");
        }
    }

    /// What a project may not store is left out: other schemes, paths,
    /// remote helpers, a password left in the scp form, odd names, lines
    /// that are not `git remote -v`'s.
    #[test]
    fn what_a_project_cannot_store_is_left_out() {
        let out = parse(
            "http\thttp://gitlab.com/shop/main.git (fetch)\n\
             file\tfile:///srv/git/main.git (fetch)\n\
             path\t/srv/git/main.git (fetch)\n\
             relative\t../main (fetch)\n\
             helper\text::sh -c touch% /tmp/x (fetch)\n\
             scp\tuser:pw@host:shop/main.git (fetch)\n\
             -opt\thttps://gitlab.com/shop/main.git (fetch)\n\
             spaced\thttps://gitlab.com/shop/a b.git (fetch)\n\
             no tab https://gitlab.com/shop/main.git (fetch)\n\
             kind\thttps://gitlab.com/shop/main.git\n\
             ok\tgit@gitlab.com:shop/main.git (fetch)\n",
        );
        assert_eq!(
            out,
            vec![remote("ok", "git@gitlab.com:shop/main.git", None)]
        );
    }

    /// An SSH user that reads like a token never leaves: dropped from an
    /// `ssh://` URL, and an scp-form remote with one is no suggestion; a
    /// URL git does not hold canonically is left out, never rewritten.
    #[test]
    fn token_shaped_ssh_users_and_rewritten_urls_never_leave() {
        let token = format!("{}{}", "gl", "pat-abcdefghijklmnopqrst");
        let out = parse(&format!(
            "a\tssh://{token}@code.example.com/shop/a.git (fetch)\n\
             b\t{token}@code.example.com:shop/b.git (fetch)\n\
             c\tssh://Deploy@code.example.com/shop/c.git (fetch)\n\
             d\thttps://Code.Example.com/shop/d.git (fetch)\n\
             e\thttps://code.example.com:443/shop/e.git (fetch)\n\
             f\tdeploy@code.example.com:shop/f.git (fetch)\n"
        ));
        assert_eq!(
            out,
            vec![
                remote("a", "ssh://code.example.com/shop/a.git", None),
                remote("c", "ssh://code.example.com/shop/c.git", None),
                remote("f", "deploy@code.example.com:shop/f.git", None),
            ]
        );
        assert!(!format!("{out:?}").contains(&token));
    }

    #[test]
    fn at_most_sixteen_remotes_are_listed() {
        let lines = (0..20)
            .map(|i| format!("r{i}\thttps://gitlab.com/shop/r{i}.git (fetch)"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = parse(&lines);
        assert_eq!(out.len(), MAX_DETECTED_REMOTES);
        assert_eq!(out[0].name, "r0");
    }

    // -----------------------------------------------------------------
    // Against git itself, in temporary repositories only.

    use std::path::PathBuf;
    use std::time::Duration;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    /// The system's `git`, or `None` (the test is skipped).
    fn real_git() -> Option<PathBuf> {
        [
            "/usr/bin/git",
            "/opt/homebrew/bin/git",
            "/usr/local/bin/git",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
    }

    /// A step of a test's setup, with no user configuration.
    fn setup(git: &Path, home: &Path, dir: &Path, args: &[&str]) {
        let status = std::process::Command::new(git)
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "a git setup step failed");
    }

    fn host(git: &Path, home: &Path) -> ToolHost {
        ToolHost::for_test()
            .with_environment(&[
                ("PATH", "/usr/bin:/bin"),
                ("HOME", &home.display().to_string()),
            ])
            .with_executable(Tool::Git, git)
    }

    /// The remotes of a temporary repository, without credentials; the
    /// repository is exactly as it was (nothing written, nothing fetched).
    #[test]
    fn a_repositorys_remotes_are_read_without_changing_it() {
        let Some(git) = real_git() else { return };
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let repo = std::fs::canonicalize(repo).unwrap();
        setup(&git, &home, &repo, &["init", "-q"]);
        setup(
            &git,
            &home,
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://user:pw@gitlab.com/shop/main.git",
            ],
        );
        setup(
            &git,
            &home,
            &repo,
            &["remote", "add", "upstream", "git@github.com:acme/main.git"],
        );
        setup(
            &git,
            &home,
            &repo,
            &[
                "remote",
                "set-url",
                "--push",
                "upstream",
                "ssh://git@github.com/acme/push.git",
            ],
        );
        let before = snapshot(&repo.join(".git"));
        let out = detect(&host(&git, &home), &repo, soon()).unwrap();
        assert_eq!(
            out,
            vec![
                remote("origin", "https://gitlab.com/shop/main.git", None),
                remote(
                    "upstream",
                    "git@github.com:acme/main.git",
                    Some("ssh://git@github.com/acme/push.git")
                ),
            ]
        );
        assert_eq!(
            snapshot(&repo.join(".git")),
            before,
            "the repository changed"
        );
        assert!(!repo.join(".git/FETCH_HEAD").exists());
    }

    /// Every file under `dir` with its bytes.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(next) = stack.pop() {
            for entry in std::fs::read_dir(next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push((path.clone(), std::fs::read(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    /// A folder that is no repository of its own, inside one (a dotfiles
    /// repository in the home folder), shows nothing of that one; a
    /// missing folder is refused before git runs.
    #[test]
    fn only_the_folders_own_repository_is_read_and_links_are_followed() {
        let Some(git) = real_git() else { return };
        let dir = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(dir.path()).unwrap();
        setup(&git, &home, &home, &["init", "-q"]);
        setup(
            &git,
            &home,
            &home,
            &[
                "remote",
                "add",
                "dotfiles",
                "https://github.com/me/dotfiles.git",
            ],
        );
        let inner = home.join("Projects/app");
        std::fs::create_dir_all(&inner).unwrap();
        let refused = detect(&host(&git, &home), &inner, soon()).unwrap_err();
        assert_eq!(refused.code, codes::NOT_A_REPOSITORY);
        let refused = detect(&host(&git, &home), &home.join("gone"), soon()).unwrap_err();
        assert_eq!(refused.code, codes::INVALID_CWD);
        let refused = detect(&host(&git, &home), Path::new("relative"), soon()).unwrap_err();
        assert_eq!(refused.code, codes::INVALID_CWD);
        // A file is no folder.
        std::fs::write(home.join("file"), "x").unwrap();
        let refused = detect(&host(&git, &home), &home.join("file"), soon()).unwrap_err();
        assert_eq!(refused.code, codes::INVALID_CWD);

        // A link to a repository is read as that repository.
        let repo = home.join("src/shop");
        std::fs::create_dir_all(&repo).unwrap();
        setup(&git, &home, &repo, &["init", "-q"]);
        setup(
            &git,
            &home,
            &repo,
            &["remote", "add", "origin", "git@gitlab.com:shop/main.git"],
        );
        #[cfg(unix)]
        let linked = {
            std::os::unix::fs::symlink(&repo, home.join("shop-link")).unwrap();
            true
        };
        // Windows: a directory symbolic link needs the privilege to create
        // one (CI runners have it). Skipped, and said so, only when this
        // account may not create links; any other failure fails the test.
        #[cfg(windows)]
        let linked = match std::os::windows::fs::symlink_dir(&repo, home.join("shop-link")) {
            Ok(()) => true,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1314) =>
            {
                eprintln!(
                    "skipped \"a link to a repository is read as that repository\": \
                     this account may not create directory links ({error})"
                );
                false
            }
            Err(error) => panic!("creating the directory link: {error}"),
        };
        if linked {
            let out = detect(&host(&git, &home), &home.join("shop-link"), soon()).unwrap();
            assert_eq!(
                out,
                vec![remote("origin", "git@gitlab.com:shop/main.git", None)]
            );
        }

        // A `.git` file pointing at another repository's git directory is
        // refused before the listing.
        let other = home.join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        setup(&git, &home, &other, &["init", "-q"]);
        let pointer = home.join("src/pointer");
        std::fs::create_dir_all(&pointer).unwrap();
        std::fs::write(
            pointer.join(".git"),
            format!("gitdir: {}\n", other.join(".git").display()),
        )
        .unwrap();
        let refused = detect(&host(&git, &home), &pointer, soon()).unwrap_err();
        assert_eq!(refused.code, codes::OUTSIDE_BINDING);
    }

    /// Exactly what runs: the containment check, then `remote -v` with the
    /// fixed hardening prefix, in a cleared environment; nothing else (no
    /// fetch, no pull, no shell).
    #[test]
    fn only_the_containment_check_and_remote_v_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let log = root.join("argv.log");
        let fake = root.join("git");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\nenv | sort >> '{log}.env'\n\
             case \" $* \" in *\" --show-toplevel \"*) pwd -P; pwd -P; exit 0;; esac\n\
             printf 'origin\\thttps://tok@gitlab.com/shop/main.git (fetch)\\n'\n",
            log = log.display()
        );
        let status = std::process::Command::new("/bin/sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(&fake)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child.stdin.take().unwrap().write_all(script.as_bytes())?;
                child.wait()
            })
            .unwrap();
        assert!(status.success());
        let host = ToolHost::for_test()
            .with_environment(&[
                ("PATH", "/usr/bin:/bin"),
                ("HOME", "/nonexistent"),
                ("GITHUB_TOKEN", "planted"),
                ("GIT_DIR", "/elsewhere"),
            ])
            .with_executable(Tool::Git, &fake);
        let out = detect(&host, &repo, soon()).unwrap();
        assert_eq!(
            out,
            vec![remote("origin", "https://gitlab.com/shop/main.git", None)]
        );
        let prefix = format!(
            "-C {} --no-pager --no-optional-locks {}",
            repo.display(),
            git::HARDENING
                .iter()
                .map(|c| format!("-c {c}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let runs = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            runs.lines().collect::<Vec<_>>(),
            vec![
                format!("{prefix} rev-parse --show-toplevel --absolute-git-dir"),
                format!("{prefix} remote -v"),
            ]
        );
        let env = std::fs::read_to_string(format!("{}.env", log.display())).unwrap();
        assert!(
            !env.contains("planted") && !env.contains("GIT_DIR="),
            "{env}"
        );
        assert!(env.contains("GIT_TERMINAL_PROMPT=0"), "{env}");
    }
}
