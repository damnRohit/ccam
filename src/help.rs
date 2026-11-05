//! Example sections for `--help` / `ccam help <command>`. `-h` stays short and leaves them out.

pub const MAIN: &str = "\
Quick start:
  ccam save work          Save the account you are logged in to as \"work\"
  ccam list               See saved accounts (* marks the live one)
  ccam usage              How much of each account's limits is used
  ccam to work            Switch; a running Claude picks it up on its next message

Run `ccam help <command>` (or `ccam <command> --help`) for details and examples.
`-h` prints the short version.";

pub const LIST: &str = "\
Examples:
  ccam list
    * work      (pro)     <- the live login
      personal  (pro)  in use on WSL (Ubuntu)   <- another install sharing these accounts";

pub const SAVE: &str = "\
Names start with a letter or digit, then use letters, digits, '.', '_', '@' or '-' (max 64).
To add another account, use `ccam login <name>`: it signs in without replacing the live login.

Examples:
  ccam save work                   Save the live login as \"work\"
  ccam save work --lane 1          Save the login from lane 1";

pub const LOGIN: &str = "\
Give the name first. Claude Code's own sign-in then opens your browser; sign in to the account you want
saved under that name. If the browser does not open, open the link it prints and paste the code back here.
If your browser is already signed in to another Claude account, use a private window, or that account is
signed in instead. The main login, lanes and running sessions are not touched.

`ccam login <existing>` replaces that account's saved login with a new one, for when it stopped working.
This is refused while the account is in use (the main login, a lane, or your other side).

Examples:
  ccam login work2                 Add another account as \"work2\"
  ccam to work2                    Then switch to it
  ccam login work                  New saved login for \"work\" when its old one stopped working";

pub const RENAME: &str = "\
Examples:
  ccam rename second personal";

pub const REMOVE: &str = "\
Examples:
  ccam remove old-trial";

pub const TO: &str = "\
Inside Claude Code the switch happens in place: the running session sends its next message as the new
account, and the conversation stays where it is. From a plain terminal it only switches, unless you ask it
to open Claude as well.

Examples:
  ccam to second                    Switch only
  ccam to second --open             Switch, then open Claude here
  ccam to second --open 1f2e3d4c-...  Switch, then resume that conversation
  ccam to second --continue         Switch, then continue the most recent conversation here
  (in Claude) ! ccam to second --restart
                                    New Windows Terminal tab on the new account continues this chat,
                                    the old tab closes
  ccam to second --force            If Claude on your other side (Windows or WSL, sharing these accounts)
                                    uses 'second', first move that side to the free account with the most
                                    room, then switch here

An account another install is logged in as is refused without --force: both using one saved login would
log one of them out at its next renewal.";

pub const START: &str = "\
Sends one tiny message (about 270 tokens, Haiku) from every saved account whose 5-hour window is not
running yet, so all windows open, and later reset, together. Accounts with a running window are skipped.

Examples:
  ccam start
  ccam start --refresh     Also renew expired logins of accounts not in use, then start them";

pub const USAGE: &str = "\
Examples:
  ccam usage               Boxes with 5-hour and 7-day use and reset times
  ccam usage --refresh     Also renew expired logins of accounts not in use
  ccam usage --json        Machine-readable output for scripts
  ccam usage --email       Also show each account's email address (only shown here, never logged)
  ccam usage --info        Also show name, plan, billing, subscription status, trial, extra usage and guest
                           passes for each account, with its email (one or two extra requests per account;
                           shown only here, never logged)";

pub const SYNC: &str = "\
Claude Code renews its login every few hours and the old refresh token stops working. The hooks from
`ccam setup` copy the new login automatically; run this by hand if hooks are off.

Examples:
  ccam sync
  ccam sync --quiet        Print nothing when there is nothing to do";

pub const SETUP: &str = "\
Adds hooks to Claude Code's settings.json. Safe to rerun; your own hooks are left alone.
Undo with `ccam uninstall`.

Examples:
  ccam setup               Keep saved logins fresh
  ccam setup --auto        Also signal `ccam auto` when a reply ends or a limit is hit, so it reacts at once";

pub const AUTO: &str = "\
Runs until stopped. When the live account reaches the threshold (5-hour or weekly), it picks the saved
account with the most room left and switches after Claude finishes its current reply (at once if a limit
was hit, or after 10 minutes if no reply ends). It rings the terminal bell and shows a desktop alert with a
sound on every switch or warning. It waits 15 minutes between switches and warns instead of looping when
every account is nearly full.

Run it in its own terminal or tmux pane. Run `ccam setup --auto` once so it hears when replies end.
Automatic switching is the riskiest use under Anthropic's consumer terms; --notify only tells you.

Examples:
  ccam auto                          Switch by itself at 98%
  ccam auto --notify                 Only alert me which account to move to
  ccam auto --threshold 90           Act at 90%
  ccam auto --notify-url https://ntfy.sh/my-topic
                                     Also push alerts to the ntfy phone app
  ccam auto status                   Is it running, what is it waiting for
  ccam auto stop                     Stop it (within a few seconds)";

pub const NOTIFY_TEST: &str = "\
Examples:
  ccam notify-test                   Bell in this terminal and a desktop alert with sound
  ccam notify-test --notify-url https://ntfy.sh/my-topic";

pub const UNINSTALL: &str = "\
Saved accounts are kept. Examples:
  ccam uninstall";

pub const IMPORT_LEGACY: &str = "\
One-time move from the old PowerShell `cs` script. Accounts ccam already has are skipped.
Stop using the old script afterwards: two tools renewing the same logins break each other's copies.

Examples:
  ccam import-legacy";

pub const RUN: &str =
    "Opens Claude Code in this terminal inside a lane: a Claude config dir that holds one account. `ccam to` and
other terminals do not move a lane, so its sessions keep their account and their prompt cache. Lanes share
your conversations, memory, plugins, skills and settings with the main login, so /resume works across them.

`ccam run <account>` makes a new lane (numbered 1, 2, ...), or joins the lane that already holds the account.
`--lane <id>` joins that lane, or creates it under that number or name. An account is in one place at a time:
the main login or one lane, because two copies of one login would log each other out.
Needs Claude Code 2.1.296 or later. Every lane event is logged to switch.log, tagged [lane <id>].

Examples:
  ccam run work                      Claude Code on 'work' in a new lane (or work's lane)
  ccam run personal --continue       Continue this folder's latest chat on 'personal'
  ccam run work --lane api           A lane named 'api' on 'work'
  ccam run --lane 2                  Another terminal in lane 2, on whatever account it holds
  ccam run --lane 2 -- --continue    The same, continuing the latest chat";

pub const LANE: &str =
    "Checks a saved account out into a lane without opening Claude Code, for programs that run Claude Code
themselves, such as T3 Code. It picks the lane as `ccam run` does (the lane already holding the account, a
free lane, or --lane) and refuses the same cases.

With T3 Code installed, every lane is a Claude instance in T3's model picker, named after the account it
holds. `ccam run`, `move`, `release` and `prune` keep those instances up to date; a released lane's instance
is turned off, a deleted lane's is removed. Lane sessions pick up edits to your main settings.json, CLAUDE.md
and keybindings.json when they start.

Examples:
  ccam lane work                     'work' in a new lane (or work's lane)
  ccam lane work --lane api          A lane named 'api' on 'work'";

pub const MOVE: &str =
    "Switches a lane to another saved account. Every session in the lane uses the new account from its next
message and rebuilds its prompt cache once; other lanes and the main login are untouched. Asks first; --yes
skips the question (required when not run from a terminal).

Examples:
  ccam move 2 personal               Lane 2 to 'personal', after confirming
  ccam move api work --yes";

pub const RELEASE: &str =
    "Saves the lane's login back and logs the lane out, so the main login and other lanes can use the account.
Refused while sessions run in the lane (see ccam sessions); --force releases anyway and logs them out. --delete
also removes the lane folder.

Examples:
  ccam release 2
  ccam release 2 --delete
  ccam release api --force";

pub const PRUNE: &str = "\
Removes empty lane directories that hold no account and have no running Claude Code sessions.
Lanes with an unsaved or unreadable login are kept, never treated as empty.

Examples:
  ccam prune";

pub const SESSIONS: &str =
    "Lists the main login and every lane: the account each holds and its running Claude Code sessions (status,
folder, session id, pid), read with `claude agents --json`.
An unknown account is shown as 'unsaved login'; incomplete credentials as 'login unreadable'.

Examples:
  ccam sessions";
