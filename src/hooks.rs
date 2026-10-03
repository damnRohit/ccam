//! ccam's Claude Code hooks: installing them into `settings.json`, and handling them when they fire.
//!
//! ccam's entries are recognised by shape (an exec-form command named `ccam` whose first argument is `hook`),
//! so reinstalling replaces them in place and uninstalling never touches hooks the user wrote.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::jsonio::{self, Style};
use crate::manager::{Manager, Network};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Event {
    SessionStart,
    FileChanged,
    AuthSuccess,
    /// A Claude Code turn ended: the only moment usage changed, and a safe point to switch.
    TurnEnd,
    /// A request failed on the usage limit.
    LimitHit,
}

struct Trigger {
    settings_key: &'static str,
    matcher: Option<&'static str>,
    arg: &'static str,
    /// SessionStart must run in the foreground because its output registers the file to watch. The rest only
    /// sync, so they run in the background and never delay the session.
    background: bool,
}

const TRIGGERS: [Trigger; 3] = [
    Trigger { settings_key: "SessionStart", matcher: Some("startup|resume"), arg: "session-start", background: false },
    Trigger { settings_key: "FileChanged", matcher: None, arg: "file-changed", background: true },
    Trigger { settings_key: "Notification", matcher: Some("auth_success"), arg: "auth-success", background: true },
];

/// Signals for `ccam auto`. Stop fires after every turn, so these are installed only when auto is wanted; each
/// handler just touches a file.
const AUTO_TRIGGERS: [Trigger; 2] = [
    Trigger { settings_key: "Stop", matcher: None, arg: "turn-end", background: true },
    Trigger { settings_key: "StopFailure", matcher: Some("rate_limit"), arg: "limit-hit", background: true },
];

/// Installs the hooks. `config_dir` is baked into the arguments because Claude Code can strip
/// `CLAUDE_CONFIG_DIR` from hook environments (`CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`).
pub fn install(settings: &Path, exe: &Path, config_dir: Option<&str>, auto: bool) -> Result<bool> {
    let exe = exe.to_string_lossy();
    let auto_triggers: &[Trigger] = if auto { &AUTO_TRIGGERS } else { &[] };
    edit_hooks(settings, |hooks| {
        remove_ccam_entries(hooks);
        for trigger in TRIGGERS.iter().chain(auto_triggers) {
            let mut args = vec!["hook".to_string(), trigger.arg.to_string()];
            if let Some(dir) = config_dir {
                args.extend(["--config-dir".to_string(), dir.to_string()]);
            }
            let mut handler = json!({"type": "command", "command": exe, "args": args});
            if trigger.background {
                handler["async"] = true.into();
            }
            let mut group = Map::new();
            if let Some(matcher) = trigger.matcher {
                group.insert("matcher".into(), matcher.into());
            }
            group.insert("hooks".into(), json!([handler]));
            hooks
                .entry(trigger.settings_key)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("hook events were checked to be arrays")
                .push(Value::Object(group));
        }
    })
}

pub fn uninstall(settings: &Path) -> Result<bool> {
    if !settings.exists() {
        return Ok(false);
    }
    edit_hooks(settings, remove_ccam_entries)
}

/// Runs a fired hook. Returns the JSON to print, if the event expects any.
///
/// Hooks must never break a Claude Code session, so failures are logged and swallowed by the caller.
pub fn handle(event: Event, input: &Value, manager: &Manager) -> Result<Option<Value>> {
    let files = crate::autorun::Files::of(manager);
    let signal = match event {
        Event::TurnEnd => Some(&files.turn_end),
        Event::LimitHit => Some(&files.limit_hit),
        _ => None,
    };
    if let Some(path) = signal {
        crate::autorun::mark(path).map_err(|e| crate::error::Error::io(path, e))?;
        return Ok(None);
    }

    let credentials = &manager.paths().credentials_file;
    let network = match event {
        // The foreground hook delays the first reply; confirming a brand-new login waits for a later trigger.
        Event::SessionStart => Network::Offline,
        _ => Network::Allowed,
    };
    let relevant = match event {
        Event::FileChanged => input["file_path"].as_str().is_some_and(|p| same_file(Path::new(p), credentials)),
        _ => true,
    };
    if relevant {
        manager.sync(network)?;
    }

    Ok((event == Event::SessionStart && credentials.exists()).then(|| {
        json!({"hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "watchPaths": [credentials.to_string_lossy()],
        }})
    }))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

fn edit_hooks(settings: &Path, edit: impl FnOnce(&mut Map<String, Value>)) -> Result<bool> {
    jsonio::edit_object(settings, Style::Pretty, false, |doc| {
        let mut hooks = match doc.shift_remove("hooks") {
            Some(Value::Object(map)) => map,
            None | Some(Value::Null) => Map::new(),
            Some(_) => return Err(Error::Malformed { path: settings.into(), reason: "\"hooks\" is not an object" }),
        };
        if hooks.values().any(|groups| !groups.is_array()) {
            return Err(Error::Malformed { path: settings.into(), reason: "a hook event is not a list" });
        }
        edit(&mut hooks);
        if !hooks.is_empty() {
            doc.insert("hooks".into(), Value::Object(hooks));
        }
        Ok(())
    })
}

fn remove_ccam_entries(hooks: &mut Map<String, Value>) {
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else { continue };
        // Drop a group only when removing ccam's handlers emptied it; the user's own empty groups stay.
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
            let had_handlers = !handlers.is_empty();
            handlers.retain(|h| !is_ccam_handler(h));
            !(had_handlers && handlers.is_empty())
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));
}

fn is_ccam_handler(handler: &Value) -> bool {
    // Split on both separators by hand: settings written on Windows use backslashes that Unix paths ignore.
    let named_ccam = handler.get("command").and_then(Value::as_str).is_some_and(|command| {
        let file = command.rsplit(['/', '\\']).next().unwrap_or(command).to_ascii_lowercase();
        file == "ccam" || file == "ccam.exe"
    });
    let hook_args = handler.get("args").and_then(|a| a.get(0)).and_then(Value::as_str) == Some("hook");
    named_ccam && hook_args
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn read(path: &Path) -> Value {
        jsonio::read(path).unwrap().unwrap()
    }

    #[test]
    fn install_adds_the_three_sync_triggers() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");

        assert!(install(&settings, Path::new("/bin/ccam"), None, false).unwrap());

        assert_eq!(
            read(&settings),
            json!({"hooks": {
                "SessionStart": [{"matcher": "startup|resume", "hooks": [
                    {"type": "command", "command": "/bin/ccam", "args": ["hook", "session-start"]}]}],
                "FileChanged": [{"hooks": [
                    {"type": "command", "command": "/bin/ccam", "args": ["hook", "file-changed"], "async": true}]}],
                "Notification": [{"matcher": "auth_success", "hooks": [
                    {"type": "command", "command": "/bin/ccam", "args": ["hook", "auth-success"], "async": true}]}],
            }})
        );
    }

    #[test]
    fn install_with_auto_adds_the_turn_end_and_limit_hit_signals() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");

        install(&settings, Path::new("/bin/ccam"), None, true).unwrap();

        let hooks = &read(&settings)["hooks"];
        assert_eq!(
            hooks["Stop"],
            json!([{"hooks": [{"type": "command", "command": "/bin/ccam", "args": ["hook", "turn-end"], "async": true}]}])
        );
        assert_eq!(
            hooks["StopFailure"],
            json!([{"matcher": "rate_limit", "hooks": [{"type": "command", "command": "/bin/ccam", "args": ["hook", "limit-hit"], "async": true}]}])
        );

        // Reinstalling without auto removes them again.
        install(&settings, Path::new("/bin/ccam"), None, false).unwrap();
        assert!(read(&settings)["hooks"].get("Stop").is_none());
    }

    #[test]
    fn install_records_a_relocated_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        install(&settings, Path::new("/bin/ccam"), Some("/work/claude"), false).unwrap();

        assert_eq!(
            read(&settings)["hooks"]["FileChanged"][0]["hooks"][0]["args"],
            json!(["hook", "file-changed", "--config-dir", "/work/claude"])
        );
    }

    #[test]
    fn install_keeps_unrelated_settings_and_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let user_hook = json!({"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]});
        fs::write(
            &settings,
            json!({"model": "opus", "hooks": {"SessionStart": [user_hook.clone()], "Stop": [{"hooks": []}]}})
                .to_string(),
        )
        .unwrap();

        install(&settings, Path::new("/bin/ccam"), None, false).unwrap();

        let after = read(&settings);
        assert_eq!(after["model"], "opus");
        assert_eq!(after["hooks"]["Stop"], json!([{"hooks": []}]));
        assert_eq!(after["hooks"]["SessionStart"][0], user_hook);
        assert_eq!(after["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn install_refuses_malformed_hook_settings_instead_of_skipping_a_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        fs::write(&settings, r#"{"hooks":{"FileChanged":{"not":"a list"}}}"#).unwrap();

        assert!(matches!(install(&settings, Path::new("/bin/ccam"), None, false), Err(Error::Malformed { .. })));
        assert_eq!(fs::read_to_string(&settings).unwrap(), r#"{"hooks":{"FileChanged":{"not":"a list"}}}"#);
    }

    #[test]
    fn install_is_idempotent_and_follows_a_moved_binary() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        install(&settings, Path::new("/old/ccam"), None, false).unwrap();
        let first = fs::read_to_string(&settings).unwrap();

        assert!(!install(&settings, Path::new("/old/ccam"), None, false).unwrap());
        assert_eq!(fs::read_to_string(&settings).unwrap(), first);

        assert!(install(&settings, Path::new("/new/ccam.exe"), None, false).unwrap());
        let text = fs::read_to_string(&settings).unwrap();
        assert!(!text.contains("/old/ccam") && text.matches("/new/ccam.exe").count() == 3);
    }

    #[test]
    fn uninstall_removes_only_ccam_entries() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        let user_hook = json!({"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]});
        fs::write(&settings, json!({"hooks": {"SessionStart": [user_hook.clone()]}}).to_string()).unwrap();
        install(&settings, Path::new("C:\\tools\\ccam.exe"), None, false).unwrap();

        assert!(uninstall(&settings).unwrap());
        assert_eq!(read(&settings), json!({"hooks": {"SessionStart": [user_hook]}}));
        assert!(!uninstall(&settings).unwrap());
    }

    #[test]
    fn uninstall_without_a_settings_file_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        assert!(!uninstall(&settings).unwrap());
        assert!(!settings.exists());
    }
}
