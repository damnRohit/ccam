# ccam

Claude Code Account Manager. Keep several Claude subscription logins on one machine and switch Claude Code between them, without `/logout` and `/login` and without losing your conversation history.

```text
$ ccam list
* work      (pro)
  personal  (pro)

$ ccam to personal
Switched from 'work' to 'personal'.

$ ccam usage
╭─ ● personal  current ────────────────────────────── pro ─╮
│ 5 hour       █████░░░░░░░░░░░░░░░  27%  resets in 4h 4m  │
│ 7 day        █████████░░░░░░░░░░░  46%  resets in 5d 9h  │
╰──────────────────────────────────────────────────────────╯
```

> [!CAUTION]
> **Use at your own risk.** Read this before installing.
>
> - **This was vibe-coded.** Nearly all of it was written by an AI (Claude Code) under one person's direction. It has a test suite and was reviewed several times, but no human has read every line.
> - **It handles your Claude login tokens.** A bug here can log you out of an account. The worst case seen in practice is having to run `/login` again. It sends tokens only to Anthropic's own endpoints, but read the code before trusting that.
> - **It relies on undocumented Claude Code behaviour.** Where Claude Code keeps its login, how it renews it, and the usage endpoint are not public API. A Claude Code update can break ccam without warning.
> - **Tested by one person.** Daily use on Windows 11 and WSL (Ubuntu). Linux runs the same code as WSL. The macOS Keychain code is unit tested and type-checked in CI but has **never run on a real Mac**.
> - **Terms of service.** Whether rotating your own subscriptions is acceptable to Anthropic is unsettled, and `ccam auto` in particular is easy to read as against their Consumer Terms. See [Terms of service](#terms-of-service). The possible outcome is action against your accounts.
> - Not affiliated with or endorsed by Anthropic. No warranty (MIT license).

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Commands](#commands)
- [Windows and WSL together](#windows-and-wsl-together)
- [Starting all 5-hour windows at once](#starting-all-5-hour-windows-at-once)
- [Switching automatically before the limit](#switching-automatically-before-the-limit)
- [How it works](#how-it-works)
- [When something goes wrong](#when-something-goes-wrong)
- [Known limitations](#known-limitations)
- [Terms of service](#terms-of-service)
- [Extras: tmux helpers for WSL](#extras-tmux-helpers-for-wsl)
- [For AI agents](#for-ai-agents)
- [Uninstall](#uninstall)
- [Development](#development)

## Install

You need [Claude Code](https://code.claude.com) and [Rust](https://rustup.rs) 1.88 or newer.

```sh
git clone <this repository's URL> ccam
cd ccam
cargo install --locked --path .
ccam setup
```

`cargo install` puts one binary, `ccam`, in `~/.cargo/bin`. `ccam setup` adds three hooks to your Claude Code `settings.json` so saved logins stay current (see [How it works](#how-it-works)). `ccam uninstall` removes them.

Know the way back before you start: your login lives in `~/.claude/.credentials.json` (or the macOS Keychain) and `~/.claude.json`. ccam's own data is only `~/.claude/ccam/`. Whatever happens, `/login` in Claude Code always gives you a working login again.

## Quick start

1. Log in to Claude Code as your first account, then save it:
   ```sh
   ccam save work
   ```
2. Inside Claude Code run `/login` as the next account. Use `/login`, not `/logout`: logging out can revoke the login you just saved. Then:
   ```sh
   ccam save personal
   ```
3. Switch whenever you like:
   ```sh
   ccam to work
   ```

Inside a Claude Code session, prefix commands with `!` (for example `! ccam to personal`). They run locally, so they work even after the session has hit its usage limit. A running session uses the new account from its next message.

## Commands

`ccam help` lists them, and `ccam help <command>` (or `ccam <command> --help`) explains one with examples.

| Command | What it does |
|---|---|
| `ccam list` | Saved accounts. `*` marks the logged-in one; `in use on ...` marks one another install is using |
| `ccam save <name>` | Save the logged-in account |
| `ccam rename <old> <new>` | Rename a saved account |
| `ccam remove <name>` | Forget a saved account (the live login is not touched) |
| `ccam to <name>` | Switch Claude Code to a saved account |
| `ccam to <name> --open` | Switch, then open Claude Code here (from a normal terminal) |
| `ccam to <name> --open <session-id>` | Switch, then resume that conversation |
| `ccam to <name> --continue` | Switch, then continue the latest conversation |
| `ccam to <name> --restart` | Switch and reopen the conversation in a new Windows Terminal tab (from inside Claude Code) |
| `ccam to <name> --force` | If your other install (Windows or WSL) uses the account, move that install to the best free account first |
| `ccam usage [--refresh] [--json]` | 5-hour, weekly and per-model usage for every account |
| `ccam start [--refresh]` | Start the 5-hour window on every account with one tiny message each |
| `ccam sync` | Save the newest login of the logged-in account (the hooks normally do this) |
| `ccam setup [--auto]` / `ccam uninstall` | Add or remove the hooks |
| `ccam auto [--notify] [--threshold 98]` | Watch usage and move to another account before the limit |
| `ccam auto status` / `ccam auto stop` | See what it is doing, or stop it |
| `ccam notify-test` | Check that terminal and desktop notifications reach you |

Names start with a letter or digit and may contain letters, digits, `.`, `_`, `@` and `-` (up to 64 characters). `CLAUDE_CONFIG_DIR` is respected.

Exit codes: `0` success, `1` the request cannot be done as asked, `2` a file, lock or credential store failed, `3` Anthropic's endpoints failed or a login could not be confirmed.

## Windows and WSL together

Claude Code on Windows and Claude Code in WSL are two installs with two separate logins. If each keeps its own copy of your saved accounts, the copies go stale: a login's refresh token works once, so when one side renews an account, the other side's copy of it is dead.

The fix is one shared set of saved accounts, plus a rule that each account is live on only one side at a time. ccam enforces the rule.

Set it up once, in WSL, after installing ccam on both sides:

```sh
mv ~/.claude/ccam ~/ccam-wsl-backup 2>/dev/null      # keep WSL's own copy, just in case
ln -s "/mnt/c/Users/<you>/.claude/ccam" ~/.claude/ccam
ccam list
```

From then on:

- Each side records which account it is logged in as. `ccam list` shows `in use on WSL (Ubuntu)` or `in use on Windows`.
- `ccam to X` refuses when the other side is using X.
- `ccam to X --force` moves the other side to the free account with the most room, then switches here.
- `ccam auto`, `usage --refresh` and `start --refresh` leave the other side's account alone.

The full guide, including what breaks and how to fix it, is [docs/WINDOWS-WSL.md](docs/WINDOWS-WSL.md).

## Starting all 5-hour windows at once

Subscription usage runs in rolling 5-hour windows that begin with the first message. `ccam start` sends one tiny message per saved account, so all the windows begin (and later reset) together. It does not switch your login.

Each message runs through the official `claude` binary in a throwaway config directory with that account's token, stripped of Claude Code's context: about 270 input tokens on Haiku, against roughly 4,500 for a normal "hi". Accounts whose window is already running are skipped, nothing is saved to your history, and your real `~/.claude` is not touched. You run it yourself; there is no scheduler.

## Switching automatically before the limit

```sh
ccam setup --auto      # once: lets ccam hear when a Claude turn ends and when a limit is hit
ccam auto              # in its own terminal or tmux pane; switches by itself at 98%
ccam auto --notify     # or: only tell me which account to switch to
ccam auto status
ccam auto stop
```

`ccam auto` watches the logged-in account. When its 5-hour or weekly usage reaches the threshold, it picks the saved account with the most room, waits for the current Claude turn to finish, and switches. A request that hits the limit switches at once; after that, send any message in Claude to continue. Each switch rings the terminal bell and shows a desktop notification with a sound (a Windows toast, also from WSL). `--notify-url` also posts to an [ntfy](https://ntfy.sh) topic for phone alerts.

It is built to run for days. With Claude idle it checks usage once every 30 minutes, more often only as usage nears the threshold, reads the other accounts only once the live one is close, and backs off for 15 minutes or more on a rate limit. Switches are at least 15 minutes apart. If every account is nearly full, it says which one reopens first and waits.

Read [Terms of service](#terms-of-service) before using this. `--notify` keeps you deciding each switch.

## How it works

Claude Code stores the login in `~/.claude/.credentials.json` (Windows, Linux) or the macOS Keychain, and the account identity in `~/.claude.json`. ccam keeps a copy of both per account in `~/.claude/ccam/accounts/<name>/` and swaps them in.

The hard part is that **refresh tokens are single-use**. Every time Claude Code renews a login it gets a new refresh token and the old one stops working, so a saved copy goes stale as soon as the account is used. Switching back to a stale copy forces a new `/login`. ccam prevents that:

- Before switching away from an account, it saves that account's current login.
- It never replaces a saved login with an older one, and refuses to switch when the saved copy is somehow newer than the live one.
- Hooks save the newest login whenever the credentials file changes (`/login`, token renewal) and after a login on macOS. They never run per message.
- When a brand-new login appears, ccam confirms with Anthropic which account the token belongs to before saving it, because `/login` updates the two files at slightly different moments.
- It takes the same lock files Claude Code takes around token renewal, never calls the network while holding them, and notices when a lock was taken over while the machine slept.
- Every write is atomic, and nothing outside `oauthAccount` and `claudeAiOauth` is touched. MCP logins stay shared across accounts.

The full design is in [docs/SPEC.md](docs/SPEC.md), and the research behind it in [docs/research](docs/research).

## When something goes wrong

[docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) lists every error message, what it means and how to fix it. The most common:

| You see | What to do |
|---|---|
| `stale login, run: ccam usage --refresh` | That account's short-lived token expired. Run `ccam usage --refresh` |
| `login expired, run /login in Claude Code and save the account again` | The saved login is dead. `/login` as that account, then `ccam save <name>` |
| `'X' is in use by Claude on ...` | Your other install is on that account. Switch it, or `ccam to X --force` |
| `the logged-in account is not saved` | `ccam save <name>` first, so switching away does not lose it |
| A running session did not pick up the switch | Restart it: `claude --continue` |
| `Input/output error` on `/mnt/c/...` in WSL | WSL lost the Windows drive; see [docs/WINDOWS-WSL.md](docs/WINDOWS-WSL.md) |

The universal fix always works: `/login` in Claude Code as the account you want, then `ccam save <name>` to store the fresh login.

## Known limitations

- **macOS has never run for real.** The Keychain code follows Claude Code's own scheme and is unit tested, but nobody has used ccam on a Mac. Expect to be the first.
- **All open sessions switch together.** Claude Code has one login per config directory, so every open session moves to the new account, each from its next request.
- **`--restart` is Windows Terminal only.** Elsewhere ccam prints `claude --continue` for you to run.
- **A second computer.** ccam knows about installs that share its saved accounts (Windows and WSL on one PC). It cannot know about another machine logged in to the same account; renewing or switching to that account here logs it out there.
- **Clock drift between Windows and WSL.** The lock around the shared accounts trusts both clocks. If WSL's clock drifts by more than a minute after sleep, one side could take over the other's lock. Not seen in practice.
- **`ccam start` is sequential**, up to about 30 seconds per account.
- **The usage endpoint is undocumented and rate limited.** `rate limited, try again in a minute` is normal when you ask often.

## Terms of service

Anthropic's Consumer Terms prohibit accessing the service "through automated or non-human means, whether through a bot, script, or otherwise" unless explicitly permitted, and "bypassing any of our systems or protective measures". Claude Code's legal page says usage limits "assume ordinary, individual usage".

Anthropic has not published a position on one person rotating their own subscriptions. Switching by hand is the mildest use. `ccam auto`, which keeps you working past each account's limit without you doing anything, is the use those clauses are most easily read against, and the pattern is visible to Anthropic. `ccam auto --notify` only tells you; you decide.

How you use multiple subscriptions is your responsibility.

## Extras: tmux helpers for WSL

`scripts/` holds optional shell tools for running Claude Code in WSL, including from a phone over SSH: persistent tmux sessions per project (`tc`, `td`), a session picker (`tm`), a resume picker that also sees Windows conversations (`cresume`), and a one-page command list (`cmds`). ccam does not need them. See [scripts/README.md](scripts/README.md).

## For AI agents

- [AGENTS.md](AGENTS.md) is for an agent **changing this code**: layout, the invariants that must hold, and how to verify.
- [skills/ccam/SKILL.md](skills/ccam/SKILL.md) is for an agent **operating ccam for you**. Copy the `skills/ccam` folder into `~/.claude/skills/` and Claude Code can check usage, switch accounts and diagnose errors when you ask.

## Uninstall

```sh
ccam uninstall                 # removes the hooks from settings.json
cargo uninstall ccam
rm -rf ~/.claude/ccam          # the saved accounts; your live login stays
```

With Windows and WSL sharing accounts, remove the link in WSL first: `rm ~/.claude/ccam` deletes only the link.

## Development

```sh
cargo test                                   # unit and end-to-end tests; no network, never your real ~/.claude
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Type-check the other platforms without a C cross-compiler:

```sh
cargo clippy --target x86_64-apple-darwin --all-targets --no-default-features -- -D warnings
```

After a Claude Code release that mentions authentication, credentials, hooks or sessions, run the checks in [docs/MANUAL-TESTS.md](docs/MANUAL-TESTS.md).

Bug reports are welcome. Never paste tokens, the contents of `.credentials.json`, or email addresses into an issue. `~/.claude/ccam/accounts/switch.log` is safe to share: it holds the account names you chose and no secrets.

## License

MIT
