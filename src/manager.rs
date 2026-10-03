//! Account operations. Every operation that touches a login upholds the invariants in `docs/SPEC.md` §5.
//!
//! Locks are typed so the order cannot be broken: a `CredentialLocks` can only be built from a `VaultLock`, and
//! a `SwitchLocks` only from a `CredentialLocks`. Writes take the lock they need as an argument, so a write
//! without the right lock does not compile. Network calls never happen while Claude Code's locks are held,
//! because Claude Code gives up on a held lock after a few retries and would then fail its own token refresh.

use serde_json::Value;

use crate::api::{ApiError, OAuthClient};
use crate::config::GlobalConfig;
use crate::error::{Error, Result};
use crate::lock::{self, LockGuard, LockTiming};
use crate::login::{Identity, OAuthLogin, UnixMillis};
use crate::name::AccountName;
use crate::paths::Paths;
use crate::presence::{self, Mark, Side};
use crate::store::LoginStore;
use crate::vault::{Account, Listing, Vault, VaultLock};

/// Whether an operation may call Anthropic to confirm who owns a new login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Allowed,
    /// For hooks that block a session and for the implicit sync before other commands.
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    NotLoggedIn,
    LiveUnreadable,
    NotSaved,
    Unchanged(AccountName),
    Updated(AccountName),
    /// The saved copy is a newer generation than the live login. Left alone: overwriting it would restore a
    /// refresh token that no longer works.
    SavedIsNewer(AccountName),
    /// A new login appeared and was not confirmed to belong to this account, so it was not saved.
    Unverified {
        name: AccountName,
        reason: String,
    },
}

/// What Claude Code is logged in as right now.
#[derive(Debug, Clone)]
pub enum LiveState {
    LoggedOut,
    /// Credentials exist but cannot be parsed or have no identity. Never overwritten or saved.
    Unreadable,
    Present(Box<Live>),
}

#[derive(Debug, Clone)]
pub struct Live {
    pub login: OAuthLogin,
    pub oauth_account: Value,
    pub identity: Identity,
}

#[derive(Debug, Clone)]
pub struct Overview {
    pub listing: Listing,
    pub live: LiveState,
}

impl Overview {
    /// The saved account Claude Code is logged in as, if it is saved.
    pub fn live_account(&self) -> Option<&Account> {
        match &self.live {
            LiveState::Present(live) => self.listing.find(&live.identity),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub from: Option<AccountName>,
    pub to: AccountName,
}

pub struct Saved {
    pub name: AccountName,
    /// Set when ownership of the login could not be checked (offline), so the user can double-check.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct LockTimings {
    pub vault: LockTiming,
    pub credentials: LockTiming,
    pub config: LockTiming,
}

impl Default for LockTimings {
    fn default() -> Self {
        Self { vault: LockTiming::VAULT, credentials: LockTiming::CREDENTIALS, config: LockTiming::CONFIG }
    }
}

/// Claude Code's two refresh locks, taken in Claude Code's order. While held, no token refresh is in flight.
///
/// Fields drop in declaration order, which is the reverse of acquisition.
#[must_use = "the locks are released as soon as this guard is dropped"]
pub struct RefreshLocks {
    legacy: LockGuard,
    primary: LockGuard,
}

impl RefreshLocks {
    fn acquire(paths: &Paths, timing: LockTiming) -> Result<Self> {
        let primary = lock::acquire(&paths.refresh_lock, timing)?;
        let legacy = lock::acquire(&paths.legacy_refresh_lock, timing)?;
        Ok(Self { legacy, primary })
    }

    fn check(&self) -> Result<()> {
        self.primary.ensure_held()?;
        self.legacy.ensure_held()
    }
}

/// The vault lock plus Claude Code's refresh locks: enough to read the live login consistently and write the
/// vault. Only constructible from a `VaultLock`, so the lock order holds by construction.
#[must_use = "the locks are released as soon as this guard is dropped"]
pub struct CredentialLocks {
    refresh: RefreshLocks,
    vault: VaultLock,
}

impl CredentialLocks {
    fn acquire(vault: VaultLock, paths: &Paths, timing: LockTiming) -> Result<Self> {
        Ok(Self { refresh: RefreshLocks::acquire(paths, timing)?, vault })
    }

    pub fn vault(&self) -> &VaultLock {
        &self.vault
    }

    fn check(&self) -> Result<()> {
        self.vault.check()?;
        self.refresh.check()
    }
}

/// Everything a switch needs: `CredentialLocks` plus the global config lock.
#[must_use = "the locks are released as soon as this guard is dropped"]
pub struct SwitchLocks {
    config: LockGuard,
    credentials: CredentialLocks,
}

impl SwitchLocks {
    fn acquire(credentials: CredentialLocks, paths: &Paths, timing: LockTiming) -> Result<Self> {
        Ok(Self { config: lock::acquire(&paths.global_config_lock, timing)?, credentials })
    }

    fn check(&self) -> Result<()> {
        self.credentials.check()?;
        self.config.ensure_held()
    }
}

pub struct Manager {
    paths: Paths,
    store: Box<dyn LoginStore>,
    client: Box<dyn OAuthClient>,
    config: GlobalConfig,
    vault: Vault,
    timings: LockTimings,
    side: Side,
}

enum Ownership {
    Confirmed,
    /// The token belongs to someone else, or was rejected outright.
    Refuted(String),
    Unknown(String),
}

/// What a locked look at the live login and its saved copy found.
enum Assessment {
    Settled(SyncOutcome),
    /// The live login is a renewal within the saved login's session: safe to save as is.
    Renewal {
        live: Live,
        saved: Account,
    },
    /// The live login is from a new `/login`: its owner must be confirmed before it is saved.
    NewLogin {
        live: Live,
        saved: Account,
    },
}

impl Manager {
    pub fn new(paths: Paths, store: Box<dyn LoginStore>, client: Box<dyn OAuthClient>) -> Self {
        Self {
            config: GlobalConfig::new(paths.global_config.clone()),
            vault: Vault::new(paths.vault_dir.clone()),
            side: Side::current(&paths),
            store,
            client,
            paths,
            timings: LockTimings::default(),
        }
    }

    pub fn with_lock_timings(mut self, timings: LockTimings) -> Self {
        self.timings = timings;
        self
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn client(&self) -> &dyn OAuthClient {
        self.client.as_ref()
    }

    /// This install of Claude Code, as other installs sharing the vault see it.
    pub fn side(&self) -> &Side {
        &self.side
    }

    /// What every other install sharing this vault is logged in as.
    pub fn marks(&self) -> Vec<Mark> {
        presence::others(&self.paths.vault_dir, &self.side)
    }

    /// The other install logged in as this account, if any. Its refresh token must not be spent here too.
    pub fn used_elsewhere(&self, identity: &Identity) -> Option<Mark> {
        self.marks().into_iter().find(|m| &m.identity == identity)
    }

    /// Under the vault lock, so another install's switch sees either no record or this one. Only a definite state
    /// changes the record: a login caught mid-write must not clear it, or another install could renew the token
    /// this one is using. Best effort: a failure only means another install may not see this one's account.
    fn record_presence(&self, lock: &VaultLock) {
        let live = match self.read_live() {
            Ok(LiveState::Present(live)) => Some(live.identity),
            Ok(LiveState::LoggedOut) => None,
            Ok(LiveState::Unreadable) | Err(_) => return,
        };
        let recorded = lock.check().and_then(|()| presence::record(&self.paths.vault_dir, &self.side, live.as_ref()));
        if let Err(e) = recorded {
            self.vault.log(&format!("presence not recorded: {e}"));
        }
    }

    fn ensure_free(&self, name: &AccountName) -> Result<()> {
        let listing = self.vault.list()?;
        let Some(target) = listing.get(name) else { return Ok(()) };
        match self.used_elsewhere(&target.identity) {
            Some(mark) => Err(Error::InUseElsewhere { name: target.name.clone(), side: mark.side.label() }),
            None => Ok(()),
        }
    }

    /// Appends to the switch log. Never pass secrets or email addresses.
    pub fn log(&self, message: &str) {
        self.vault.log(message);
    }

    /// A consistent-enough read of saved accounts and the live login, without locks, for display.
    pub fn overview(&self) -> Result<Overview> {
        Ok(Overview { listing: self.vault.list()?, live: self.read_live()? })
    }

    pub fn sync(&self, network: Network) -> Result<SyncOutcome> {
        let outcome = self.sync_now(network);
        if let Ok(lock) = self.lock_vault() {
            self.record_presence(&lock);
        }
        outcome
    }

    fn sync_now(&self, network: Network) -> Result<SyncOutcome> {
        let locks = self.lock_credentials()?;
        let (live, saved) = match self.assess(&locks)? {
            Assessment::Settled(outcome) => return Ok(outcome),
            Assessment::Renewal { live, saved } => return self.save_renewal(&locks, live, saved),
            Assessment::NewLogin { live, saved } => (live, saved),
        };

        // Confirming needs the network, which must not happen under Claude Code's locks.
        drop(locks);
        if network == Network::Offline {
            return Ok(SyncOutcome::Unverified { name: saved.name, reason: "not checked offline".into() });
        }
        match self.ownership(&live) {
            Ownership::Confirmed => {}
            Ownership::Refuted(reason) | Ownership::Unknown(reason) => {
                self.vault.log(&format!("sync '{}' skipped: {reason}", saved.name));
                return Ok(SyncOutcome::Unverified { name: saved.name, reason });
            }
        }

        // The confirmation only applies to the exact login that was checked.
        let locks = self.lock_credentials()?;
        match self.assess(&locks)? {
            Assessment::Settled(outcome) => Ok(outcome),
            Assessment::Renewal { live, saved } => self.save_renewal(&locks, live, saved),
            Assessment::NewLogin { live: current, saved } if current.login == live.login => {
                self.save_renewal(&locks, current, saved)
            }
            Assessment::NewLogin { saved, .. } => Ok(SyncOutcome::Unverified {
                name: saved.name,
                reason: "the login changed while it was being checked".into(),
            }),
        }
    }

    pub fn save(&self, name: &AccountName) -> Result<Saved> {
        let LiveState::Present(checked) = self.read_live()? else {
            return Err(self.not_saveable()?);
        };
        // Confirm ownership before taking Claude Code's locks; the network call can take seconds.
        let warning = match self.ownership(&checked) {
            Ownership::Confirmed => None,
            Ownership::Refuted(reason) => return Err(Error::LiveUnverified { name: name.clone(), reason }),
            Ownership::Unknown(reason) => {
                Some(format!("could not confirm which account this login belongs to ({reason})"))
            }
        };

        let locks = self.lock_credentials()?;
        let listing = self.vault.list()?;
        let LiveState::Present(live) = self.read_live()? else { return Err(self.not_saveable()?) };
        if live.login != checked.login || live.identity != checked.identity {
            return Err(Error::LiveUnverified {
                name: name.clone(),
                reason: "the login changed while it was being checked; retry".into(),
            });
        }
        if let Some(existing) = listing.find(&live.identity) {
            if &existing.name != name {
                return Err(Error::AlreadySavedAs(existing.name.clone()));
            }
            if existing.login.is_newer_than(&live.login) {
                return Err(Error::GenerationConflict(existing.name.clone()));
            }
        }
        if let Some(occupant) = listing.get(name)
            && occupant.identity != live.identity
        {
            return Err(Error::NameHoldsOtherAccount(occupant.name.clone()));
        }

        // Re-saving under a different capitalisation keeps the stored spelling.
        let name = listing.get(name).map_or_else(|| name.clone(), |a| a.name.clone());
        let account = Account {
            name: name.clone(),
            identity: live.identity,
            oauth_account: live.oauth_account,
            login: live.login,
        };
        self.vault.put(locks.vault(), &account)?;
        self.vault.log(&format!("save '{name}'"));
        self.record_presence(locks.vault());
        Ok(Saved { name, warning })
    }

    pub fn rename(&self, from: &AccountName, to: &AccountName) -> Result<()> {
        let lock = self.lock_vault()?;
        let listing = self.vault.list()?;
        let existing = listing.get(from).ok_or_else(|| Error::UnknownAccount(from.to_string()))?;
        if let Some(other) = listing.get(to)
            && other.name.as_str() != existing.name.as_str()
            && other.identity != existing.identity
        {
            return Err(Error::NameTaken(other.name.clone()));
        }
        self.vault.rename(&lock, &existing.name, to)?;
        self.vault.log(&format!("rename '{}' -> '{to}'", existing.name));
        Ok(())
    }

    pub fn remove(&self, name: &AccountName) -> Result<()> {
        let lock = self.lock_vault()?;
        let listing = self.vault.list()?;
        let existing = listing.get(name).ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
        self.vault.remove(&lock, &existing.name)?;
        self.vault.log(&format!("remove '{}'", existing.name));
        Ok(())
    }

    /// Checks whether this side could switch to `name` once no other install uses it. `--force` asks this before
    /// moving another install, so a switch that would fail here anyway changes nothing there.
    pub fn check_switch_here(&self, name: &AccountName) -> Result<()> {
        plan_switch(&self.vault.list()?, &self.read_live()?, name).map(drop)
    }

    /// Checks whether `switch_to(name)` would be allowed right now, without changing anything.
    pub fn check_switch(&self, name: &AccountName) -> Result<()> {
        self.ensure_free(name)?;
        plan_switch(&self.vault.list()?, &self.read_live()?, name).map(drop)
    }

    pub fn switch_to(&self, name: &AccountName) -> Result<Switched> {
        self.switch(name, self.timings.credentials)
    }

    /// For the new tab after `--restart`: the terminated Claude Code may have died holding its refresh lock, so
    /// this waits long enough to take a stale lock over.
    pub fn switch_after_kill(&self, name: &AccountName) -> Result<Switched> {
        self.switch(name, LockTiming::CREDENTIALS_AFTER_KILL)
    }

    fn switch(&self, name: &AccountName, credentials: LockTiming) -> Result<Switched> {
        self.ensure_free(name)?;
        // Save a renewed or confirmed login of the account being left first. This may need the network, so it
        // runs before the switch takes Claude Code's locks.
        if let SyncOutcome::Unverified { name, reason } = self.sync(Network::Allowed)? {
            return Err(Error::LiveUnverified { name, reason });
        }

        let locks = self.lock_switch(credentials)?;
        let listing = self.vault.list()?;
        let live = self.read_live()?;
        let plan = plan_switch(&listing, &live, name)?;
        // Checked again under the vault lock: the early check can race another install's switch, and both
        // installing the same saved login would let the first renewal log the other out.
        if let Some(mark) = self.used_elsewhere(&plan.target.identity) {
            return Err(Error::InUseElsewhere { name: plan.target.name, side: mark.side.label() });
        }

        if let Some(leaving) = &plan.save_leaving {
            self.vault.put(locks.credentials.vault(), leaving)?;
            self.vault.log(&format!("sync '{}'", leaving.name));
        }
        self.install(&locks, &plan.target)?;
        self.record_presence(locks.credentials.vault());

        let from_label = plan.from.as_ref().map_or_else(|| "(logged out)".to_string(), ToString::to_string);
        self.vault.log(&format!("switch '{from_label}' -> '{}'", plan.target.name));
        Ok(Switched { from: plan.from, to: plan.target.name })
    }

    /// Makes `target` the live login and identity. The two writes cannot be atomic together, and a half-done
    /// switch would pair one account's login with the other's identity, so the identity is undone when the
    /// login write fails.
    fn install(&self, locks: &SwitchLocks, target: &Account) -> Result<()> {
        locks.check()?;
        let previous = self.config.read_oauth_account()?;
        self.config.write_oauth_account(&target.oauth_account)?;
        let Err(e) = self.store.write(&target.login) else { return Ok(()) };
        let restored = match &previous {
            Some(account) => self.config.write_oauth_account(account),
            None => self.config.remove_oauth_account(),
        };
        if let Err(restore) = restored {
            self.vault.log(&format!("switch to '{}' failed; identity restore failed: {restore}", target.name));
        }
        Err(e)
    }

    /// Renews the login of a saved account that is not in use and saves the new generation before returning it.
    pub fn refresh_saved(&self, name: &AccountName, now: UnixMillis) -> Result<Account> {
        // The vault lock, not Claude Code's: this holds it across a network call, and a switch to this account
        // also takes it, so the switch can never install the refresh token this is about to spend.
        let lock = self.lock_vault()?;
        let listing = self.vault.list()?;
        let account = listing.get(name).cloned().ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
        if let LiveState::Present(live) = self.read_live()?
            && (live.identity == account.identity || live.login.refresh_token() == account.login.refresh_token())
        {
            return Err(Error::LoginInUse(account.name));
        }
        if self.used_elsewhere(&account.identity).is_some() {
            return Err(Error::LoginInUse(account.name));
        }
        if account.login.usable_at(now) {
            return Ok(account);
        }

        let login = self.client.refresh(&account.login)?;
        let refreshed = Account { login, ..account };
        // The old refresh token is already spent, so saving the new one matters more than the lock. If the lock
        // was taken over during the network call, take a fresh one rather than discard the only working login.
        match lock.check() {
            Ok(()) => self.vault.put(&lock, &refreshed)?,
            Err(e) => {
                self.vault.log(&format!("refresh '{}': {e}; re-locking to save", refreshed.name));
                drop(lock);
                self.vault.put(&self.lock_vault()?, &refreshed)?;
            }
        }
        self.vault.log(&format!("refresh '{}'", refreshed.name));
        Ok(refreshed)
    }

    pub fn import_legacy(&self) -> Result<Vec<(String, std::result::Result<AccountName, String>)>> {
        let lock = self.lock_vault()?;
        let results = self.vault.import_legacy(&lock, &self.paths.prototype_vault_dir)?;
        for (entry, result) in &results {
            if result.is_ok() {
                self.vault.log(&format!("import '{entry}'"));
            }
        }
        Ok(results)
    }

    /// Holds Claude Code's refresh locks until the returned guard is dropped, so no token refresh is in flight.
    /// The vault lock is not taken: nothing here writes the vault, and holding it would block the new tab's
    /// switch for longer than necessary.
    pub fn hold_refresh_locks(&self) -> Result<RefreshLocks> {
        RefreshLocks::acquire(&self.paths, self.timings.credentials)
    }

    fn lock_vault(&self) -> Result<VaultLock> {
        VaultLock::acquire(&self.vault, self.timings.vault)
    }

    fn lock_credentials(&self) -> Result<CredentialLocks> {
        CredentialLocks::acquire(self.lock_vault()?, &self.paths, self.timings.credentials)
    }

    fn lock_switch(&self, credentials: LockTiming) -> Result<SwitchLocks> {
        let credentials = CredentialLocks::acquire(self.lock_vault()?, &self.paths, credentials)?;
        SwitchLocks::acquire(credentials, &self.paths, self.timings.config)
    }

    /// Compares the live login with its saved copy. Needs the credential locks so the comparison is against a
    /// login Claude Code is not in the middle of replacing.
    fn assess(&self, locks: &CredentialLocks) -> Result<Assessment> {
        locks.check()?;
        let listing = self.vault.list()?;
        let live = match self.read_live()? {
            LiveState::LoggedOut => return Ok(Assessment::Settled(SyncOutcome::NotLoggedIn)),
            LiveState::Unreadable => return Ok(Assessment::Settled(SyncOutcome::LiveUnreadable)),
            LiveState::Present(live) => *live,
        };
        let Some(saved) = listing.find(&live.identity).cloned() else {
            return Ok(Assessment::Settled(SyncOutcome::NotSaved));
        };
        Ok(if saved.login == live.login {
            Assessment::Settled(SyncOutcome::Unchanged(saved.name))
        } else if !live.login.is_newer_than(&saved.login) {
            Assessment::Settled(SyncOutcome::SavedIsNewer(saved.name))
        } else if live.login.same_session_as(&saved.login) {
            Assessment::Renewal { live, saved }
        } else {
            Assessment::NewLogin { live, saved }
        })
    }

    fn save_renewal(&self, locks: &CredentialLocks, live: Live, saved: Account) -> Result<SyncOutcome> {
        let name = saved.name.clone();
        let account = Account { login: live.login, oauth_account: live.oauth_account, ..saved };
        self.vault.put(locks.vault(), &account)?;
        self.vault.log(&format!("sync '{name}'"));
        Ok(SyncOutcome::Updated(name))
    }

    fn read_live(&self) -> Result<LiveState> {
        let Some(raw) = self.store.read()? else { return Ok(LiveState::LoggedOut) };
        if OAuthLogin::is_cleared(&raw) {
            return Ok(LiveState::LoggedOut);
        }
        let (Ok(login), Some(oauth_account)) = (OAuthLogin::parse(raw), self.config.read_oauth_account()?) else {
            return Ok(LiveState::Unreadable);
        };
        let Some(identity) = Identity::from_oauth_account(&oauth_account) else { return Ok(LiveState::Unreadable) };
        Ok(LiveState::Present(Box::new(Live { login, oauth_account, identity })))
    }

    fn not_saveable(&self) -> Result<Error> {
        Ok(match self.read_live()? {
            LiveState::Unreadable => Error::LiveUnreadable,
            _ => Error::NotLoggedIn,
        })
    }

    fn ownership(&self, live: &Live) -> Ownership {
        match self.client.account_uuid(live.login.access_token()) {
            Ok(uuid) if uuid == live.identity.account_uuid => Ownership::Confirmed,
            Ok(_) => Ownership::Refuted("the token belongs to a different account".into()),
            Err(ApiError::Unauthorized) => Ownership::Refuted("Anthropic rejected the token".into()),
            Err(e) => Ownership::Unknown(e.to_string()),
        }
    }
}

struct SwitchPlan {
    target: Account,
    from: Option<AccountName>,
    /// A renewed login of the account being left, to save before it is replaced.
    save_leaving: Option<Account>,
}

/// The rules for whether a switch may replace the live login. Shared by the switch itself and by the restart
/// flow's pre-check, so both refuse the same things.
fn plan_switch(listing: &Listing, live: &LiveState, name: &AccountName) -> Result<SwitchPlan> {
    let target = listing.get(name).cloned().ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
    let live = match live {
        LiveState::LoggedOut => return Ok(SwitchPlan { target, from: None, save_leaving: None }),
        LiveState::Unreadable => return Err(Error::LiveUnreadable),
        LiveState::Present(live) => live,
    };
    if live.identity == target.identity {
        return Err(Error::AlreadyCurrent(target.name));
    }
    let saved = listing.find(&live.identity).ok_or(Error::LiveNotSaved)?;
    let from = Some(saved.name.clone());

    if saved.login == live.login {
        return Ok(SwitchPlan { target, from, save_leaving: None });
    }
    if !live.login.is_newer_than(&saved.login) {
        return Err(Error::GenerationConflict(saved.name.clone()));
    }
    if !live.login.same_session_as(&saved.login) {
        return Err(Error::LiveUnverified {
            name: saved.name.clone(),
            reason: "the login changed since it was confirmed; retry".into(),
        });
    }
    let leaving = Account { login: live.login.clone(), oauth_account: live.oauth_account.clone(), ..saved.clone() };
    Ok(SwitchPlan { target, from, save_leaving: Some(leaving) })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;

    use std::path::Path;

    use super::*;
    use crate::jsonio;
    use crate::presence::Platform;
    use crate::store::FileStore;

    const FAST: LockTiming = LockTiming {
        stale_after: Duration::from_secs(60),
        wait: Duration::from_millis(200),
        heartbeat: Duration::from_millis(50),
        poll: Duration::from_millis(20),
    };
    const FAST_TIMINGS: LockTimings = LockTimings { vault: FAST, credentials: FAST, config: FAST };

    /// Resolves tokens minted by `write_live`, whose access tokens name their owner: `at-<owner>-<expiry>`.
    /// Owner `OFFLINE` simulates no network. Records whether Claude Code's lock was held during each call.
    #[derive(Default)]
    struct FakeClient {
        calls: AtomicUsize,
        refresh_lock: Mutex<Option<std::path::PathBuf>>,
        called_under_lock: AtomicUsize,
    }

    /// Shared with the fixture so tests can inspect calls after handing the client to the manager.
    impl OAuthClient for Arc<FakeClient> {
        fn account_uuid(&self, access_token: &str) -> std::result::Result<String, ApiError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.refresh_lock.lock().unwrap().as_ref().is_some_and(|p| p.exists()) {
                self.called_under_lock.fetch_add(1, Ordering::Relaxed);
            }
            match access_token.split('-').nth(1) {
                Some("OFFLINE") => Err(ApiError::Transport("offline".into())),
                Some("REJECTED") => Err(ApiError::Unauthorized),
                Some(owner) => Ok(owner.to_string()),
                None => Err(ApiError::Malformed),
            }
        }

        fn refresh(&self, login: &OAuthLogin) -> std::result::Result<OAuthLogin, ApiError> {
            Ok(login.refreshed("at-fresh", Some("rt-fresh"), login.expires_at().plus(Duration::from_secs(1))))
        }

        fn usage(&self, _: &str) -> std::result::Result<Value, ApiError> {
            unreachable!()
        }
    }

    struct BrokenStore;

    impl LoginStore for BrokenStore {
        fn read(&self) -> Result<Option<Value>> {
            Ok(None)
        }
        fn write(&self, _: &OAuthLogin) -> Result<()> {
            Err(Error::Keychain("disk full".into()))
        }
    }

    struct Fixture {
        _home: tempfile::TempDir,
        paths: Paths,
        manager: Manager,
        client: Arc<FakeClient>,
    }

    impl Fixture {
        fn client(&self) -> &FakeClient {
            &self.client
        }
    }

    fn fixture() -> Fixture {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(home.path(), None);
        fs::create_dir_all(&paths.config_home).unwrap();
        let client = Arc::new(FakeClient::default());
        *client.refresh_lock.lock().unwrap() = Some(paths.refresh_lock.clone());
        let store = Box::new(FileStore::new(paths.credentials_file.clone()));
        let manager = Manager::new(paths.clone(), store, Box::new(Arc::clone(&client))).with_lock_timings(FAST_TIMINGS);
        Fixture { _home: home, paths, manager, client }
    }

    fn name(s: &str) -> AccountName {
        s.parse().unwrap()
    }

    /// Claude Code logged in as `uuid`, with a token renewed to expire at `expires_at`. Renewals of one login
    /// share `refreshTokenExpiresAt`.
    fn log_in(paths: &Paths, uuid: &str, expires_at: i64) {
        write_live(paths, uuid, uuid, expires_at, Some(9_000));
    }

    /// A live login owned by `token_owner` while `.claude.json` names `identity`, to reproduce the moment during
    /// `/login` when only one of the two files has been updated.
    fn write_live(paths: &Paths, identity: &str, token_owner: &str, expires_at: i64, session: Option<i64>) {
        let mut login = json!({
            "accessToken": format!("at-{token_owner}-{expires_at}"),
            "refreshToken": format!("rt-{token_owner}-{expires_at}"),
            "expiresAt": expires_at,
            "subscriptionType": "pro",
        });
        if let Some(session) = session {
            login["refreshTokenExpiresAt"] = session.into();
        }
        write_credentials(paths, login);
        let mut config =
            jsonio::read(&paths.global_config).unwrap().unwrap_or(json!({"projects": {"C:/x": 1, "c:/x": 2}}));
        config["oauthAccount"] = json!({"accountUuid": identity, "organizationUuid": "org", "emailAddress": format!("{identity}@example.com")});
        fs::write(&paths.global_config, config.to_string()).unwrap();
    }

    fn write_credentials(paths: &Paths, login: Value) {
        let creds = json!({"claudeAiOauth": login, "mcpOAuth": {"server": "machine-state"}});
        fs::write(&paths.credentials_file, creds.to_string()).unwrap();
    }

    fn live_credentials(paths: &Paths) -> Value {
        jsonio::read(&paths.credentials_file).unwrap().unwrap()
    }

    fn live_config(paths: &Paths) -> Value {
        jsonio::read(&paths.global_config).unwrap().unwrap()
    }

    fn saved_expiry(paths: &Paths, account: &str) -> i64 {
        let doc = jsonio::read(&paths.vault_dir.join(account).join("login.json")).unwrap().unwrap();
        doc["claudeAiOauth"]["expiresAt"].as_i64().unwrap()
    }

    fn entries(f: &Fixture) -> Vec<(String, bool)> {
        let overview = f.manager.overview().unwrap();
        let live = overview.live_account().map(|a| a.name.clone());
        overview.listing.accounts.iter().map(|a| (a.name.to_string(), Some(&a.name) == live.as_ref())).collect()
    }

    fn save(f: &Fixture, account: &str) {
        f.manager.save(&name(account)).unwrap();
    }

    #[test]
    fn save_then_overview_marks_the_logged_in_account() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "work");
        log_in(&f.paths, "B", 100);
        save(&f, "home.2");

        assert_eq!(entries(&f), [("home.2".into(), true), ("work".into(), false)]);
    }

    #[test]
    fn saved_login_excludes_machine_wide_mcp_state() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");

        let text = fs::read_to_string(f.paths.vault_dir.join("a/login.json")).unwrap();
        assert!(!text.contains("mcpOAuth"));
    }

    #[test]
    fn save_refuses_a_login_whose_token_belongs_to_another_account() {
        let f = fixture();
        write_live(&f.paths, "A", "B", 100, Some(9_000));

        assert!(matches!(f.manager.save(&name("a")), Err(Error::LiveUnverified { .. })));
        assert!(!f.paths.vault_dir.join("a").exists());
    }

    #[test]
    fn save_works_offline_but_says_ownership_was_not_checked() {
        let f = fixture();
        write_live(&f.paths, "A", "OFFLINE", 100, Some(9_000));

        let saved = f.manager.save(&name("a")).unwrap();
        assert!(saved.warning.is_some());
    }

    #[test]
    fn save_refuses_duplicates_and_occupied_names() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        assert!(matches!(f.manager.save(&name("other")), Err(Error::AlreadySavedAs(n)) if n == name("a")));

        log_in(&f.paths, "B", 100);
        assert!(matches!(f.manager.save(&name("A")), Err(Error::NameHoldsOtherAccount(_))));
    }

    #[test]
    fn save_reports_logged_out_and_unreadable_logins() {
        let f = fixture();
        assert!(matches!(f.manager.save(&name("a")), Err(Error::NotLoggedIn)));
        log_in(&f.paths, "A", 100);
        write_credentials(&f.paths, json!({"accessToken": "at-A-1", "refreshToken": "r"}));
        assert!(matches!(f.manager.save(&name("a")), Err(Error::LiveUnreadable)));
    }

    #[test]
    fn sync_copies_a_renewed_login_without_the_network() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        let calls_after_save = f.client().calls.load(Ordering::Relaxed);

        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::Unchanged(name("a")));
        log_in(&f.paths, "A", 200);
        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::Updated(name("a")));
        assert_eq!(saved_expiry(&f.paths, "a"), 200);
        assert_eq!(f.client().calls.load(Ordering::Relaxed), calls_after_save);
    }

    #[test]
    fn sync_never_replaces_a_newer_saved_login_with_an_older_live_one() {
        let f = fixture();
        log_in(&f.paths, "A", 300);
        save(&f, "a");
        log_in(&f.paths, "A", 100);

        assert_eq!(f.manager.sync(Network::Allowed).unwrap(), SyncOutcome::SavedIsNewer(name("a")));
        assert_eq!(saved_expiry(&f.paths, "a"), 300);
    }

    #[test]
    fn sync_skips_a_new_login_whose_token_belongs_to_another_account() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        write_live(&f.paths, "A", "B", 200, Some(7_000));

        assert!(matches!(f.manager.sync(Network::Allowed).unwrap(), SyncOutcome::Unverified { .. }));
        assert_eq!(saved_expiry(&f.paths, "a"), 100);
    }

    #[test]
    fn sync_confirms_a_new_login_without_holding_claude_codes_lock() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        write_live(&f.paths, "A", "A", 200, Some(7_000));

        assert_eq!(f.manager.sync(Network::Allowed).unwrap(), SyncOutcome::Updated(name("a")));
        assert_eq!(saved_expiry(&f.paths, "a"), 200);
        assert_eq!(f.client().called_under_lock.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn offline_sync_never_calls_the_network() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        let calls = f.client().calls.load(Ordering::Relaxed);
        write_live(&f.paths, "A", "A", 200, Some(7_000));

        assert!(matches!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::Unverified { .. }));
        assert_eq!(f.client().calls.load(Ordering::Relaxed), calls);
    }

    #[test]
    fn sync_reports_unsaved_logged_out_and_unreadable_states() {
        let f = fixture();
        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::NotLoggedIn);
        log_in(&f.paths, "A", 100);
        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::NotSaved);
        write_credentials(&f.paths, json!({"accessToken": "x"}));
        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::LiveUnreadable);
    }

    #[test]
    fn switch_saves_the_renewed_login_of_the_account_it_leaves() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        save(&f, "b");
        log_in(&f.paths, "B", 500);

        f.manager.switch_to(&name("a")).unwrap();

        assert_eq!(saved_expiry(&f.paths, "b"), 500);
    }

    #[test]
    fn switch_installs_the_target_login_and_identity_and_keeps_everything_else() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 200);
        save(&f, "b");

        let switched = f.manager.switch_to(&name("a")).unwrap();

        assert_eq!(switched, Switched { from: Some(name("b")), to: name("a") });
        let creds = live_credentials(&f.paths);
        assert_eq!(creds["claudeAiOauth"]["expiresAt"], 100);
        assert_eq!(creds["mcpOAuth"], json!({"server": "machine-state"}));
        let config = live_config(&f.paths);
        assert_eq!(config["oauthAccount"]["accountUuid"], "A");
        assert_eq!(config["projects"], json!({"C:/x": 1, "c:/x": 2}));
        assert_eq!(entries(&f), [("a".into(), true), ("b".into(), false)]);
    }

    #[test]
    fn switch_refuses_to_abandon_an_unsaved_or_unreadable_login() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        assert!(matches!(f.manager.switch_to(&name("a")), Err(Error::LiveNotSaved)));

        write_credentials(&f.paths, json!({"accessToken": "at-B-1", "refreshToken": "rt-B-1"}));
        assert!(matches!(f.manager.switch_to(&name("a")), Err(Error::LiveUnreadable)));
        assert_eq!(live_credentials(&f.paths)["claudeAiOauth"]["accessToken"], "at-B-1");
    }

    #[test]
    fn switch_refuses_when_the_saved_copy_is_newer_than_the_live_login() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 900);
        save(&f, "b");
        log_in(&f.paths, "B", 300);

        assert!(matches!(f.manager.switch_to(&name("a")), Err(Error::GenerationConflict(_))));
        assert_eq!(live_config(&f.paths)["oauthAccount"]["accountUuid"], "B");
    }

    #[test]
    fn switch_rejects_unknown_and_current_accounts() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");

        assert!(matches!(f.manager.switch_to(&name("zzz")), Err(Error::UnknownAccount(_))));
        assert!(matches!(f.manager.switch_to(&name("A")), Err(Error::AlreadyCurrent(_))));
    }

    #[test]
    fn switch_works_when_logged_out() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        fs::remove_file(&f.paths.credentials_file).unwrap();

        assert_eq!(f.manager.switch_to(&name("a")).unwrap(), Switched { from: None, to: name("a") });
        assert_eq!(live_credentials(&f.paths)["claudeAiOauth"]["expiresAt"], 100);
    }

    #[test]
    fn an_account_in_use_by_another_install_is_neither_switched_to_nor_renewed() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        save(&f, "b");
        let a = f.manager.overview().unwrap().listing.get(&name("a")).unwrap().identity.clone();
        let wsl = Side::new(Platform::Wsl { distro: "Ubuntu".into() }, "/ccam".into(), Path::new("/elsewhere"));
        presence::record(&f.paths.vault_dir, &wsl, Some(&a)).unwrap();

        assert!(
            matches!(f.manager.switch_to(&name("a")), Err(Error::InUseElsewhere { side, .. }) if side == "WSL (Ubuntu)")
        );
        assert!(matches!(f.manager.check_switch(&name("a")), Err(Error::InUseElsewhere { .. })));
        assert!(matches!(
            f.manager.refresh_saved(&name("a"), UnixMillis::from_millis(1_000)),
            Err(Error::LoginInUse(_))
        ));
        assert_eq!(live_config(&f.paths)["oauthAccount"]["accountUuid"], "B");

        presence::record(&f.paths.vault_dir, &wsl, None).unwrap();
        f.manager.switch_to(&name("a")).unwrap();
        let seen_from_wsl = presence::others(&f.paths.vault_dir, &wsl);
        assert_eq!(seen_from_wsl.len(), 1);
        assert_eq!(seen_from_wsl[0].identity, a, "this install's record follows its switch");
    }

    #[test]
    fn a_login_claude_code_cleared_counts_as_logged_out() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        // What Claude Code leaves behind when a renewal is refused: the shape stays, the secrets go.
        write_credentials(&f.paths, json!({"accessToken": "", "refreshToken": "", "expiresAt": 0, "scopes": []}));

        assert_eq!(f.manager.sync(Network::Offline).unwrap(), SyncOutcome::NotLoggedIn);
        assert_eq!(f.manager.switch_to(&name("a")).unwrap(), Switched { from: None, to: name("a") });
        assert_eq!(live_credentials(&f.paths)["claudeAiOauth"]["expiresAt"], 100);
    }

    #[test]
    fn switch_waits_for_each_lock_and_changes_nothing_on_timeout() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        save(&f, "b");

        for held_by_someone_else in [&f.paths.vault_lock, &f.paths.refresh_lock, &f.paths.global_config_lock] {
            let _held = lock::acquire(held_by_someone_else, FAST).unwrap();
            assert!(matches!(f.manager.switch_to(&name("a")), Err(Error::LockTimeout { .. })));
            assert_eq!(live_config(&f.paths)["oauthAccount"]["accountUuid"], "B");
        }
    }

    #[test]
    fn a_failed_switch_restores_the_previous_identity() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        let broken = Manager::new(f.paths.clone(), Box::new(BrokenStore), Box::new(Arc::new(FakeClient::default())))
            .with_lock_timings(FAST_TIMINGS);

        // BrokenStore reports no live login, so the switch proceeds as if logged out and then fails to write.
        assert!(matches!(broken.switch_to(&name("a")), Err(Error::Keychain(_))));
        assert_eq!(live_config(&f.paths)["oauthAccount"]["accountUuid"], "B");
    }

    #[test]
    fn check_switch_refuses_what_the_switch_would_refuse() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        save(&f, "b");
        write_live(&f.paths, "B", "B", 200, Some(7_000));

        assert!(matches!(f.manager.check_switch(&name("a")), Err(Error::LiveUnverified { .. })));
        assert!(matches!(f.manager.check_switch(&name("b")), Err(Error::AlreadyCurrent(_))));
    }

    #[test]
    fn refreshing_a_saved_account_saves_the_rotated_login() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", i64::MAX / 2);
        save(&f, "b");

        let refreshed = f.manager.refresh_saved(&name("a"), UnixMillis::from_millis(10_000)).unwrap();

        assert_eq!(refreshed.login.refresh_token(), "rt-fresh");
        let saved = jsonio::read(&f.paths.vault_dir.join("a/login.json")).unwrap().unwrap();
        assert_eq!(saved["claudeAiOauth"]["refreshToken"], "rt-fresh");
    }

    #[test]
    fn refreshing_refuses_a_login_that_is_in_use() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");

        assert!(matches!(
            f.manager.refresh_saved(&name("a"), UnixMillis::from_millis(10_000)),
            Err(Error::LoginInUse(_))
        ));
        assert_eq!(saved_expiry(&f.paths, "a"), 100);
    }

    #[test]
    fn rename_keeps_the_current_marker_and_allows_case_only_changes() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");

        f.manager.rename(&name("a"), &name("Work-1")).unwrap();
        assert_eq!(entries(&f), [("Work-1".into(), true)]);
        f.manager.rename(&name("work-1"), &name("work-1")).unwrap();
        assert_eq!(entries(&f), [("work-1".into(), true)]);
    }

    #[test]
    fn rename_and_remove_validate_names_and_wait_for_the_vault_lock() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");
        log_in(&f.paths, "B", 100);
        save(&f, "b");

        assert!(matches!(f.manager.rename(&name("zzz"), &name("x")), Err(Error::UnknownAccount(_))));
        assert!(matches!(f.manager.rename(&name("a"), &name("B")), Err(Error::NameTaken(_))));
        assert!(matches!(f.manager.remove(&name("zzz")), Err(Error::UnknownAccount(_))));

        let _held = lock::acquire(&f.paths.vault_lock, FAST).unwrap();
        assert!(matches!(f.manager.remove(&name("a")), Err(Error::LockTimeout { .. })));
        assert!(matches!(f.manager.rename(&name("a"), &name("c")), Err(Error::LockTimeout { .. })));
    }

    #[test]
    fn remove_forgets_the_account_but_not_the_live_login() {
        let f = fixture();
        log_in(&f.paths, "A", 100);
        save(&f, "a");

        f.manager.remove(&name("A")).unwrap();
        assert!(entries(&f).is_empty());
        assert_eq!(live_credentials(&f.paths)["claudeAiOauth"]["expiresAt"], 100);
    }
}
