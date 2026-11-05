# `ccam login <name>`: adding an account without touching the live login

Researched 2026-10-30 against Claude Code 2.1.314 (native install, Windows 11). The question: can ccam add an account by running Claude Code's own login in a throwaway config dir, importing the result into the vault, and deleting the dir, so the user never runs `/login` in a session (which replaces the main login)?

This file does not repeat `docs/research/internals.md` (credential paths, Keychain naming, rotation, locks) or `docs/SPEC.md`. It cites them by section.

Marks: **(inferred)** is my reading, not stated by a source. **(local)** was run or checked on this machine without reading any token, email, `~/.claude/.credentials.json`, `~/.claude.json` or vault file. **(binary)** is read from the strings of the local `~/.local/bin/claude.exe` 2.1.314. Minified names are quoted as they appear in that build and will differ in others.

Sources:
- AUTH = https://code.claude.com/docs/en/authentication
- CLI = https://code.claude.com/docs/en/cli-reference
- E = https://code.claude.com/docs/en/env-vars
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- CS = realiti4/claude-swap at `72ccd58612` (2026-10-28), https://github.com/realiti4/claude-swap
- CCEX = falconx1/ccex at `e5632b935f` (2026-09-28), https://github.com/falconx1/ccex
- CCS = kaitranntt/ccs at `f45fa923f2` (2026-09-11), https://github.com/kaitranntt/ccs
- CT, AUP, LC = the Consumer Terms, Usage Policy and legal page as quoted in `docs/research/window-warmup.md` section 4

---

## 1. `claude auth login` and its neighbours

### 1.1 What it is

- Added in 2.1.41: "Added `claude auth login`, `claude auth status`, and `claude auth logout` CLI subcommands" (CL 2.1.41). `--console` was added in 2.1.79 (CL 2.1.79). The changelog does not record when `--email` and `--sso` arrived. Both were already present on 2.1.274 (internals.md 2.4).
- It is documented: "`claude auth login`: Sign in to your Anthropic account. Use `--email` to pre-fill your email address, `--sso` to force SSO authentication, and `--console` to sign in with Anthropic Console" (CLI, CLI commands table). AUTH "Log in to Claude Code" says a `claude auth login` run before the first session lets first launch skip the login prompt.
- Local help (`claude auth login --help`, 2.1.314) **(local)**:
  ```
  --claudeai       Use Claude subscription (default)
  --console        Use Anthropic Console (API usage billing) instead of Claude subscription
  --email <email>  Pre-populate email address on the login page
  --sso            Force SSO login flow
  ```
  `claude auth` has only `login`, `logout` and `status`. `logout` takes no options. `status` takes `--json` (default) and `--text`.
- Passing both `--console` and `--claudeai` is an error. Managed settings `forceLoginMethod` / `forceLoginOrgUUID` still apply, and a gateway-forced machine refuses with "run interactive /login" **(binary)**, `authLogin` body.

### 1.2 How the flow runs

**(binary)** The `authLogin` handler (`async function Ye({email,sso,console,claudeai},...)`):
1. Prints `Opening browser to sign in…`, then `If the browser didn't open, visit: <url>`, and `Paste code here if prompted > `.
2. Runs an OAuth authorization-code flow with a localhost callback listener (`http://localhost:<port>/callback`). At the same time it reads stdin line by line for a pasted `code#state`, which is the manual fallback CL 2.1.126 added: "`claude auth login` now accepts the OAuth code pasted into the terminal when the browser callback can't reach localhost (WSL2, SSH, containers)".
3. If the callback listener cannot start, it falls back to printing `Sign in at: <url>` and waiting for a paste. That fallback is only armed when `process.stdin.isTTY`. Without a TTY a listener failure is fatal.
4. On success it prints `Login successful.` and exits 0. A failure prints `Login failed: ...` and exits 1. Stdin closing before a code was pasted exits 130 ("Input ended before a code was pasted").
5. `--email` becomes the authorize URL's login hint, and `--sso` sets the login method to `sso` (`RZr({...loginHint, loginMethod...})`).

It is a plain subcommand. It does not start the REPL, so there is no onboarding, theme or trust dialog. On first launch the docs show the same flow inside `claude` (AUTH "Log in to Claude Code"). The only extra TTY-gated step after success is a one-time "Your organization may apply global configuration" notice **(binary)**, function `X`.

**(inferred)** A browser and a human are required. `--email` is only a pre-fill and cannot pick the account. The authorize page uses whatever claude.ai session the browser already has, so a user signed in to their main account can approve as the wrong account. GaiusSheh/cc-account-switcher tells users to "sign out there or use a private window first" (README at `e31af03211`, "Add New Account").

### 1.3 What a successful login writes, and where

**(binary)** After the token exchange, `O9e` → `Kgo`:
- First it clears the previous login in this config dir by calling `E0e({clearOnboarding:false, preserveInProcessTokens:true, ...})`. With `preserveInProcessTokens:true` the revoke branch is skipped (section 1.5), so a login does **not** revoke whatever login the dir held before.
- It fetches the profile (`/api/oauth/profile`, or uses the one returned with the token) and writes `oauthAccount` (`accountUuid`, `emailAddress`, `organizationUuid`, `displayName`, `billingType`, `seatTier`, `profileFetchedAt`, ...) into the global config via `wCn`.
- It saves the credentials through the platform store under Claude Code's refresh lock (`Ieo(e,n,"sign_in")` inside the lock helper).
- It then fetches roles (`ROLES_URL`) and adds `organizationRole`, `workspaceRole` and `organizationName` to `oauthAccount`. For claude.ai scopes it also fetches `/api/organization/claude_code_first_token_date` into the global config. A Console login creates an API key instead.
- Last, it re-reads and validates the stored login (`fD`), and tries to restore the previous one if the new one did not land (`stn`).

The global config path is `process.env.CLAUDE_CONFIG_DIR || <home>` joined with `.claude.json` **(binary)**, function `Fbr`, which matches internals.md 2.3. With `CLAUDE_CONFIG_DIR=<tmp>`, `oauthAccount` lands in `<tmp>/.claude.json`.

### 1.4 `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`: the browserless variant

E: "`CLAUDE_CODE_OAUTH_REFRESH_TOKEN`: OAuth refresh token for Claude.ai authentication. When set, `claude auth login` exchanges this token directly instead of opening a browser. Requires `CLAUDE_CODE_OAUTH_SCOPES`." The binary's own warning reads: "The refresh token in CLAUDE_CODE_OAUTH_REFRESH_TOKEN was used for this attempt and cannot be used again" **(binary)**.

**(inferred)** This spends a refresh token. It is useless for adding a new account, and dangerous if inherited: if the variable leaked into ccam's child environment, the "login" would spend some other copy's token. ccam must strip it.

### 1.5 `claude auth logout` revokes the refresh token server-side

- **(binary)** `E0e` with `preserveInProcessTokens:false` (the logout path, function `qe`) reads the stored `claudeAiOauth`, `designOauth` and `cloudSessionsOauth`. For each refresh token it calls `sS` → `bCn`, which POSTs `{token, token_type_hint:"refresh_token", client_id}` to `${TOKEN_URL}/revoke`. Only then does it delete the stored credential.
- AUTH (Console keyless sign-in): "`/logout`, which removes and revokes the credential this sign-in wrote."
- CS README "Add more accounts" (added in `8bb6566de5`, 2026-08-11): "Do not run `/logout` first: current Claude Code may revoke the refresh token stored for the account you are leaving."

**Consequence for ccam:** never run `claude auth logout` (or `/logout`) in the temp dir after importing. The temp dir and the vault would hold the same refresh token, and logout would kill the vault copy. Delete the directory instead. The same applies to an unwanted login (wrong account): **(inferred)** delete it rather than revoke it. Whether a revoke kills only that token or the account's whole grant is not documented.

### 1.6 `claude auth status`

- CLI: JSON by default, `--text` for humans, exit 0 when logged in and 1 when not. `configDirectory` arrived in 2.1.268 (CL 2.1.268). `authMethod` is one of `none`, `claude.ai`, `oauth_token`, `api_key`, `api_key_helper`, `third_party`.
- **(binary)** For `claude.ai` it also prints `email`, `orgId`, `orgName` and `subscriptionType`. ccam must never echo this output.
- Every command, `auth status` included, starts an OAuth refresh at init when the stored token is near expiry, and can exit before saving the result. That leaves a spent refresh token behind: https://github.com/anthropics/claude-code/issues/95822 (open, reproduced on macOS, Linux and Windows). CL 2.1.301 fixed one variant ("unexpected sign-outs after a short command such as `claude auth status` ... exited in the middle of a token refresh"). **(inferred)** ccam should run no `claude` command against a config dir holding a login it has imported. A fresh login is far from expiry, so the window is small, but ccam should delete the temp dir as soon as it has imported.

### 1.7 `claude setup-token` as an alternative

- AUTH "Generate a long-lived token": "generate a one-year OAuth token with `claude setup-token`". It "opens the same browser authorization flow as `/login`", and the token "prints to the terminal ... It does not save the token anywhere". The token "can only make model requests, so it can't establish Remote Control sessions or fetch claude.ai connectors". `--bare` does not read it.
- CLI: "Prints the token to the terminal without saving it. Requires a Claude subscription." 2.1.314 help shows no options **(local)**.
- CS treats setup-tokens as inference-only: "Setup-tokens are inference-only server-side; wider scopes trigger 403s on profile endpoints" (CS `src/claude_swap/switcher.py` line 111-113, `SETUP_TOKEN_SCOPES = ("user:inference",)`). Its `add-token` stores them with a synthetic `oauthAccount` that has an empty `accountUuid` (switcher.py, `add_token`).

Trade-offs for ccam **(inferred)**:
- For: no refresh token, so no rotation and nothing to spend twice. It lasts a year.
- Against: no `user:profile` scope, so the profile endpoint cannot confirm identity (invariant 2) and `/api/oauth/usage` cannot show usage. There is no `refreshToken`, so it does not parse as an `OAuthLogin` (SPEC section 3 requires one). It cannot do Remote Control or connectors. Its only supported use is the `CLAUDE_CODE_OAUTH_TOKEN` variable, not the live credentials file, so `ccam to` could not switch to it. And it prints the secret to the terminal.
- Verdict: a different account kind, not a way to add a normal account. Out of scope for `ccam login`.

---

## 2. Running it in a throwaway config dir

### 2.1 Empty-dir probe **(local)**

`CLAUDE_CONFIG_DIR=<new empty temp dir> claude auth status --json`, with the Claude Code session variables removed:

```json
{ "loggedIn": false, "authMethod": "none", "apiProvider": "firstParty", "analyticsDisabled": false,
  "projectsDirectory": "<tmp>\\projects", "configDirectory": "C:/Users/<short>/AppData/Local/Temp/tmp.XXXX" }
```

- Exit code 1. `--text` printed `Not logged in. Run claude auth login to authenticate.`
- Afterwards the temp dir held `.claude.json` (only `firstStartTime`, `firstStartVersion`, `machineID`, migration flags, `seenNotifications`) and `backups/.claude.json.backup.<ms>`. There was no `.credentials.json` and no `<tmp>.lock` sibling.
- `~/.claude.json` and `~/.claude/.credentials.json` mtimes were older than the temp dir, so neither was touched. The temp dir was deleted afterwards.
- `configDirectory` is the raw string the process received, not a resolved path: Git Bash's own `/tmp/...` rewrite came back verbatim. This is the same `Fe()` value the Keychain suffix hashes (2.2).

### 2.2 Where the login lands

| Platform | Tokens | Identity |
|---|---|---|
| Windows | `<tmp>/.credentials.json` (AUTH "Credential management": with `CLAUDE_CONFIG_DIR` "Claude Code keeps the `.credentials.json` file under that directory"). See the Credential Manager caveat below. | `<tmp>/.claude.json` `oauthAccount` (1.3) |
| Linux / WSL | `<tmp>/.credentials.json`, mode 0600 (AUTH) | same |
| macOS | Keychain service `Claude Code-credentials-<sha256(NFC(value of CLAUDE_CONFIG_DIR))[0..8]>`, account `$USER` (internals.md 2.2). If the Keychain refuses the write, `<tmp>/.credentials.json` (AUTH). | same |

macOS service name, confirmed in 2.1.314 **(binary)**:

```js
function ple(e=""){let n=process.env.CLAUDE_SECURESTORAGE_CONFIG_DIR,
  t=n!==void 0?!n:!process.env.CLAUDE_CONFIG_DIR,
  r=n!==void 0?n.normalize("NFC"):Fe(),
  a=t?"":`-${sha256(r).hex.substring(0,8)}`;
  return`Claude Code${OAUTH_FILE_SUFFIX}${e}${a}`}   // e = "-credentials"
```

- ccam picks the string it puts in `CLAUDE_CONFIG_DIR`, so it can compute the service name itself. `claude auth status --json`'s `configDirectory` prints the exact hashed string, which is a cheap check in a test step. CCEX computes the same name independently: "the value of CLAUDE_CONFIG_DIR, NFC-normalised" (`lib/py/creds.py` `service_for`, lines 108-118). After a scratch-dir login it rewrites the item under the new dir's name (`lib/py/rekey.py`), which confirms that the item is keyed per dir.
- A hashed item can carry an ACL that only lists `/usr/bin/security`, so Claude Code itself cannot read it after a reboot (https://github.com/anthropics/claude-code/issues/20553, comment on v2.1.56; #19456 closed). **(inferred)** That does not hurt ccam, which reads and deletes through `/usr/bin/security` (SPEC section 8), and the item is deleted right after import anyway.

**Hazard: `CLAUDE_SECURESTORAGE_CONFIG_DIR`.** **(binary)** `tH()`: when this variable is defined, secure storage keys on its value, or on `~/.claude` when it is defined but empty. That covers the Keychain item and also the directory `.credentials.json` is written to. The Agent SDK sets it for the processes it spawns (`...CLAUDE_SECURESTORAGE_CONFIG_DIR = ... ?? process.env.CLAUDE_CONFIG_DIR ?? ""`). **(inferred)** If ccam inherited it, defined and empty, a "temp" login would write the main login's store, which is exactly the failure this command exists to avoid. It is undocumented (E has no match). ccam must remove it from the child's environment.

**Caveat: Windows Credential Manager backend.** **(binary)** 2.1.314 contains a `windows-credman` store built on `Bun.secrets`, with service `ple("-credentials")`, account `claude-code-user`, chunked entries, a 2 s timeout and a plaintext fallback (`windows-credman-with-plaintext-fallback`).
- It is chosen when `CLAUDE_CODE_FORCE_WINDOWS_CREDMAN=1`, or when the config dir's global config has `cachedGrowthBookFeatures.tengu_windows_credman === true`, a server-side feature flag (functions `Ygr`, `he`, `Mve`).
- The choice is made once per process from the config dir being used. **(inferred)** A brand-new temp dir has no cached flags, so the temp login lands in `<tmp>/.credentials.json` even if the main install has been moved to Credential Manager. To be sure, ccam should unset `CLAUDE_CODE_FORCE_WINDOWS_CREDMAN` in the child and check that the file exists after login.
- `cmdkey /list` on this machine shows no `Claude Code*` target, so the main install is still on the file store **(local)**.
- Neither the docs nor the changelog mention this backend. **(inferred)** Once Anthropic turns that flag on, ccam's file-only Windows store would read the main install as logged out. That is a separate risk to ccam as a whole, not just to `login`. It belongs in internals.md and MANUAL-TESTS.

**Locks.** Claude Code's legacy refresh lock is `<config home>.lock`, a sibling of the dir (internals.md 3.2). A temp login in `%TEMP%\x` may leave `%TEMP%\x.lock` behind if killed. ccam should remove it together with the dir.

### 2.3 What the login can touch outside the temp dir

- **Managed settings** (`forceLoginMethod`, `forceLoginOrgUUID`, gateway) always apply (1.1). This is intended.
- **`~/.claude/.device-keys.json`**: a per-machine device-key store at a fixed `homedir()/.claude` path that ignores `CLAUDE_CONFIG_DIR`. It is used for device registration and trusted-device binding, and has messages such as "deviceRegistry: account device limit reached" **(binary)**, `Mrr()` and the `deviceKeyStore` / `deviceRegistry` strings. The file does not exist on this machine **(local)**. **(inferred)** I found no call from `authLogin` into the registry, but I could not rule out a later startup step registering the new account's device. It is machine state that ccam neither reads nor writes.
- **The credentials file's other keys.** Logout deletes `claudeAiOauth`, `organizationUuid`, `trustedDeviceToken`, `trustedDeviceEnrolledAfterRefusalAtMs`, `designOauth` and `cloudSessionsOauth` **(binary)**, `E0e`. **(inferred)** The temp dir's `.credentials.json` may therefore hold sibling keys besides `claudeAiOauth`. ccam imports only `claudeAiOauth` (SPEC invariant 7), so those are dropped with the temp dir. This matches what `ccam to` already does.
- **Remote Control on the main login.** Since 2.1.234, "signing this computer in to a different claude.ai account or organization now stops the running session within seconds" (CL 2.1.234). https://github.com/anthropics/claude-code/issues/88855 reports a sign-in under a different `CLAUDE_CONFIG_DIR` dropping Remote Control in an unrelated session (closed as stale 2026-10-15, no fix noted). SPEC 9.7 already records this for lanes. **(inferred)** `ccam login` of another account can do the same thing to main-login Remote Control sessions.
- **`~/.claude/ide`** is read in addition to the config dir's `ide` when `CLAUDE_CONFIG_DIR` is set **(binary)**, `znr`. It is read-only and irrelevant to `auth login`.
- **Project settings** can no longer set `CLAUDE_CONFIG_DIR` (CL: "Changed project-level `.claude/settings.json` `env` to no longer set `CLAUDE_CONFIG_DIR`"). **(inferred)** Running the child with the temp dir as its working directory avoids any project settings at all.
- **No ccam hooks fire.** The temp dir has no `settings.json`, so no `FileChanged` or `Notification` hook runs. The user's main hooks belong to sessions under the main config dir, not to this subprocess **(inferred)** from H as summarised in internals.md 1.5.

---

## 3. Does a fresh login affect other logins of the same account?

Evidence:
- **Login does not revoke the dir's previous login**, and it calls no revoke endpoint at all. Only logout revokes **(binary)**, sections 1.3 and 1.5.
- **Each config dir holds an independent session.** "Each entry has its own independent OAuth session with separate access tokens, refresh tokens, and expiry times" (issue #20553, comment confirming v2.1.56). AUTH "Log in with multiple accounts": "To stay signed in to multiple accounts at once ... give each account its own configuration directory".
- **A re-login of a known account is the recovery path in prior art.** That only works if the new session stands alone. CS README: a dead-token account is "quarantined ... until you either log in with it and re-run `cswap add --slot N`". CCEX README: "Logging in as an account you already have re-authenticates that slot rather than making a second one". GaiusSheh `src/extension.ts` (`handleAddNewAccount`): "Same account signed in again: refresh its saved credentials".
- **ccam already models sessions as distinct.** SPEC section 2, "Session: the lifetime of one `/login`; renewals within it share `refreshTokenExpiresAt` (*observed, undocumented*)". Invariant 2 treats any login outside the session as new and confirms it through the profile endpoint.
- **What does kill sessions** is replaying a spent refresh token. internals.md 3.1 covers single-use rotation, and #93879 (open) shows "OAuth access token has been revoked" followed by "refresh token is no longer valid". Third-party commenters there read it as "whole-family revocation" on reuse, and flag that as a guess. **(inferred)** A family is one login session, so a new login starts a new family and leaves the others alone.
- **Revocation is weakly enforced anyway.** Issue #43801 (closed) reports that "Log out all sessions" and "Revoke all Claude Code instances" on claude.ai left existing tokens working for days.

Unknowns:
- Whether Anthropic caps concurrent sessions per account. No doc, changelog entry or issue says so. #95349 (open, single report) claims a stuck 5-hour bucket after logging in to a second account on one Mac. It is anecdotal and about usage accounting, not token validity.
- Whether `POST /v1/oauth/token/revoke` revokes one token or the whole account grant (1.5).

Conclusion **(inferred, well supported)**: logging in again as an account already saved or live elsewhere creates a second, independent token family and does not invalidate the existing copy. The problem it does create is ccam's own. The vault holds one login per identity, and that slot is the sync target of whichever install is live on it. Overwriting it with the new session would make the live install's login "older" than the saved one, which is a generation conflict under invariant 1, so `to` would refuse. `ccam login` should therefore refuse an identity that is already saved under another name, and one that is live on any install (presence records, invariant 8). It should then discard the temp login by deleting the dir, never by logging out.

---

## 4. Prior art: adding an account without disturbing the active one

| Tool | How it adds an account | Touches the live login? | Source |
|---|---|---|---|
| **falconx1/ccex** `ccex add [name]` | Exactly the candidate design. Makes a scratch dir `$ROOT/.adding`, links shared files, seeds onboarding and trust from the main config, runs `CLAUDE_CONFIG_DIR="$d" claude auth login "$@"`, reads the identity, then moves the dir into a named slot. On macOS it rewrites the Keychain item under the new dir's hashed name (`rekey.py`). Same account again: replaces the old slot ("re-authenticated"). The account already live in `~/.claude`: discards the scratch dir ("nothing to park"). | No | `bin/ccex` lines 412-455; `lib/py/rekey.py`; `lib/py/creds.py` 108-118; `lib/py/seed.py`; README "ccex add" |
| **kaitranntt/ccs** `ccs auth create <name>` | Creates a persistent instance dir and spawns interactive `claude` (not `auth login`) with `stdio: 'inherit'` and `CLAUDE_CONFIG_DIR=<instance>`. It strips `CLAUDECODE` (`stripClaudeCodeEnv`) and ambient provider credentials first. The login stays in that dir for good, with no import. Rolls back the instance if login fails. | No | `src/auth/commands/create-command.ts` lines 245-300 |
| **ftery0/claude-account-switch** `cpf add` / `cpf login <p>` | One persistent `CLAUDE_CONFIG_DIR` per profile. "Sign `work2` in to another Claude account. Other profiles keep theirs". | No | README at `f821beac32`, command table |
| **realiti4/claude-swap** `cswap add` | Captures whatever is live. To add another account, the user logs in to it in Claude Code first, which replaces the live login. `add-token` registers a setup-token or API key without any login. Its session profiles (`cswap run`) are seeded from stored backups, not from a new login. | Yes (the user's `/login`) | README "Add more accounts", "Add an account from a raw token"; `switcher.py` `add_token` |
| **GaiusSheh/cc-account-switcher** (VS Code) | Backs up the live account, runs `claude auth login` in a terminal against the live config, detects the new identity, registers it, and keeps the old one restorable. | Yes (replaces live, then relies on backup) | `src/extension.ts` lines 324-420 at `e31af03211` |
| **ming86/cc-account-switcher** | Manual: log in, `--add-account`, log out, log in to the next account, `--add-account` again. | Yes, and its "Log out" step can revoke (1.5) | README "First Time Setup" at `c79a8edf62` |

Takeaways **(inferred)**:
- CCEX shows the temp-dir `claude auth login` approach works in practice on Linux and macOS, including Keychain re-keying.
- ccam differs in one way: it imports into its vault and deletes the dir instead of keeping it, so on macOS it reads and deletes the hashed item rather than rewriting it.
- No tool revokes an unwanted login. All of them delete or ignore it.

---

## 5. Rate limits and terms

- **Login rate limits:** none documented. CL has no entry on OAuth login or token-exchange throttling, and issue searches ("OAuth login 429", "login rate limited", "too many requests login") found nothing relevant. The only login-adjacent limit is the device registry's "account device limit reached" string **(binary)**, which is undocumented. Treat repeated logins as ordinary user actions. Adding an account once is not a loop.
- **Terms**, quoted in `docs/research/window-warmup.md` section 4:
  - CT section 2 forbids sharing account credentials with anyone else.
  - CT section 3 forbids automated access "except ... where we otherwise explicitly permit it".
  - LC says developers "may not collect, store, or intermediate Claude.ai credentials or session tokens".
  - **(inferred)** `ccam login` is the user signing in to their own account in the browser by hand, through Claude Code's own client. Nothing is automated. The new concern is narrow: it adds one more locally stored copy of the user's own login, which the vault already does (window-warmup.md 4, point 7). Nothing in CT or AUP limits how often one person logs in to their own accounts.

---

## 6. Verdict for ccam

**Viable.**
- The flow is documented: `claude auth login` with a per-dir `CLAUDE_CONFIG_DIR`, and AUTH's multi-account section.
- On all three platforms it writes only into the temp dir (tokens and `oauthAccount`), or into a per-dir Keychain item on macOS.
- It neither revokes nor rewrites the main login.
- CCEX ships the same design.

Shape **(inferred)**, for the design step:
1. Validate `<name>` and check it is free (SPEC section 3) before opening a browser. Refuse inside Claude Code, as `run` does, and refuse without a TTY, since the paste fallback needs one (1.2).
2. Create a unique empty dir outside the vault, not linked to it. **Do not** add the lane-style `ccam/accounts` junction: deleting the dir through a junction deletes the vault (SPEC 9.7).
3. Spawn `claude auth login --claudeai` with inherited stdio, working directory set to the temp dir, and `CLAUDE_CONFIG_DIR=<tmp>`. Remove every `CLAUDE*` and `ANTHROPIC*` variable first, as `start` already does (SPEC 8a). That removal must cover `CLAUDE_SECURESTORAGE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`, `CLAUDE_CODE_OAUTH_SCOPES` and `CLAUDE_CODE_FORCE_WINDOWS_CREDMAN`. Wait for exit 0.
4. Read `<tmp>/.credentials.json` (or, on macOS, the hashed Keychain item, falling back to the file) and `<tmp>/.claude.json` `oauthAccount`. Parse them with the existing `OAuthLogin` and `Identity` types.
5. Confirm identity through the profile endpoint (invariant 2; this is a new session). Then, under the vault lock, refuse if the identity is saved under another name, if `<name>` holds a different account, or if the identity is live on any install (section 3). Otherwise write the vault entry.
   - Unlike `Manager::save`, this must **not** write a presence record. The temp dir is not an install and will not exist a moment later.
   - **(inferred)** The temp login is not live on any install, so Claude Code's refresh locks for the main home are not involved. Invariant 5 holds as long as the network call happens before the vault write, as `save` already does.
6. Delete the temp dir, `<tmp>.lock`, and on macOS the `Claude Code-credentials-<sha8>` item. Do this on every exit path, including refusal and failure, and never through `claude auth logout`.

Remaining risks:
- **Wrong account in the browser.** The user approves as an account that is already saved, often the main one. Step 5 catches it. The message should suggest a private window. The discarded login stays valid server-side until it expires (an orphaned session, not harmful).
- **Windows Credential Manager rollout** (2.2). It is unlikely to affect a fresh temp dir. It is a real threat to ccam's Windows file store once the flag reaches the main install. Add a MANUAL-TESTS step that checks `cmdkey /list` for `Claude Code-credentials` per release.
- **Environment leakage** (`CLAUDE_SECURESTORAGE_CONFIG_DIR`, `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`). Mitigated by step 3 and covered by a test that the child gets a clean environment.
- **Remote Control drop** on main-login sessions when a different account signs in on the machine (2.3).
- **Leftovers after a crash** (temp dir with live tokens, `<tmp>.lock`, a macOS Keychain item). **(inferred)** Use a fixed parent such as `<vault>/../login-tmp/` (still not inside `accounts/`), so the next `ccam login` can sweep stale dirs and their Keychain items.
- **Undocumented internals** this relies on: the `.credentials.json` / `oauthAccount` layout, the macOS hash formula, and `CLAUDE_SECURESTORAGE_CONFIG_DIR`. They should go into internals.md (version 2.1.314) and be marked *undocumented* in SPEC when the command is built, with a MANUAL-TESTS step for a real login on each platform. No automated test can run a real browser login.
