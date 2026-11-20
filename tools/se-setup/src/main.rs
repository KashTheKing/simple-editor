//! Simple Editor setup: the installer, the updater and the uninstaller in one small program. The role
//! comes from the file name (`update.exe`, `uninstall.exe`, anything else installs) or from `--update` /
//! `--uninstall`. `--silent` runs without a window, with the defaults.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod addons;
mod json;
mod net;
mod paths;
mod reg;
mod report;
mod setup;
mod sha256;
mod version;
mod zip;
#[path = "../../../src/shell_keys.rs"]
mod shell_keys;
#[cfg(test)]
mod testutil;
#[cfg(windows)]
mod win;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    Install,
    Update,
    Uninstall,
}

/// Which program this is, from the arguments and then the exe's own name.
pub fn role(args: &[String], exe: &std::path::Path) -> Role {
    if args.iter().any(|a| a == "--uninstall") {
        return Role::Uninstall;
    }
    if args.iter().any(|a| a == "--update") {
        return Role::Update;
    }
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if stem.starts_with("uninstall") || stem.starts_with("se-uninstall") {
        Role::Uninstall
    } else if stem.starts_with("update") {
        Role::Update
    } else {
        Role::Install
    }
}

#[cfg(windows)]
fn main() {
    win::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Simple Editor setup runs on Windows.");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn role_follows_the_file_name_and_flags() {
        let none: Vec<String> = vec![];
        assert_eq!(role(&none, Path::new("simple-editor-setup.exe")), Role::Install);
        assert_eq!(role(&none, Path::new("Update.exe")), Role::Update);
        assert_eq!(role(&none, Path::new("uninstall.exe")), Role::Uninstall);
        assert_eq!(role(&["--uninstall".into()], Path::new("se-uninstall-1234.exe")), Role::Uninstall);
        assert_eq!(role(&["--update".into()], Path::new("simple-editor-setup.exe")), Role::Update);
    }
}
