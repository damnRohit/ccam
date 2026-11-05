//! T3 Code provider instances for lanes: one `claudeAgent` instance per lane in T3's `settings.json`.
//!
//! T3 watches that file and reloads it on every write, and a file that fails to parse blocks T3's own saves, so
//! the file is rewritten atomically and only when something changed. Layout notes: `docs/research/t3code.md`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::jsonio::{self, Style};
use crate::lane::LaneId;
use crate::name::AccountName;

const INSTANCES: &str = "providerInstances";
const ID_PREFIX: &str = "ccam-lane-";
/// T3 instance ids match `^[a-zA-Z][a-zA-Z0-9_-]*$` and are at most 64 long.
const ID_MAX: usize = 64;

pub struct LaneInstance {
    pub lane: LaneId,
    pub home: PathBuf,
    pub account: Option<AccountName>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sync {
    Unchanged,
    Updated,
}

/// The lane's instance id, one per lane. A lane id of only ASCII letters, digits and `-` is used as it is. Any
/// other is sanitised, shortened if needed, and ends in `_` plus a hash of the lane id: clean ids never contain
/// `_`, so the two forms cannot meet, and the hash keeps lanes that sanitise or shorten alike apart.
pub fn instance_id(lane: &LaneId) -> String {
    const HASH: usize = 8;
    let lane = lane.to_string();
    let id = format!("{ID_PREFIX}{lane}");
    if id.len() <= ID_MAX && lane.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return id;
    }
    let sanitised: String = lane.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let room = ID_MAX - ID_PREFIX.len() - 1 - HASH;
    let hash = hex::encode(Sha256::digest(lane.to_ascii_lowercase().as_bytes()));
    format!("{ID_PREFIX}{}_{}", &sanitised[..sanitised.len().min(room)], &hash[..HASH])
}

/// The name a lane's instance shows in T3's model picker.
pub fn display_name(lane: &LaneId, account: Option<&AccountName>) -> String {
    match account {
        Some(account) => format!("Claude · lane {lane} · {account}"),
        None => format!("Claude · lane {lane}"),
    }
}

/// Makes T3's provider instances match `lanes`: one `ccam-lane-*` instance per lane, none for lanes that are gone.
/// Only `driver`, `displayName`, `enabled` and `config.homePath` of a lane instance are ccam's; everything else
/// in the document is left as it is, in its order. A `providerInstances` that is not an object is left alone.
/// Returns whether anything changed.
pub fn reconcile(doc: &mut Map<String, Value>, lanes: &[LaneInstance]) -> bool {
    if lanes.is_empty() && !doc.contains_key(INSTANCES) {
        return false;
    }
    let Value::Object(instances) = doc.entry(INSTANCES).or_insert_with(|| json!({})) else {
        return false;
    };
    let before = instances.clone();

    let wanted: Vec<String> = lanes.iter().map(|l| instance_id(&l.lane)).collect();
    instances.retain(|id, _| !id.starts_with(ID_PREFIX) || wanted.contains(id));
    for (id, lane) in wanted.into_iter().zip(lanes) {
        let entry = instances.entry(id).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        entry["driver"] = json!("claudeAgent");
        entry["displayName"] = json!(display_name(&lane.lane, lane.account.as_ref()));
        entry["enabled"] = json!(lane.account.is_some());
        let config = &mut entry["config"];
        if !config.is_object() {
            *config = json!({});
        }
        config["homePath"] = json!(lane.home.to_string_lossy());
    }
    *instances != before
}

/// T3's settings file, or `None` when T3 Code is not installed (no `userdata` directory).
pub fn settings_file() -> Option<PathBuf> {
    settings_file_for(std::env::var_os("T3CODE_HOME"), std::env::home_dir())
}

fn settings_file_for(t3code_home: Option<OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    let t3_home = match t3code_home.filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home?.join(".t3"),
    };
    let userdata = t3_home.join("userdata");
    userdata.is_dir().then(|| userdata.join("settings.json"))
}

/// Reconciles the settings file with `lanes`, writing it only when something changed. T3 re-reads the file before
/// its own saves, but a save landing between its read and its rename can drop ccam's edit, so the file is read
/// back and the edit redone once if it did not stick.
pub fn sync(file: &Path, lanes: &[LaneInstance]) -> Result<Sync> {
    const WRITES: usize = 2;
    for writes in 0..=WRITES {
        let mut doc = read_settings(file)?;
        if !reconcile(&mut doc, lanes) {
            return Ok(if writes == 0 { Sync::Unchanged } else { Sync::Updated });
        }
        if writes == WRITES {
            break;
        }
        jsonio::write_atomic(file, &Value::Object(doc), Style::Pretty, false)?;
    }
    Err(Error::Malformed { path: file.into(), reason: "T3 Code kept overwriting the lane instances; try again" })
}

fn read_settings(file: &Path) -> Result<Map<String, Value>> {
    let malformed = |reason| Error::Malformed { path: file.into(), reason };
    match jsonio::read(file)? {
        None => Ok(Map::new()),
        Some(Value::Object(doc)) => match doc.get(INSTANCES) {
            Some(instances) if !instances.is_object() => Err(malformed("providerInstances is not a JSON object")),
            _ => Ok(doc),
        },
        Some(_) => Err(malformed("top level is not a JSON object")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn lane(id: &str, account: Option<&str>) -> LaneInstance {
        LaneInstance {
            lane: id.parse().unwrap(),
            home: PathBuf::from(format!("/home/u/.claude/ccam/lanes/{id}")),
            account: account.map(|a| a.parse().unwrap()),
        }
    }

    fn object(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("not an object"),
        }
    }

    fn keys(map: &Value) -> Vec<&str> {
        map.as_object().unwrap().keys().map(String::as_str).collect()
    }

    fn user_settings() -> Value {
        json!({
            "projectSettingsFolded": true,
            "providerInstances": {
                "codex": {"driver": "codex", "enabled": true, "config": {"setupMode": "managed"}},
                "claudeAgent": {"driver": "claudeAgent", "enabled": true}
            },
            "idleCompaction": "onReturn"
        })
    }

    #[test]
    fn a_new_lane_gets_an_instance_of_the_exact_shape() {
        let mut doc = Map::new();
        assert!(reconcile(&mut doc, &[lane("1", Some("work")), lane("2", None)]));
        assert_eq!(
            Value::Object(doc),
            json!({"providerInstances": {
                "ccam-lane-1": {
                    "driver": "claudeAgent",
                    "displayName": "Claude · lane 1 · work",
                    "enabled": true,
                    "config": {"homePath": "/home/u/.claude/ccam/lanes/1"}
                },
                "ccam-lane-2": {
                    "driver": "claudeAgent",
                    "displayName": "Claude · lane 2",
                    "enabled": false,
                    "config": {"homePath": "/home/u/.claude/ccam/lanes/2"}
                }
            }})
        );
    }

    #[test]
    fn reconciling_twice_changes_nothing_the_second_time() {
        let mut doc = object(user_settings());
        let lanes = [lane("1", Some("work"))];
        assert!(reconcile(&mut doc, &lanes));
        let once = doc.clone();
        assert!(!reconcile(&mut doc, &lanes));
        assert_eq!(doc, once);
    }

    #[test]
    fn no_lanes_and_no_instances_leaves_the_document_alone() {
        let mut doc = object(json!({"projectSettingsFolded": true}));
        assert!(!reconcile(&mut doc, &[]));
        assert_eq!(Value::Object(doc), json!({"projectSettingsFolded": true}));
    }

    #[test]
    fn a_move_updates_the_display_name_and_a_release_disables_the_instance() {
        let mut doc = Map::new();
        reconcile(&mut doc, &[lane("1", Some("work"))]);

        assert!(reconcile(&mut doc, &[lane("1", Some("home"))]));
        assert_eq!(doc["providerInstances"]["ccam-lane-1"]["displayName"], "Claude · lane 1 · home");
        assert_eq!(doc["providerInstances"]["ccam-lane-1"]["enabled"], true);

        assert!(reconcile(&mut doc, &[lane("1", None)]));
        assert_eq!(doc["providerInstances"]["ccam-lane-1"]["displayName"], "Claude · lane 1");
        assert_eq!(doc["providerInstances"]["ccam-lane-1"]["enabled"], false);
    }

    #[test]
    fn a_deleted_lane_loses_its_instance_and_other_instances_stay() {
        let mut doc = object(user_settings());
        reconcile(&mut doc, &[lane("1", Some("work")), lane("2", None)]);

        assert!(reconcile(&mut doc, &[lane("2", None)]));
        assert_eq!(keys(&doc["providerInstances"]), ["codex", "claudeAgent", "ccam-lane-2"]);
    }

    #[test]
    fn user_instances_and_other_settings_are_untouched_and_keep_their_order() {
        let mut doc = object(user_settings());
        reconcile(&mut doc, &[lane("1", Some("work"))]);

        assert_eq!(keys(&Value::Object(doc.clone())), ["projectSettingsFolded", "providerInstances", "idleCompaction"]);
        assert_eq!(keys(&doc["providerInstances"]), ["codex", "claudeAgent", "ccam-lane-1"]);
        let original = user_settings();
        assert_eq!(doc["providerInstances"]["codex"], original["providerInstances"]["codex"]);
        assert_eq!(doc["providerInstances"]["claudeAgent"], original["providerInstances"]["claudeAgent"]);
        assert_eq!(doc["projectSettingsFolded"], true);
        assert_eq!(doc["idleCompaction"], "onReturn");
    }

    #[test]
    fn keys_added_to_a_lane_instance_by_the_user_or_t3_are_kept_in_place() {
        let mut doc = object(json!({"providerInstances": {"ccam-lane-1": {
            "driver": "claudeAgent",
            "displayName": "old",
            "accentColor": "#ff8800",
            "environment": [{"name": "X", "value": "1", "sensitive": false}],
            "enabled": false,
            "config": {"launchArgs": "--verbose", "homePath": "/stale"}
        }}}));

        assert!(reconcile(&mut doc, &[lane("1", Some("work"))]));
        let entry = &doc["providerInstances"]["ccam-lane-1"];
        assert_eq!(
            *entry,
            json!({
                "driver": "claudeAgent",
                "displayName": "Claude · lane 1 · work",
                "accentColor": "#ff8800",
                "environment": [{"name": "X", "value": "1", "sensitive": false}],
                "enabled": true,
                "config": {"launchArgs": "--verbose", "homePath": "/home/u/.claude/ccam/lanes/1"}
            })
        );
        assert_eq!(keys(entry), ["driver", "displayName", "accentColor", "environment", "enabled", "config"]);
        assert_eq!(keys(&entry["config"]), ["launchArgs", "homePath"]);
    }

    fn id_of(lane: &str) -> String {
        instance_id(&lane.parse().unwrap())
    }

    fn fits_t3(id: &str) -> bool {
        let mut chars = id.chars();
        id.len() <= ID_MAX
            && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }

    #[test]
    fn clean_lane_ids_keep_their_spelling() {
        assert_eq!(id_of("7"), "ccam-lane-7");
        assert_eq!(id_of("Work-2"), "ccam-lane-Work-2");
        assert_eq!(id_of(&"b".repeat(54)), format!("ccam-lane-{}", "b".repeat(54)));
    }

    #[test]
    fn lane_ids_that_sanitise_or_truncate_alike_get_distinct_instance_ids() {
        let ids = [
            "a.b",
            "a_b",
            "a@b",
            "a-b",
            &format!("b{}", "x".repeat(63)),
            &format!("b{}y", "x".repeat(62)),
            &format!("b{}", "x".repeat(53)),
            &format!("{}-0123abcd", "q".repeat(45)),
            &"q".repeat(64),
        ]
        .map(id_of);
        for (i, id) in ids.iter().enumerate() {
            assert!(fits_t3(id), "{id}");
            assert!(ids[i + 1..].iter().all(|other| other != id), "{id} is not unique");
        }
        assert_eq!(id_of("a.b"), id_of("a.b"), "stable");
        assert_eq!(id_of("a.b"), format!("ccam-lane-a_b_{}", &hex::encode(Sha256::digest(b"a.b"))[..8]));
        assert_eq!(id_of(&"q".repeat(64)).len(), ID_MAX);
    }

    #[test]
    fn sync_writes_once_then_leaves_the_file_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, serde_json::to_vec_pretty(&user_settings()).unwrap()).unwrap();
        let lanes = [lane("1", Some("work"))];

        assert_eq!(sync(&file, &lanes).unwrap(), Sync::Updated);
        let written = fs::read(&file).unwrap();
        assert_eq!(sync(&file, &lanes).unwrap(), Sync::Unchanged);
        assert_eq!(fs::read(&file).unwrap(), written);

        let on_disk: Value = serde_json::from_slice(&written).unwrap();
        assert_eq!(keys(&on_disk), ["projectSettingsFolded", "providerInstances", "idleCompaction"]);
        assert_eq!(on_disk["providerInstances"]["ccam-lane-1"]["displayName"], "Claude · lane 1 · work");
    }

    #[test]
    fn sync_with_no_lanes_does_not_create_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        assert_eq!(sync(&file, &[]).unwrap(), Sync::Unchanged);
        assert!(!file.exists());

        assert_eq!(sync(&file, &[lane("1", None)]).unwrap(), Sync::Updated);
        assert!(file.exists());
    }

    #[test]
    fn sync_refuses_a_file_it_cannot_safely_edit_and_leaves_it_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        for content in ["[1]", "{\"providerInstances\": []}", "{not json"] {
            fs::write(&file, content).unwrap();
            let refused = sync(&file, &[lane("1", Some("work"))]);
            assert!(matches!(refused, Err(Error::Malformed { .. } | Error::Json { .. })), "{content}");
            assert_eq!(fs::read_to_string(&file).unwrap(), content);
        }
    }

    #[test]
    fn settings_file_honours_t3code_home_and_needs_userdata() {
        let t3 = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let in_t3 = || settings_file_for(Some(t3.path().into()), Some(home.path().into()));
        let in_home = |env: Option<OsString>| settings_file_for(env, Some(home.path().into()));

        assert_eq!(in_t3(), None);
        assert_eq!(in_home(None), None);

        fs::create_dir(t3.path().join("userdata")).unwrap();
        fs::create_dir_all(home.path().join(".t3").join("userdata")).unwrap();
        assert_eq!(in_t3(), Some(t3.path().join("userdata").join("settings.json")));
        let default = Some(home.path().join(".t3").join("userdata").join("settings.json"));
        assert_eq!(in_home(None), default);
        assert_eq!(in_home(Some(OsString::new())), default);
        assert_eq!(settings_file_for(None, None), None);
    }
}
