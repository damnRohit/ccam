use std::io;
use std::path::PathBuf;

use crate::api::ApiError;
use crate::name::{AccountName, InvalidName};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    InvalidName(#[from] InvalidName),

    #[error("no saved account named '{0}'")]
    UnknownAccount(String),

    #[error("an account named '{0}' already exists")]
    NameTaken(AccountName),

    #[error("the logged-in account is already saved as '{0}'")]
    AlreadySavedAs(AccountName),

    #[error("'{0}' holds a different account; remove it first or pick another name")]
    NameHoldsOtherAccount(AccountName),

    #[error("not logged in to Claude Code")]
    NotLoggedIn,

    #[error("already on '{0}'")]
    AlreadyCurrent(AccountName),

    #[error("the logged-in account is not saved, so switching would lose its login. Run: ccam save <name>")]
    LiveNotSaved,

    #[error(
        "Claude Code's login could not be read (missing tokens or expiry), so ccam will not replace it. Run /login in Claude Code first"
    )]
    LiveUnreadable,

    #[error(
        "the logged-in login is new and ccam could not confirm it belongs to '{name}' ({reason}). Nothing was changed. Retry when online; if the problem persists, run /login again"
    )]
    LiveUnverified { name: AccountName, reason: String },

    #[error(
        "the saved login for '{0}' is newer than the logged-in one, which should not happen. Nothing was changed. Run /login for this account, then: ccam save {0}"
    )]
    GenerationConflict(AccountName),

    #[error("'{0}' is logged in right now (here or elsewhere with the same login), so ccam will not renew its token")]
    LoginInUse(AccountName),

    #[error(
        "'{name}' is in use by Claude on {side}; using it here too would log one of them out. Switch {side} to another account first, or run: ccam to {name} --force (moves {side} to the best free account)"
    )]
    InUseElsewhere { name: AccountName, side: String },

    #[error("{} is held by another process (waited {waited_secs}s)", path.display())]
    LockTimeout { path: PathBuf, waited_secs: u64 },

    #[error("lost the lock {} (the machine may have slept); nothing further was written", .0.display())]
    LockLost(PathBuf),

    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    #[error("{}: invalid JSON: {source}", path.display())]
    Json { path: PathBuf, source: serde_json::Error },

    #[error("{}: {reason}", path.display())]
    Malformed { path: PathBuf, reason: &'static str },

    #[error("cannot determine the home directory")]
    NoHome,

    #[error("macOS Keychain: {0}")]
    Keychain(String),

    #[error(transparent)]
    Api(#[from] ApiError),

    #[error("{0}")]
    Restart(String),
}

/// Process exit codes.
pub mod exit {
    /// The request cannot be done as asked: bad name, unknown account, wrong state.
    pub const USAGE: u8 = 1;
    /// Files, locks or the credential store failed.
    pub const ENVIRONMENT: u8 = 2;
    /// Anthropic's endpoints failed or were unreachable.
    pub const NETWORK: u8 = 3;
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidName(_)
            | Self::UnknownAccount(_)
            | Self::NameTaken(_)
            | Self::AlreadySavedAs(_)
            | Self::NameHoldsOtherAccount(_)
            | Self::NotLoggedIn
            | Self::AlreadyCurrent(_)
            | Self::LiveNotSaved
            | Self::LiveUnreadable
            | Self::GenerationConflict(_)
            | Self::LoginInUse(_)
            | Self::InUseElsewhere { .. } => exit::USAGE,
            Self::LiveUnverified { .. } | Self::Api(_) => exit::NETWORK,
            Self::LockTimeout { .. }
            | Self::LockLost(_)
            | Self::Io { .. }
            | Self::Json { .. }
            | Self::Malformed { .. }
            | Self::NoHome
            | Self::Keychain(_)
            | Self::Restart(_) => exit::ENVIRONMENT,
        }
    }
}
