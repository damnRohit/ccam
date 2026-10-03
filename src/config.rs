//! The `oauthAccount` entry of Claude Code's global config (`.claude.json`).

use std::path::PathBuf;

use serde_json::Value;

use crate::error::Result;
use crate::jsonio::{self, Style};

const ACCOUNT_KEY: &str = "oauthAccount";

pub struct GlobalConfig {
    path: PathBuf,
}

impl GlobalConfig {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn read_oauth_account(&self) -> Result<Option<Value>> {
        Ok(jsonio::read(&self.path)?
            .and_then(|mut doc| doc.get_mut(ACCOUNT_KEY).map(Value::take))
            .filter(Value::is_object))
    }

    /// Replaces `oauthAccount` and leaves the rest of the file, which Claude Code owns, as it was, key order
    /// included. The file can hold keys that differ only by case, which a case-folding parser would merge.
    pub fn write_oauth_account(&self, account: &Value) -> Result<()> {
        jsonio::edit_object(&self.path, Style::Pretty, false, |map| {
            map.insert(ACCOUNT_KEY.into(), account.clone());
            Ok(())
        })
        .map(drop)
    }

    pub fn remove_oauth_account(&self) -> Result<()> {
        jsonio::edit_object(&self.path, Style::Pretty, false, |map| {
            map.shift_remove(ACCOUNT_KEY);
            Ok(())
        })
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, json};

    use super::*;

    #[test]
    fn replacing_the_account_keeps_other_keys_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"projects":{"C:/x":{"a":1},"c:/x":{"a":2}},"oauthAccount":{"accountUuid":"old"},"zeta":true}"#,
        )
        .unwrap();

        let config = GlobalConfig::new(path.clone());
        config.write_oauth_account(&json!({"accountUuid": "new"})).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let keys: Vec<_> = serde_json::from_str::<Map<String, Value>>(&text).unwrap().keys().cloned().collect();
        assert_eq!(keys, ["projects", "oauthAccount", "zeta"]);
        assert!(text.contains("\"C:/x\"") && text.contains("\"c:/x\""));
        assert_eq!(config.read_oauth_account().unwrap(), Some(json!({"accountUuid": "new"})));
    }
}
