//! Saved accounts on disk: `<vault>/<name>/login.json` and `<vault>/<name>/account.json`.
//!
//! Every write takes a `&VaultLock`: proof that the caller holds the vault lock. A write without one does not
//! compile, and each write re-checks that the lock was not taken over while the machine slept.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::jsonio::{self, Style};
use crate::lock::{self, LockGuard, LockTiming};
use crate::login::{Identity, OAuthLogin};
use crate::name::AccountName;

const LOGIN_FILE: &str = "login.json";
const ACCOUNT_FILE: &str = "account.json";
const LOG_FILE: &str = "switch.log";
pub const LOCK_DIR: &str = ".vault.lock";

#[derive(Debug, Clone)]
pub struct Account {
    pub name: AccountName,
    pub identity: Identity,
    /// The `oauthAccount` object verbatim, restored into `.claude.json` on switch.
    pub oauth_account: Value,
    pub login: OAuthLogin,
}

/// A directory in the vault that could not be loaded. Reported, never fatal: one broken entry must not lock
/// the user out of every other account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub entry: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub accounts: Vec<Account>,
    pub problems: Vec<Problem>,
}

impl Listing {
    pub fn get(&self, name: &AccountName) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.name == name)
    }

    pub fn find(&self, identity: &Identity) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.identity == identity)
    }
}

/// Exclusive access to the vault. Obtained only through `VaultLock::acquire`, so holding one proves the lock
/// is held; passing it to a write proves the write is allowed.
#[must_use = "the vault lock is released as soon as this guard is dropped"]
pub struct VaultLock(LockGuard);

impl VaultLock {
    pub fn acquire(vault: &Vault, timing: LockTiming) -> Result<Self> {
        lock::acquire(&vault.dir.join(LOCK_DIR), timing).map(Self)
    }

    /// Fails when the lock was taken over since it was acquired.
    pub fn check(&self) -> Result<()> {
        self.0.ensure_held()
    }
}

pub struct Vault {
    dir: PathBuf,
}

impl Vault {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// All loadable accounts sorted by name, plus the entries that failed to load.
    pub fn list(&self) -> Result<Listing> {
        let mut listing = Listing::default();
        for (name, path) in self.entries()? {
            match load(&path, name.clone()) {
                Ok(Some(account)) => listing.accounts.push(account),
                Ok(None) => {}
                Err(reason) => listing.problems.push(Problem { entry: name.to_string(), reason }),
            }
        }
        listing.accounts.sort_by_key(|a| a.name.as_str().to_ascii_lowercase());
        Ok(listing)
    }

    /// Creates or overwrites an account. The login is written last, so a crash never leaves a new login next to
    /// a stale identity.
    pub fn put(&self, lock: &VaultLock, account: &Account) -> Result<()> {
        lock.check()?;
        let dir = self.dir.join(account.name.as_str());
        jsonio::write_atomic(&dir.join(ACCOUNT_FILE), &account.oauth_account, Style::Pretty, true)?;
        let login = json!({ "claudeAiOauth": account.login.to_value() });
        jsonio::write_atomic(&dir.join(LOGIN_FILE), &login, Style::Pretty, true)
    }

    /// `from` must be the stored spelling.
    pub fn rename(&self, lock: &VaultLock, from: &AccountName, to: &AccountName) -> Result<()> {
        lock.check()?;
        let source = self.dir.join(from.as_str());
        let target = self.dir.join(to.as_str());
        // Two steps, so a case-only rename works on case-insensitive filesystems.
        let staging = self.dir.join(format!(".{}.renaming", to.as_str()));
        fs::rename(&source, &staging).map_err(|e| Error::io(&source, e))?;
        fs::rename(&staging, &target).map_err(|e| {
            // Put it back rather than leave the account hidden under the staging name.
            let _ = fs::rename(&staging, &source);
            Error::io(&target, e)
        })
    }

    /// `name` must be the stored spelling.
    pub fn remove(&self, lock: &VaultLock, name: &AccountName) -> Result<()> {
        lock.check()?;
        let dir = self.dir.join(name.as_str());
        fs::remove_dir_all(&dir).map_err(|e| Error::io(dir, e))
    }

    /// Appends one line to the switch log. Never pass secrets or email addresses.
    pub fn log(&self, message: &str) {
        let line = format!("{} {message}\n", jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ"));
        let path = self.dir.join(LOG_FILE);
        // Logging is diagnostic; a read-only vault must not turn a successful switch into a failure.
        let _ = fs::create_dir_all(&self.dir)
            .and_then(|()| OpenOptions::new().create(true).append(true).open(&path))
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }

    /// Copies accounts saved by the PowerShell prototype (`<name>/.credentials.json` and
    /// `<name>/oauthAccount.json` under `prototype_dir`) into this vault. The prototype's folder is only read.
    /// Accounts already in the vault are skipped.
    ///
    /// The prototype must not be used afterwards: both tools spending the same single-use refresh tokens would
    /// leave each other's copies dead.
    pub fn import_legacy(
        &self,
        lock: &VaultLock,
        prototype_dir: &Path,
    ) -> Result<Vec<(String, std::result::Result<AccountName, String>)>> {
        lock.check()?;
        let existing = self.list()?;
        let mut results = Vec::new();
        for (name, path) in dirs_named_as_accounts(prototype_dir)? {
            if existing.get(&name).is_some() || !path.join(".credentials.json").exists() {
                continue;
            }
            let outcome = convert_legacy(&path, &name).and_then(|account| {
                self.put(lock, &account).map_err(|e| e.to_string())?;
                Ok(account.name)
            });
            results.push((name.to_string(), outcome));
        }
        Ok(results)
    }

    fn entries(&self) -> Result<Vec<(AccountName, PathBuf)>> {
        dirs_named_as_accounts(&self.dir)
    }
}

fn dirs_named_as_accounts(dir: &Path) -> Result<Vec<(AccountName, PathBuf)>> {
    {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(dir, e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(dir, e))?;
            // Lock directories and temp files start with '.', which is not a valid account name.
            let Some(name) = entry.file_name().to_str().and_then(|n| n.parse::<AccountName>().ok()) else { continue };
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                out.push((name, entry.path()));
            }
        }
        Ok(out)
    }
}

fn load(dir: &Path, name: AccountName) -> std::result::Result<Option<Account>, String> {
    let read = |file: &str| jsonio::read(&dir.join(file)).map_err(|e| e.to_string());
    let (login_doc, oauth_account) = match (read(LOGIN_FILE)?, read(ACCOUNT_FILE)?) {
        (Some(login), Some(account)) => (login, account),
        (None, None) => return Ok(None),
        _ => return Err("incomplete: login.json or account.json is missing".into()),
    };
    build(name, oauth_account, login_doc.get("claudeAiOauth").cloned().unwrap_or(Value::Null)).map(Some)
}

fn convert_legacy(dir: &Path, name: &AccountName) -> std::result::Result<Account, String> {
    let read = |file: &str| {
        jsonio::read(&dir.join(file)).map_err(|e| e.to_string())?.ok_or_else(|| format!("{file} is missing"))
    };
    let credentials = read(".credentials.json")?;
    let oauth_account = read("oauthAccount.json")?;
    build(name.clone(), oauth_account, credentials.get("claudeAiOauth").cloned().unwrap_or(Value::Null))
}

fn build(name: AccountName, oauth_account: Value, login: Value) -> std::result::Result<Account, String> {
    let identity = Identity::from_oauth_account(&oauth_account).ok_or("account has no accountUuid")?;
    let login = OAuthLogin::parse(login).map_err(str::to_string)?;
    Ok(Account { name, identity, oauth_account, login })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockTiming;
    use crate::login::UnixMillis;

    fn account(name: &str, uuid: &str) -> Account {
        let oauth_account = json!({"accountUuid": uuid, "organizationUuid": "org"});
        Account {
            name: name.parse().unwrap(),
            identity: Identity::from_oauth_account(&oauth_account).unwrap(),
            oauth_account,
            login: OAuthLogin::parse(json!({"accessToken": "a", "refreshToken": "r", "expiresAt": 1})).unwrap(),
        }
    }

    fn locked(vault: &Vault) -> VaultLock {
        VaultLock::acquire(vault, LockTiming::VAULT).unwrap()
    }

    #[test]
    fn a_broken_entry_is_reported_and_the_rest_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::new(dir.path().to_path_buf());
        vault.put(&locked(&vault), &account("good", "u1")).unwrap();
        fs::create_dir_all(dir.path().join("broken")).unwrap();
        fs::write(dir.path().join("broken/login.json"), "{oops").unwrap();
        fs::write(dir.path().join("broken/account.json"), "{}").unwrap();
        let listing = vault.list().unwrap();

        assert_eq!(listing.accounts.iter().map(|a| a.name.to_string()).collect::<Vec<_>>(), ["good"]);
        assert_eq!(listing.problems.len(), 1);
        assert_eq!(listing.problems[0].entry, "broken");
    }

    #[test]
    fn import_copies_prototype_accounts_once_and_only_reads_the_prototype_folder() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::new(dir.path().join("ccam"));
        let prototype = dir.path().join("prototype");
        let legacy = prototype.join("work.main");
        fs::create_dir_all(&legacy).unwrap();
        let credentials =
            json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": 7}, "mcpOAuth": {"x": 1}});
        fs::write(legacy.join(".credentials.json"), format!("\u{feff}{credentials}")).unwrap();
        fs::write(legacy.join("oauthAccount.json"), json!({"accountUuid": "u"}).to_string()).unwrap();
        fs::create_dir_all(prototype.join("half")).unwrap();
        fs::write(prototype.join("half/.credentials.json"), credentials.to_string()).unwrap();

        let results = vault.import_legacy(&locked(&vault), &prototype).unwrap();

        assert!(results.iter().any(|(n, r)| n == "work.main" && r.is_ok()));
        assert!(results.iter().any(|(n, r)| n == "half" && r.is_err()));
        let listing = vault.list().unwrap();
        assert_eq!(listing.accounts[0].login.expires_at(), UnixMillis::from_millis(7));
        let imported = fs::read_to_string(dir.path().join("ccam/work.main/login.json")).unwrap();
        assert!(!imported.contains("mcpOAuth"));
        assert_eq!(fs::read_dir(&legacy).unwrap().count(), 2, "nothing was added to the prototype's folder");
        let again = vault.import_legacy(&locked(&vault), &prototype).unwrap();
        assert!(again.iter().all(|(n, _)| n != "work.main"), "a second import skips it");
    }

    #[test]
    fn rename_that_cannot_complete_restores_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::new(dir.path().to_path_buf());
        let lock = locked(&vault);
        vault.put(&lock, &account("a", "u1")).unwrap();
        fs::create_dir_all(dir.path().join("b/occupied")).unwrap();

        assert!(vault.rename(&lock, &"a".parse().unwrap(), &"b".parse().unwrap()).is_err());
        assert!(dir.path().join("a/login.json").exists());
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::lock::LockTiming;

    #[test]
    fn writes_refuse_a_lock_that_was_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::new(dir.path().to_path_buf());
        let lock = VaultLock::acquire(&vault, LockTiming::VAULT).unwrap();
        let oauth_account = json!({"accountUuid": "u"});
        let account = Account {
            name: "a".parse().unwrap(),
            identity: Identity::from_oauth_account(&oauth_account).unwrap(),
            oauth_account,
            login: OAuthLogin::parse(json!({"accessToken": "a", "refreshToken": "r", "expiresAt": 1})).unwrap(),
        };
        vault.put(&lock, &account).unwrap();

        // Another process decided the lock was stale and recreated it.
        let lock_dir = dir.path().join(LOCK_DIR);
        fs::remove_dir(&lock_dir).unwrap();
        fs::create_dir(&lock_dir).unwrap();

        assert!(matches!(vault.put(&lock, &account), Err(Error::LockLost(_))));
        assert!(matches!(vault.remove(&lock, &account.name), Err(Error::LockLost(_))));
        assert!(dir.path().join("a/login.json").exists());
    }
}
