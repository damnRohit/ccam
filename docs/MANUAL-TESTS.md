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

## 8. Lanes (`ccam run`, `move`, `release`)

Three saved accounts a, b, c, main login on c, `ccam setup` done. Keep the `[lane ...]` lines of `switch.log` with the result.

1. Terminal 1: `claude`. Terminal 2: `ccam run a` (prints `Lane 1`). Terminal 3: `ccam run b` (lane 2). `/status` shows c, a and b.
2. In terminal 1, `! ccam to` another free account, or leave it. Terminals 2 and 3 keep their account, and their next reply still reads from the cache (`/cost` or the usage line).
3. In terminal 2, `/resume` lists conversations started in terminal 1, and `/rewind` offers code checkpoints for one of them.
4. In terminal 2, ask Claude to remember something. It saves without asking for approval, and the note is in `~/.claude/projects/<project>/memory/`. A plain `claude` in that project recalls it.
5. Terminal 4: `ccam run a` joins lane 1. `ccam sessions` lists both lane 1 sessions, lane 2's and the main one.
6. Wait for (or force, by editing `expiresAt` to the past) a renewal in lane 1. `switch.log` has `[lane 1] sync 'a'`.
7. `ccam move 1 b` is refused (b is in lane 2). `ccam move 1 <free account>` asks, then terminals 2 and 4 show the new account in `/status` after their next message; terminal 3 does not change.
8. `ccam release 1` is refused while terminals 2 and 4 are open. Close them, `ccam release 1`, then `ccam to <that account>` in terminal 1 works without `/login`.
9. `~/.claude/projects` still holds every transcript, and Remote Control in terminal 1 (if used) is still connected (anthropics/claude-code#88855).

Pass: no `/login` at any step, no memory approval prompts, and a lane changes account only on `move`.

## 9. `ccam login`

Accounts a and b saved, main login on a, one Claude Code session open on it. A third claude.ai account d that is not saved. Keep the `login` lines of `switch.log` with the result.

1. `ccam login d`. The browser opens; sign in as d. ccam prints `Saved 'd'. Switch to it with: ccam to d`.
2. The open session's `/status` still shows a. `ccam list` shows d saved and a still marked `*`. `switch.log` has a line ending in `login 'd' (new)`. `~/.claude/ccam/login/` is empty and no `login/*.lock` remains. On macOS, `security dump-keychain` lists no `Claude Code-credentials-<hash>` item beyond the ones that existed before.
3. `ccam to d`, then send a message. It works without `/login`, which shows the saved login was not revoked. `ccam to a` back.
4. Private window case. In a browser still signed in to a, run `ccam login e` and approve as a. ccam prints `you signed in to 'a', which is already saved, not to a new account; nothing was saved` and suggests a private window. `ccam list` is unchanged and `login/` is empty. Run it again in a private window and sign in as a fourth account: it is saved as `e`.
5. A name holding a different account. `ccam login a`, signing in as d: refused with `'a' holds a different account`. Nothing changes.
6. Refresh. With d in use nowhere, `ccam login d` and sign in as d. ccam prints `Refreshed the saved login of 'd'.`, `switch.log` has `login 'd' (refreshed)`, and `login.json` has a newer `expiresAt` (compare values only). Then `ccam to d` and run `ccam login d` again: refused before the browser opens with `'d' is in use on the main login, so its login works and needs no new sign-in; nothing was changed`, and `login.json` is unchanged. Repeat with d in a lane (`ccam lane d`) and, if you have WSL, on the other install: each is refused the same way with `lane N`, `Windows` or `WSL (<distro>)`.
6a. Unsaved account that is live. Log Claude Code in as an unsaved account g and run `ccam login h`, signing in as g: ccam prints `the account you signed in to is the one the main login uses, and it is not saved; nothing was saved. Save it from there instead: ccam save h` and exits 1. Nothing is saved. Then put g in a lane or on the other install without saving it, run `ccam login h` as g again: `the account you signed in to is in use on lane N` (or `Windows`, `WSL (<distro>)`), exit 1, nothing saved.
6b. Offline. Turn the network off after the browser sign-in finishes, or run with `CCAM_API_BASE` pointing at a closed port. ccam prints `could not confirm which account you signed in to (...); nothing was saved. Retry when online`, exits 3, `ccam list` is unchanged and `login/` is empty.
6c. Unreadable entry. Break `accounts/d/login.json` (invalid JSON) after copying it elsewhere, then run `ccam login d`: refused before the browser opens with `'d' exists in the vault but cannot be read (...); nothing was saved. Fix or delete .../d first`, exit 1. `ccam remove d` says `no saved account named 'd'`. Restore the file.
7. Cancel. Start `ccam login f` and close the browser tab, or press Ctrl+C in the terminal. ccam prints `the sign-in did not finish (claude exited with N); nothing was saved` (or is stopped), and `ccam list` is unchanged.
8. Crash leftovers. Kill ccam while the browser is open, so its cleanup does not run. Check that `~/.claude/ccam/login/` holds a folder. The next `ccam login <name>` deletes it before it starts.
9. Each release, on Windows: `cmdkey /list` shows no `Claude Code-credentials` target. If it does, Claude Code has moved to Credential Manager. `ccam login` then prints `Claude Code reported a login but ccam cannot read it` and saves nothing, and ccam as a whole cannot read the main login either.

Pass: in every step the main login, lanes and open sessions keep working with no `/login`, no `claude auth logout` is run (watch the process list during step 1), and the `login/` folder is empty after each command. If Remote Control is on in the open session, note whether it drops when d signs in (anthropics/claude-code#88855).

## 10. Lanes in T3 Code

T3 Code installed and signed in to Claude, a project open in it. Three saved accounts a, b, c, main login on c, `ccam setup` done. Copy `~/.t3/userdata/settings.json` before you start.

1. **Memory in full-access mode.** Run `ccam lane a`. In T3, start a thread on the instance `Claude · lane 1 · a` in full-access mode and ask Claude to remember a fact, then to list its memory. Pass: the note saves and the listing runs with no approval prompt, and the note is in `~/.claude/projects/<project>/memory/`. A plain `claude` in the project recalls it. If a prompt appears, anthropics/claude-code#98044 also hits `bypassPermissions` in T3: record it, and see Gap 1 of `docs/research/t3code.md` for the planned fix. The rest of this section does not depend on the result.
2. **Creating the instance.** `ccam lane a` did not open Claude and printed `Lane 1 holds 'a'.` and `T3 Code: lane 1 is in the model picker as 'Claude · lane 1 · a'.`. `settings.json` now has `providerInstances.ccam-lane-1` with `driver` `claudeAgent`, `displayName` `Claude · lane 1 · a`, `enabled` `true` and `config.homePath` set to the lane folder. The instance appears in T3's model picker within a few seconds, with no restart. A diff against your copy shows changes only under `ccam-lane-*`.
3. **Reuse and naming.** Run `ccam lane a` again: it prints the same two lines for lane 1, and `settings.json` is not rewritten (its modified time does not change). `ccam lane b --lane api` prints `Lane api holds 'b'.` and creates `ccam-lane-api`. `ccam lane c` is refused with `'c' is the main login; open it with plain `claude`, ...`, and so is `ccam lane b --lane 1` (`'b' is in lane api` ...). `ccam lane a --lane api` is refused too.
4. **Parallel accounts.** Start a thread on each of the two lane instances and send a message in both at the same time. `ccam sessions` shows each session under its lane, and `ccam list` marks a and b as in use. A thread on the default instance still runs on c.
5. **Child agents.** In a thread on the lane 1 instance, ask Claude to start a child agent with `delegate_task`, without naming a target. The child runs on the lane 1 instance, and `ccam sessions` lists it under lane 1. A Codex thread, if you have one, is unaffected.
6. **Move.** `ccam move 1 <free account>` (answer yes). It prints `Moved lane 1 from 'a' to '<account>'.`. The label becomes `Claude · lane 1 · <account>`. The open thread keeps its conversation, and its next reply goes out on the new account. The prompt cache is rebuilt once and not on later messages.
7. **Changing instance in T3.** Switch that thread to the default instance in the picker. Record what happens: expected is a fresh session that sees about 16k tokens of history, not the whole conversation. Switch it back.
8. **Config refresh.** Add a line to `~/.claude/CLAUDE.md`. Start a new thread on the lane 1 instance and ask Claude what the line says. Pass: it knows. The lane's own `CLAUDE.md` (`~/.claude/ccam/lanes/1/CLAUDE.md`) now has the line, and `switch.log` has `[lane 1] settings refreshed` for that session start. Start another thread without editing anything: the lane's files keep their modified times. Then break the main `settings.json` (make it a JSON array) and start a thread: `switch.log` has `[lane 1] settings not refreshed: ...`, the thread still runs, and the lane's files are unchanged. Restore the file.
9. **Renewals.** After a renewal in a lane thread, `switch.log` has `[lane 1] sync '<account>'`.
10. **Release and delete.** Close the lane's threads, then `ccam release 1`. It prints `Released '<account>' from lane 1. Any terminal can use it again.`. The instance is disabled (`enabled` is `false`, the label is `Claude · lane 1`) and gone from the picker. `ccam lane <account> --lane 1` brings it back. Then `ccam release 1 --delete` prints `Removed lane 1.` (and, for a second released lane, `ccam prune` prints `Removed empty lane <id>.`) and removes the `ccam-lane-*` entries. Other instances in `settings.json` are unchanged throughout.
11. **Instance ids.** `ccam lane a --lane work.2` creates an entry `ccam-lane-work_2_` followed by 8 hex characters, with the label `Claude · lane work.2 · a`, and T3 shows it. `ccam lane a --lane work-2` (after releasing `work.2`) creates `ccam-lane-work-2`, a different id. Release and delete both when done.
12. **No T3.** With `T3CODE_HOME` pointing at an empty folder, `ccam lane a` prints `Lane <id> holds 'a'.` and `T3 Code not found; skipped adding the lane to T3.`, still creates the lane, and exits 0.
13. **T3 file unusable.** Replace `settings.json` with `[1]`, then run `ccam lane a`. It prints `Lane <id> holds 'a'.` and, on stderr, `ccam: could not update T3 Code: <path>: top level is not a JSON object`, exits 0, and leaves the file byte for byte as it was. `switch.log` has `t3: <path>: top level is not a JSON object`. Restore your copy and run `ccam lane a` again: the instance is back. Repeat with `{not json` (the message ends in `invalid JSON: ...`).

Pass: each lane has exactly one instance whose label matches its account, no thread needs `/login`, a T3 problem never fails a lane command, and nothing outside `ccam-lane-*` in `settings.json` ever changes.

## Results

| Date | Claude Code | OS | Tests | Result | Notes |
|---|---|---|---|---|---|
| | | | | | |
