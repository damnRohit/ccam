# ccam

ccam keeps several Claude subscription logins and decides which one each Claude Code config dir runs on, without ever letting two places spend one login.

## Language

### Accounts and logins

**Account**:
A saved Claude subscription login under a user-chosen name, kept in the vault.
_Avoid_: profile, user

**Login**:
The live tokens a config dir is using right now. It belongs to exactly one account.
_Avoid_: credentials, session (when meaning tokens)

**Main login**:
The login of the default config dir (`~/.claude`). `ccam to` moves it.
_Avoid_: global account, default lane

### Lanes

**Lane**:
A ccam-managed config dir, numbered or named, that holds at most one account and keeps it until moved or released. It shares history with the main login.
_Avoid_: pin, profile, slot

**Move**:
Changing which account a lane holds. Every session in the lane follows.
_Avoid_: switch (reserved for the main login)

**Release**:
Handing a lane's account back so the main login or another lane can take it. The lane stays, holding no account.

**Session**:
One running Claude Code conversation process inside a config dir: the main login's or a lane's.
_Avoid_: thread (that is T3's word), terminal

### T3 Code

**Instance**:
A T3 Code provider entry. For Claude, it names the config dir its threads run in. Each lane has exactly one instance, labelled with the account the lane holds.
_Avoid_: provider, lane

**Thread**:
A T3 Code conversation. It runs as a session in its instance's config dir.
_Avoid_: session (when meaning the T3 object), chat

**Child agent**:
A thread started by another thread through T3's orchestration. A Claude child runs in its parent's instance, and so in its parent's lane.
_Avoid_: subagent (Claude Code's in-session helper, a different thing)

### Adding accounts

**Fresh login**:
A login made by `ccam login` in a throwaway config dir. The name is given before signing in, and the login is saved only if it belongs to the account that name means.
_Avoid_: re-login, /login (which replaces the main login)
