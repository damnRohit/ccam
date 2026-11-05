//! Lanes: Claude Code config dirs that each hold one account, so the sessions started in a lane stay on it while
//! other terminals use other accounts.
//!
//! A lane lives at `<main config home>/ccam/lanes/<id>/` and is an install of its own (section 9.7 of the SPEC):
//! its login is checked out of the vault with a normal switch, its presence record keeps every other install off
//! the account, and its hooks sync renewals back. History, plugins and skills are links to the main config home,
//! and the vault is a link to the main vault, so a lane sees the same conversations and accounts.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::hooks;
use crate::name::{AccountName, InvalidName};
use crate::paths::Paths;

/// Which lane: a number handed out in order, or a name the user chose. All-digit text is always a number, so a
/// name can never shadow one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneId {
    Number(u16),
    Name(AccountName),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "invalid lane {0:?}: use a number from 1, or a name that starts with a letter or digit, is not only digits, and uses letters, digits, '.', '_', '@' or '-' (max 64)"
)]
pub struct InvalidLane(pub String);

impl FromStr for LaneId {
    type Err = InvalidLane;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let invalid = || InvalidLane(s.to_string());
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
            // Leading zeros would give one lane two spellings, and two directories.
            return match s.parse::<u16>() {
                Ok(n) if n > 0 && !s.starts_with('0') => Ok(Self::Number(n)),
                _ => Err(invalid()),
            };
        }
        s.parse().map(Self::Name).map_err(|InvalidName(_)| invalid())
    }
}

impl fmt::Display for LaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::Name(name) => write!(f, "{name}"),
        }
    }
}

const LANES_DIR: &str = "lanes";

/// The oldest Claude Code lanes run on. 2.1.280 stopped Windows cleanup from deleting a junctioned folder (here,
/// the shared history), and 2.1.296 fixed synced skills when several sessions share a config dir.
pub const MIN_CLAUDE: (u32, u32, u32) = (2, 1, 296);

/// Directories shared with the main config home. Transcripts live in `projects`, so `/resume` sees the same
/// conversations in every lane; `file-history` keeps `/rewind` working for a conversation resumed in a lane.
const SHARED_DIRS: [&str; 8] =
    ["projects", "file-history", "plans", "plugins", "skills", "commands", "agents", "output-styles"];

/// Files copied from the main config home, besides the settings, so edits there reach lanes.
const COPIED_FILES: [&str; 2] = ["CLAUDE.md", "keybindings.json"];

/// The lane a config home is, when it is one.
pub fn lane_of(config_home: &Path) -> Option<String> {
    let lanes = config_home.parent()?;
    let is_lanes = lanes.file_name()? == LANES_DIR && lanes.parent()?.file_name()? == "ccam";
    is_lanes.then(|| config_home.file_name()?.to_str().map(str::to_string))?
}

fn root(main: &Paths) -> PathBuf {
    main.config_home.join("ccam").join(LANES_DIR)
}

/// Every lane, numbers first in order, then names.
pub fn all(main: &Paths) -> Vec<LaneId> {
    let Ok(entries) = fs::read_dir(root(main)) else { return Vec::new() };
    let mut lanes: Vec<LaneId> =
        entries.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().to_str()?.parse().ok()).collect();
    lanes.sort_by_key(|id| match id {
        LaneId::Number(n) => (0, *n, String::new()),
        LaneId::Name(name) => (1, 0, name.as_str().to_ascii_lowercase()),
    });
    lanes
}

/// The lane `id` names, spelled as it is stored, when it exists.
pub fn find(main: &Paths, id: &LaneId) -> Option<LaneId> {
    all(main).into_iter().find(|lane| lane == id)
}

/// The lowest number no lane uses.
pub fn next_free(lanes: &[LaneId]) -> LaneId {
    let taken = |n: &u16| lanes.contains(&LaneId::Number(*n));
    LaneId::Number((1..=u16::MAX).find(|n| !taken(n)).unwrap_or(u16::MAX))
}

/// The lane's own paths. `id` must be the stored spelling, since it names a directory.
pub fn paths(main: &Paths, id: &LaneId) -> Result<Paths> {
    let dir = root(main).join(id.to_string());
    Paths::for_config_dir(Some(dir.to_string_lossy().into_owned()))
}

/// Creates the lane's config home, or brings it up to date with the main one. Safe to rerun while sessions in the
/// lane are open: links that exist are kept, and files are replaced atomically.
///
/// A lane directory is never deleted: removing it would follow its links into the main history and vault.
pub fn prepare(main: &Paths, lane: &Paths, exe: &Path) -> Result<()> {
    let home = &lane.config_home;
    let vault_parent = home.join("ccam");
    fs::create_dir_all(&vault_parent).map_err(|e| Error::io(&vault_parent, e))?;
    link_dir(&main.vault_dir, &lane.vault_dir)?;
    for dir in SHARED_DIRS {
        link_dir(&main.config_home.join(dir), &home.join(dir))?;
    }
    refresh_copies(main, lane, exe)?;

    // Onboarding state, theme and project trust come from the main config the first time. The identity does not:
    // the switch that checks the account out writes it.
    if !lane.global_config.exists() {
        let mut config = crate::jsonio::read(&main.global_config)?.unwrap_or_else(|| json!({}));
        if let Some(map) = config.as_object_mut() {
            map.shift_remove("oauthAccount");
        }
        crate::jsonio::write_atomic(&lane.global_config, &config, crate::jsonio::Style::Pretty, false)?;
    }
    Ok(())
}

/// Brings the lane's copies of the main settings, `CLAUDE.md` and `keybindings.json` up to date, with hooks that
/// sync the lane's login in place of main's. Run on every `ccam run` and lane session start, so edits in the main
/// config home reach lanes that ccam never launches (T3 Code's). Files are replaced only when they changed.
pub fn refresh_copies(main: &Paths, lane: &Paths, exe: &Path) -> Result<()> {
    for file in COPIED_FILES {
        copy_if_present(&main.config_home.join(file), &lane.config_home.join(file))?;
    }
    hooks::install_copy(&main.settings_file, &lane.settings_file, exe, lane.config_dir_env.as_deref())?;
    Ok(())
}

/// The main config home's paths, for a lane's. `home` is the user's home directory, which decides whether the
/// main config home is the default one.
pub fn main_of(lane: &Paths, home: &Path) -> Option<Paths> {
    lane_of(&lane.config_home)?;
    let main_home = lane.config_home.parent()?.parent()?.parent()?;
    let config_dir = (main_home != home.join(".claude")).then(|| main_home.to_string_lossy().into_owned());
    Some(Paths::resolve(home, config_dir))
}

/// Writes the settings a lane session in `project` launches with, and returns their file for `--settings`.
///
/// Auto memory stays the main login's. Reached through the lane's `projects` link, every memory read and write
/// would ask for approval (anthropics/claude-code#98044), so memory is pointed at the real folder, and a read rule
/// lets Claude list it without asking. `None` when the folder cannot be named, which leaves memory asking.
pub fn launch_settings(main: &Paths, lane: &Paths, project: &Path) -> Result<Option<PathBuf>> {
    let Some(slug) = project_slug(&main.config_home.join("projects"), project) else { return Ok(None) };
    let memory = main.config_home.join("projects").join(&slug).join("memory");
    let settings = json!({
        "autoMemoryDirectory": memory,
        "permissions": {"allow": [read_rule(&memory)]},
    });
    let file = lane.config_home.join("ccam").join("settings").join(format!("{slug}.json"));
    crate::jsonio::write_atomic(&file, &settings, crate::jsonio::Style::Pretty, false)?;
    Ok(Some(file))
}

/// The folder name Claude Code gives `project` under `projects/`: every character but ASCII letters and digits
/// becomes `-`. Claude Code shortens names over 200 characters with a hash ccam cannot reproduce, so a long name
/// is found among the existing folders instead.
fn project_slug(projects: &Path, project: &Path) -> Option<String> {
    let slug: String =
        project.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    if slug.len() <= 200 {
        return Some(slug);
    }
    let prefix = format!("{}-", &slug[..200]);
    let mut matches = fs::read_dir(projects).ok()?.flatten().filter_map(|e| e.file_name().into_string().ok());
    let found = matches.find(|name| name.starts_with(&prefix))?;
    matches.all(|name| !name.starts_with(&prefix)).then_some(found)
}

/// A permission rule for reading anything under `dir`, in Claude Code's absolute form: `//c/Users/...` on
/// Windows, `//home/...` elsewhere.
fn read_rule(dir: &Path) -> String {
    let path = dir.to_string_lossy().replace('\\', "/");
    let absolute = match path.as_bytes() {
        [drive, b':', ..] => format!("/{}{}", (*drive as char).to_ascii_lowercase(), &path[2..]),
        _ => path,
    };
    format!("Read(/{absolute}/**)")
}

/// The repository `cwd` belongs to, which is how Claude Code picks the project folder (worktrees share their main
/// repository's). Outside a repository, `cwd` itself.
pub fn project_root(cwd: &Path) -> PathBuf {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(cwd)
        .output();
    let common =
        output.ok().filter(|o| o.status.success()).map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    match common {
        Some(git) if git.file_name().is_some_and(|n| n == ".git") => git.parent().map(Path::to_path_buf).unwrap_or(git),
        _ => cwd.to_path_buf(),
    }
}

/// `x.y.z` from `claude --version` output such as `2.1.296 (Claude Code)`.
pub fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    let mut parts = output.split_whitespace().next()?.split('.').map(|p| p.parse().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// A Claude Code session that `claude agents --json` reports for one config dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub pid: Option<u64>,
    pub session_id: String,
    pub cwd: String,
    /// `busy`, `waiting` or `idle` for an interactive session; the job state of a background one.
    pub status: String,
}

/// Parses `claude agents --json`. Every entry counts, background ones included: they run on the lane's login too.
pub fn parse_sessions(output: &str) -> std::result::Result<Vec<Session>, String> {
    let doc: Value = serde_json::from_str(output.trim()).map_err(|e| format!("unexpected output: {e}"))?;
    let entries = doc.as_array().ok_or("unexpected output: not a list")?;
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_string();
    Ok(entries
        .iter()
        .map(|s| Session {
            pid: s["pid"].as_u64(),
            session_id: text(s, "sessionId"),
            cwd: text(s, "cwd"),
            status: s["status"].as_str().or(s["state"].as_str()).unwrap_or("unknown").to_string(),
        })
        .collect())
}

fn copy_if_present(from: &Path, to: &Path) -> Result<()> {
    let Some(doc) = fs::read(from).ok() else { return Ok(()) };
    if fs::read(to).is_ok_and(|current| current == doc) {
        return Ok(());
    }
    crate::jsonio::write_atomic_bytes(to, &doc, false)
}

/// Points `link` at `target`. An existing entry at `link` is left alone: it is either the link from an earlier run
/// or a real directory Claude Code made, and replacing a real directory could lose what is in it.
fn link_dir(target: &Path, link: &Path) -> Result<()> {
    if fs::symlink_metadata(link).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(target).map_err(|e| Error::io(target, e))?;
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).map_err(|e| Error::io(link, e));
    #[cfg(windows)]
    {
        // A junction, unlike a directory symlink, needs neither admin rights nor Developer Mode. mklink reads a
        // forward slash as the start of a switch, and CLAUDE_CONFIG_DIR may be written with them.
        let native = |p: &Path| p.to_string_lossy().replace('/', "\\");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .output()
            .map_err(|e| Error::io(link, e))?;
        if output.status.success() {
            Ok(())
        } else {
            let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(Error::io(link, std::io::Error::other(format!("cannot link to {}: {reason}", target.display()))))
        }
    }
}

/// Safely deletes a lane directory. It unlinks every shared junction or symlink first, so the target directories
/// in the main config home and vault are never touched.
pub(crate) fn remove(main: &Paths, id: &LaneId, locks: &crate::manager::SwitchLocks) -> Result<()> {
    locks.check()?;
    let lane = paths(main, id)?;
    let home = &lane.config_home;
    if !home.exists() {
        return Ok(());
    }
    unlink_dir(&lane.vault_dir);
    for dir in SHARED_DIRS {
        unlink_dir(&home.join(dir));
    }
    #[cfg(windows)]
    if let Ok(entries) = fs::read_dir(home) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                use std::os::windows::fs::MetadataExt;
                if (meta.file_attributes() & 0x400) != 0 {
                    let _ = fs::remove_dir(entry.path());
                }
            }
        }
    }
    fs::remove_dir_all(home).map_err(|e| Error::io(home, e))
}

fn unlink_dir(path: &Path) {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return;
    }
    #[cfg(unix)]
    {
        let _ = fs::remove_file(path).or_else(|_| fs::remove_dir(path));
    }
    #[cfg(windows)]
    {
        let _ = fs::remove_dir(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> LaneId {
        s.parse().unwrap()
    }

    #[test]
    fn digits_are_lane_numbers_and_anything_else_is_a_lane_name() {
        assert_eq!(id("2"), LaneId::Number(2));
        assert_eq!(id("api").to_string(), "api");
        assert_eq!(id("ui-2").to_string(), "ui-2");
    }

    #[test]
    fn a_lane_name_can_never_be_mistaken_for_a_number() {
        for bad in ["0", "007", "", "-x", "a b", "a/b", "..", "99999999"] {
            assert!(bad.parse::<LaneId>().is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn lane_names_compare_without_case() {
        assert_eq!(id("API"), id("api"));
        assert_ne!(id("1"), id("api"));
    }

    #[test]
    fn a_lane_is_recognised_by_where_it_lives() {
        let main = Paths::resolve(Path::new("/h"), None);
        let lane = paths(&main, &id("work")).unwrap();
        assert_eq!(lane_of(&lane.config_home).as_deref(), Some("work"));
        assert_eq!(lane_of(&main.config_home), None);
        assert_eq!(lane_of(Path::new("/h/other/lanes/work")), None);
    }

    #[test]
    fn new_lanes_take_the_lowest_free_number_and_names_sort_after_numbers() {
        let home = tempfile::tempdir().unwrap();
        let main = Paths::resolve(home.path(), None);
        assert_eq!(next_free(&all(&main)), LaneId::Number(1));
        for dir in ["3", "api", "1", "007", "Docs"] {
            fs::create_dir_all(root(&main).join(dir)).unwrap();
        }
        let lanes = all(&main);
        assert_eq!(lanes, ["1", "3", "api", "docs"].map(id), "a directory that is not a lane id is ignored");
        assert_eq!(next_free(&lanes), LaneId::Number(2));
        assert_eq!(find(&main, &id("DOCS")).unwrap().to_string(), "Docs");
        assert_eq!(find(&main, &id("2")), None);
    }

    #[test]
    fn preparing_shares_history_and_the_vault_and_installs_hooks_for_the_lane() {
        let home = tempfile::tempdir().unwrap();
        let main = Paths::resolve(home.path(), None);
        fs::create_dir_all(main.vault_dir.join("a")).unwrap();
        fs::create_dir_all(main.config_home.join("projects").join("p")).unwrap();
        fs::write(&main.settings_file, r#"{"model":"opus"}"#).unwrap();
        fs::write(&main.global_config, r#"{"theme":"dark","oauthAccount":{"accountUuid":"A"}}"#).unwrap();
        let lane = Paths::resolve(home.path(), Some(root(&main).join("b").to_string_lossy().into()));

        prepare(&main, &lane, Path::new("/bin/ccam")).unwrap();
        prepare(&main, &lane, Path::new("/bin/ccam")).unwrap();

        assert!(lane.vault_dir.join("a").is_dir());
        assert!(lane.config_home.join("projects").join("p").is_dir());
        assert!(main.config_home.join("file-history").is_dir(), "shared folders are created in main when missing");
        let settings = crate::jsonio::read(&lane.settings_file).unwrap().unwrap();
        assert_eq!(settings["model"], "opus");
        let args = settings["hooks"]["SessionStart"][0]["hooks"][0]["args"].to_string();
        assert!(args.contains("--config-dir") && args.contains("lanes"), "{args}");
        let config = crate::jsonio::read(&lane.global_config).unwrap().unwrap();
        assert_eq!(config, json!({"theme": "dark"}));
    }

    #[test]
    fn a_lane_knows_its_main_config_home() {
        let home = Path::new("/h");
        let main = Paths::resolve(home, None);
        let lane = paths(&main, &id("2")).unwrap();
        let found = main_of(&lane, home).unwrap();
        assert_eq!(found.config_home, main.config_home);
        assert_eq!(found.global_config, main.global_config);
        assert_eq!(found.config_dir_env, None);

        let relocated = Paths::resolve(home, Some("/work/claude".into()));
        let found = main_of(&paths(&relocated, &id("api")).unwrap(), home).unwrap();
        assert_eq!(found.config_home, relocated.config_home);
        assert_eq!(found.global_config, relocated.global_config);

        assert!(main_of(&main, home).is_none(), "the main config home is not a lane");
    }

    #[test]
    fn refreshing_picks_up_edited_main_settings_and_keeps_the_lanes_hooks() {
        let home = tempfile::tempdir().unwrap();
        let main = Paths::resolve(home.path(), None);
        fs::create_dir_all(&main.config_home).unwrap();
        fs::write(&main.settings_file, r#"{"model":"opus"}"#).unwrap();
        hooks::install(&main.settings_file, Path::new("/bin/ccam"), None, false).unwrap();
        let lane = Paths::resolve(home.path(), Some(root(&main).join("b").to_string_lossy().into()));
        prepare(&main, &lane, Path::new("/bin/ccam")).unwrap();
        unlink_dir(&lane.config_home.join("agents"));

        let mut edited = crate::jsonio::read(&main.settings_file).unwrap().unwrap();
        edited["model"] = json!("sonnet");
        crate::jsonio::write_atomic(&main.settings_file, &edited, crate::jsonio::Style::Pretty, false).unwrap();
        fs::write(main.config_home.join("CLAUDE.md"), "be brief").unwrap();
        refresh_copies(&main, &lane, Path::new("/bin/ccam")).unwrap();

        let settings = crate::jsonio::read(&lane.settings_file).unwrap().unwrap();
        assert_eq!(settings["model"], "sonnet");
        let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 1, "main's own ccam hooks are replaced, not added: {settings}");
        let args = session_start[0]["hooks"][0]["args"].to_string();
        assert!(args.contains("--config-dir") && args.contains("lanes"), "{args}");
        assert_eq!(fs::read_to_string(lane.config_home.join("CLAUDE.md")).unwrap(), "be brief");
        assert!(fs::symlink_metadata(lane.config_home.join("agents")).is_err(), "a refresh makes no links");

        let before = fs::metadata(&lane.settings_file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        refresh_copies(&main, &lane, Path::new("/bin/ccam")).unwrap();
        let after = fs::metadata(&lane.settings_file).unwrap().modified().unwrap();
        assert_eq!(before, after, "an unchanged refresh does not rewrite the settings sessions are reading");
    }

    #[test]
    fn a_copy_is_rewritten_only_when_its_source_changed_and_keeps_its_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let (source, copy) = (dir.path().join("main.md"), dir.path().join("CLAUDE.md"));
        copy_if_present(&source, &copy).unwrap();
        assert!(!copy.exists(), "a missing source makes no copy");

        fs::write(&source, "be brief").unwrap();
        copy_if_present(&source, &copy).unwrap();
        assert_eq!(fs::read_to_string(&copy).unwrap(), "be brief");
        let before = fs::metadata(&copy).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        copy_if_present(&source, &copy).unwrap();
        assert_eq!(fs::metadata(&copy).unwrap().modified().unwrap(), before, "an unchanged copy is not rewritten");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&copy, fs::Permissions::from_mode(0o600)).unwrap();
            fs::write(&source, "be thorough").unwrap();
            copy_if_present(&source, &copy).unwrap();
            assert_eq!(fs::read_to_string(&copy).unwrap(), "be thorough");
            assert_eq!(fs::metadata(&copy).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn concurrent_refreshes_never_expose_a_torn_copy_or_leave_temp_files() {
        const SIZE: usize = 200_000;
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("CLAUDE.md");
        let sources: Vec<PathBuf> = (0..4u8)
            .map(|i| {
                let source = dir.path().join(format!("source-{i}"));
                fs::write(&source, vec![b'a' + i; SIZE]).unwrap();
                source
            })
            .collect();

        std::thread::scope(|s| {
            let writers: Vec<_> = sources
                .iter()
                .map(|source| {
                    let copy = &copy;
                    s.spawn(move || (0..30).try_for_each(|_| copy_if_present(source, copy)))
                })
                .collect();
            while !writers.iter().all(|w| w.is_finished()) {
                if let Ok(doc) = fs::read(&copy) {
                    assert!(doc.len() == SIZE && doc.iter().all(|b| *b == doc[0]), "torn copy of {} bytes", doc.len());
                }
            }
            for w in writers {
                w.join().unwrap().unwrap();
            }
        });
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), sources.len() + 1, "temp files must not be left behind");
    }

    #[test]
    fn removing_a_lane_unlinks_junctions_and_leaves_main_history_and_vault_intact() {
        let home = tempfile::tempdir().unwrap();
        let main = Paths::resolve(home.path(), None);
        fs::create_dir_all(main.vault_dir.join("a")).unwrap();
        fs::create_dir_all(main.config_home.join("projects").join("p")).unwrap();
        let lane = Paths::resolve(home.path(), Some(root(&main).join("b").to_string_lossy().into()));

        prepare(&main, &lane, Path::new("/bin/ccam")).unwrap();
        assert!(lane.config_home.exists());
        assert!(lane.vault_dir.join("a").is_dir());

        let manager = |paths: &Paths| {
            let endpoint = "http://127.0.0.1:9".to_string();
            let api = crate::api::Api::new(
                crate::api::Endpoints {
                    usage: endpoint.clone(),
                    profile: endpoint.clone(),
                    token: endpoint.clone(),
                    organizations: endpoint,
                },
                std::time::Duration::from_millis(1),
            );
            crate::manager::Manager::new(
                paths.clone(),
                Box::new(crate::store::FileStore::new(paths.credentials_file.clone())),
                Box::new(api),
            )
        };
        assert!(manager(&main).remove_lane(&manager(&lane), &id("b")).unwrap());
        assert!(!lane.config_home.exists(), "lane directory must be deleted");
        assert!(main.vault_dir.join("a").is_dir(), "main vault must remain intact");
        assert!(!main.vault_lock.exists(), "the main vault lock must be released after unlinking the lane");
        assert!(main.config_home.join("projects").join("p").is_dir(), "main projects must remain intact");
    }

    #[test]
    fn memory_is_the_main_logins_reached_without_the_link() {
        let home = tempfile::tempdir().unwrap();
        let main = Paths::resolve(home.path(), None);
        let lane = Paths::resolve(home.path(), Some(root(&main).join("1").to_string_lossy().into()));
        let project = Path::new(r"C:\Users\Me Too\src\app.v2");

        let file = launch_settings(&main, &lane, project).unwrap().unwrap();

        let settings = crate::jsonio::read(&file).unwrap().unwrap();
        let memory = main.config_home.join("projects").join("C--Users-Me-Too-src-app-v2").join("memory");
        assert_eq!(settings["autoMemoryDirectory"], json!(memory));
        assert_eq!(settings["permissions"]["allow"][0], read_rule(&memory));
        assert!(file.starts_with(&lane.config_home));
    }

    #[test]
    fn read_rules_use_claude_codes_absolute_form() {
        assert_eq!(read_rule(Path::new(r"C:\Users\x\memory")), "Read(//c/Users/x/memory/**)");
        assert_eq!(read_rule(Path::new("/home/x/memory")), "Read(//home/x/memory/**)");
    }

    #[test]
    fn a_long_project_name_is_found_among_the_existing_folders_or_left_alone() {
        let projects = tempfile::tempdir().unwrap();
        let long = Path::new("/").join("d".repeat(240));
        assert_eq!(project_slug(projects.path(), &long), None);
        let stored = format!("-{}-k3j2", &"d".repeat(240)[..199]);
        fs::create_dir(projects.path().join(&stored)).unwrap();
        assert_eq!(project_slug(projects.path(), &long), Some(stored));
    }

    #[test]
    fn versions_and_sessions_are_read_from_claudes_output() {
        assert_eq!(parse_version("2.1.296 (Claude Code)\n"), Some((2, 1, 296)));
        assert_eq!(parse_version("garbage"), None);
        assert!(parse_version("2.1.279 (Claude Code)").unwrap() < MIN_CLAUDE);

        let out = r#"[{"kind":"interactive","pid":12,"sessionId":"s1","cwd":"/a","status":"busy"},
                      {"kind":"background","id":"j","sessionId":"s2","cwd":"/b","state":"blocked"}]"#;
        let sessions = parse_sessions(out).unwrap();
        assert_eq!(
            sessions[0],
            Session { pid: Some(12), session_id: "s1".into(), cwd: "/a".into(), status: "busy".into() }
        );
        assert_eq!((sessions[1].pid, sessions[1].status.as_str()), (None, "blocked"));
        assert!(parse_sessions("[]").unwrap().is_empty());
        assert!(parse_sessions("Error: not logged in").is_err());
    }
}
