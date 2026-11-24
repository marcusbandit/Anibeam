//! Discord starts the app for a Join or an accepted invite (#966) by opening
//! `discord-<application id>://`, so the desktop needs a handler for that scheme. A small
//! hidden entry of its own, `discord-<id>.desktop`, runs the app the way the user's own entry
//! does, and becomes the handler when nothing else is. The join itself then comes over
//! Discord's socket once the app is up.
//!
//! A package (the AUR's anibeam and anibeam-bin, the deb, the rpm) ships its own hidden
//! entry under `/usr/share/applications` instead (#1167), so a Join works before the first
//! launch and the entry goes away with the package. A run with no live launcher entry of the
//! user's writes no entry: it removes a hidden entry an AppImage left in the user's folder
//! once that AppImage is gone, since the user's copy would hide the package's, and names the
//! package's entry the handler when nothing is.
//!
//! Only the user's own files are touched, never with a sandbox root, a drive or a shot, and
//! nothing when the build has no Discord application.

/// Writes the entry once, in the background.
pub fn ensure() {
    #[cfg(target_os = "linux")]
    {
        let args = crate::runtime::args();
        if args.root.is_some() || args.drive.is_some() || args.shoot.is_some() {
            return;
        }
        let Some(id) = anibeam_core::presence::bundled_client_id() else {
            return;
        };
        let id = id.to_string();
        let _ = std::thread::Builder::new()
            .name("discord-link".into())
            .spawn(move || linux::ensure(&id));
    }
}

/// The entry for `id`, running `exec` (the user's own Exec line) with the link as its URL.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn entry(id: &str, exec: &str) -> String {
    let exec = exec.trim();
    let exec = ["%U", "%u", "%F", "%f"]
        .iter()
        .find(|code| exec.contains(*code))
        .map(|code| exec.replace(code, "%u"))
        .unwrap_or_else(|| format!("{exec} %u"));
    format!(
        "[Desktop Entry]\nType=Application\nName=Anibeam\nExec={exec}\nNoDisplay=true\n\
         Terminal=false\nMimeType=x-scheme-handler/discord-{id};\n"
    )
}

/// The Exec line of an entry's `[Desktop Entry]` group, never an action's.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn main_exec(entry: &str) -> Option<String> {
    let mut in_main = false;
    for line in entry.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_main = t == "[Desktop Entry]";
        } else if in_main && let Some(exec) = t.strip_prefix("Exec=") {
            return Some(exec.to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use crate::scheme::linux::{applications, launcher_entry, quiet};

    pub fn ensure(id: &str) {
        let name = format!("discord-{id}.desktop");
        let search = std::env::var_os("PATH");
        // The user's own launcher entry, while what it runs is still there
        let own = launcher_entry().and_then(|own| {
            let exec = std::fs::read_to_string(&own)
                .ok()
                .and_then(|t| super::main_exec(&t))?;
            live(&exec, search.as_deref()).then_some((own, exec))
        });
        if let Some((own, exec)) = own {
            let Some(dir) = own.parent().map(|d| d.to_path_buf()) else {
                return;
            };
            let path = dir.join(&name);
            let want = super::entry(id, &exec);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(want.as_str()) {
                let tmp = dir.join(format!(".{name}.anibeam-tmp"));
                if std::fs::write(&tmp, &want).is_err() || std::fs::rename(&tmp, &path).is_err() {
                    let _ = std::fs::remove_file(&tmp);
                    return;
                }
                quiet(Command::new("update-desktop-database").arg(&dir));
            }
        } else {
            // A package's run: its own hidden entry handles the scheme, unless a dead copy
            // in the user's folder hides it
            if let Some(dir) = applications()
                && drop_dead(&dir.join(&name), search.as_deref())
            {
                quiet(Command::new("update-desktop-database").arg(&dir));
            }
            if !data_dirs()
                .iter()
                .any(|d| d.join("applications").join(&name).is_file())
            {
                return;
            }
        }
        // Named as the default too: a desktop without desktop-file-utils never lists the
        // package's entry in mimeinfo.cache, and xdg-open then finds nothing
        let mime = format!("x-scheme-handler/discord-{id}");
        let current = Command::new("xdg-mime")
            .args(["query", "default", &mime])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        if let Ok(out) = current
            && String::from_utf8_lossy(&out.stdout).trim().is_empty()
        {
            quiet(Command::new("xdg-mime").args(["default", &name, &mime]));
        }
    }

    /// `$XDG_DATA_DIRS`, absolute paths only, else its default.
    fn data_dirs() -> Vec<PathBuf> {
        let dirs: Vec<PathBuf> = std::env::var_os("XDG_DATA_DIRS")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        if dirs.is_empty() {
            vec!["/usr/local/share".into(), "/usr/share".into()]
        } else {
            dirs
        }
    }

    /// Whether `exec` starts something: its program can run, as a path or on `search`. An
    /// `env` with options counts as live, since those can take the place of the program.
    pub(super) fn live(exec: &str, search: Option<&OsStr>) -> bool {
        let mut words = exec.split_whitespace();
        if words
            .next()
            .is_some_and(|w| w == "env" || w.ends_with("/env"))
            && words.next().is_some_and(|w| w.starts_with('-'))
        {
            return true;
        }
        crate::scheme::program(exec).is_some_and(|p| runs(&p, search))
    }

    /// Removes the hidden entry at `path` when it starts nothing: its Exec names no program
    /// that can run, as a path or on `search` (a `$PATH` value). True when it went.
    pub(super) fn drop_dead(path: &Path, search: Option<&OsStr>) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let starts = super::main_exec(&text).is_some_and(|e| live(&e, search));
        !starts && std::fs::remove_file(path).is_ok()
    }

    /// Whether `program` can be started: an executable file at its path, or one on `search`.
    fn runs(program: &str, search: Option<&OsStr>) -> bool {
        let executable = |p: &Path| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        };
        if program.contains('/') {
            let p = Path::new(program);
            return p.is_absolute() && executable(p);
        }
        search.is_some_and(|s| {
            std::env::split_paths(s)
                .filter(|d| d.is_absolute())
                .any(|d| executable(&d.join(program)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_runs_the_app_as_its_own_entry_does_with_the_link() {
        let e = entry("123", "env A=1 /opt/anibeam %U");
        assert!(e.contains("\nExec=env A=1 /opt/anibeam %u\n"));
        assert!(e.contains("MimeType=x-scheme-handler/discord-123;"));
        assert!(e.contains("NoDisplay=true"));
        assert!(entry("9", "/opt/anibeam").contains("\nExec=/opt/anibeam %u\n"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_hidden_entry_goes_only_once_what_it_runs_is_gone() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let app = bin.join("anibeam");
        std::fs::write(&app, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
        let search = bin.clone().into_os_string();
        let at = |exec: &str| {
            let path = root.path().join("discord-1.desktop");
            std::fs::write(&path, entry("1", exec)).unwrap();
            let gone = linux::drop_dead(&path, Some(&search));
            assert_eq!(gone, !path.exists());
            gone
        };
        // A program that is there, by path or on PATH, keeps its entry
        assert!(!at(&format!("{} %U", app.display())));
        assert!(!at("anibeam %U"));
        assert!(!at(&format!("env A=1 {}", app.display())));
        // env's options can stand where the program would, so those stay
        assert!(!at("env -u FOO /nowhere/anibeam %U"));
        // An AppImage that was deleted, a name not on PATH, a file that cannot run
        assert!(at("/nowhere/Anibeam.AppImage %U"));
        assert!(at("anibeam-gone %U"));
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(at("anibeam %U"));
        // No entry at all is nothing to remove
        assert!(!linux::drop_dead(
            &root.path().join("none.desktop"),
            Some(&search)
        ));
    }

    #[test]
    fn only_the_main_group_s_exec_counts() {
        let e = "[Desktop Action new]\nExec=/opt/other\n\n[Desktop Entry]\nName=A\nExec=/opt/anibeam %U\n";
        assert_eq!(main_exec(e).as_deref(), Some("/opt/anibeam %U"));
        assert_eq!(main_exec("[Desktop Action x]\nExec=/a\n"), None);
    }
}
