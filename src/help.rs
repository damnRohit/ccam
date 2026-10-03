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

Examples:
  ccam save work                   Save the live login as \"work\"
  (in Claude) /login, then:
  ccam save work2                  Add another account";

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
  ccam usage --json        Machine-readable output for scripts";

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
