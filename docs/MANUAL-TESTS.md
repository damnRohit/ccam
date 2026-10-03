# Manual tests

These depend on a real Claude Code install and real logins, so CI cannot run them. Repeat them after a Claude Code release that mentions authentication, credentials, hooks or session handling in its changelog, and record the result below.

Use two low-stakes accounts. Never paste tokens or email addresses anywhere.

## 1. In-place switch is picked up by a running session

1. Logged in as account A: `ccam save a`. Then `/login` as B inside Claude Code and `ccam save b`.
2. In a running Claude Code session: `! ccam to a`.
3. Send a message, then run `/status`.

Pass: `/status` shows account A without restarting. macOS may need up to 30 s.

## 2. Restart flow (Windows Terminal)

1. Inside Claude Code in Windows Terminal: `! ccam to b --restart`.
2. A new tab opens in the same directory within a few seconds; the old tab closes about 5 s later.
3. The new tab runs `claude --continue`; `/status` shows B.

Pass: one new tab, the conversation continues, `ccam list` marks `b`. With an unknown name, nothing closes and the error prints in the old tab.

## 3. Token renewals keep saved logins fresh

1. `ccam setup`, then start a new Claude Code session.
2. Note `expiresAt` and `refreshTokenExpiresAt` in `accounts/<name>/login.json` (values only).
3. Wait for a renewal (about 8 h, or let the access token expire) and send a message.

Pass: `login.json` has a newer `expiresAt`, `refreshTokenExpiresAt` is unchanged, and `switch.log` has a line ending in exactly `sync '<name>'`. If `refreshTokenExpiresAt` changed, the "same session" assumption in `login.rs` no longer holds; ccam still works but confirms every renewal online.

## 4. `/login` inside a session

With hooks installed and account A saved, run `/login` in Claude Code and log in as an unsaved account C.

Pass: A's `login.json` is unchanged, `switch.log` has no new line ending in `sync 'a'` (a `sync 'a' skipped: ...` line is fine), and `ccam list` says the logged-in account is not saved.

## 5. macOS Keychain

On macOS with a normal Keychain login: `ccam save a`, `/login` as B, `ccam save b`, `ccam to a`.

Pass: `security find-generic-password -s "Claude Code-credentials"` exists, Claude Code works as A, and no `.credentials.json` appeared.

## 6. ccam auto

1. `ccam setup --auto`, then `ccam notify-test`: a terminal line with a bell and a desktop popup with an alarm sound.
2. In a tmux pane: `ccam auto --threshold 60` (a low threshold to reach quickly), and use Claude Code in another pane.
3. When the live account passes 60%: `ccam auto status` shows it armed with candidates read; at the end of the next turn it switches, notifies, and `ccam list` marks the new account.
4. `ccam auto stop` ends it within a few seconds; `ccam auto status` then says it is not running.

Pass: one switch, the next Claude message goes out on the new account (`/status`), no repeated switching.

## 7. Windows and WSL sharing a vault

Setup as in `docs/WINDOWS-WSL.md`, with three saved accounts a, b, c.

1. Windows `ccam to a`, WSL `ccam to b`. `ccam list` on each side marks the other's account `in use on ...`.
2. Windows `ccam to b`: refused, naming WSL, exit code 1. Nothing changes on either side.
3. Windows `ccam to b --force`: WSL moves to c (or whichever free account has the most room), then Windows switches to b. Both sides' `/status` agree with `ccam list`.
4. In WSL, `ccam usage --refresh`: the Windows account is not renewed (its card is read from Windows' side or reported in use).

Pass: at no point does either side need `/login`, and `switch.log` shows one switch per side per step.

## Results

| Date | Claude Code | OS | Tests | Result | Notes |
|---|---|---|---|---|---|
| | | | | | |
