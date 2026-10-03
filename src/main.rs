use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::Value;

use ccam::api::{Api, Endpoints};
use ccam::error::{Error, Result, exit};
use ccam::hooks::{self, Event};
use ccam::manager::{LiveState, Manager, Network, SyncOutcome};
use ccam::name::AccountName;
use ccam::paths::Paths;
use ccam::{store, usage};

mod help;

#[derive(Parser)]
#[command(
    name = "ccam",
    bin_name = "ccam",
    version,
    about = "Keep several Claude subscription logins and switch Claude Code between them",
    after_long_help = help::MAIN
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List saved accounts and mark the logged-in one
    #[command(after_long_help = help::LIST)]
    List,
    /// Save the logged-in account under a name
    #[command(after_long_help = help::SAVE)]
    Save {
        /// Name to save it under, e.g. work or me@home
        name: AccountName,
    },
    /// Rename a saved account
    #[command(after_long_help = help::RENAME)]
    Rename {
        /// Current name
        from: AccountName,
        /// New name
        to: AccountName,
    },
    /// Forget a saved account (the live login is not touched)
    #[command(after_long_help = help::REMOVE)]
    Remove {
        /// Saved account to forget
        name: AccountName,
    },
    /// Switch Claude Code to a saved account
    #[command(after_long_help = help::TO)]
    To {
        /// Saved account to switch to (see `ccam list`)
        name: AccountName,
        /// Reopen the conversation in a new Windows Terminal tab instead of switching the running session
        #[arg(long, conflicts_with = "open")]
        restart: bool,
        /// After switching, open Claude Code here: plain with no value, or resuming the given session id
        #[arg(long, value_name = "SESSION", num_args = 0..=1, default_missing_value = "")]
        open: Option<String>,
        /// After switching, open Claude Code here continuing the most recent conversation
        #[arg(long = "continue", conflicts_with_all = ["open", "restart"])]
        continue_: bool,
        /// If Claude on your other side (Windows or WSL) uses this account, first move that side to the best
        /// free account
        #[arg(long)]
        force: bool,
    },
    /// Start the 5-hour usage window on every saved account with one minimal message each
    #[command(after_long_help = help::START)]
    Start {
        /// Renew expired logins of accounts that are not in use, so they can be started too
        #[arg(long)]
        refresh: bool,
    },
    /// Show 5-hour, weekly and per-model usage for every saved account
    #[command(after_long_help = help::USAGE)]
    Usage {
        /// Renew expired logins of accounts that are not in use, to read their usage
        #[arg(long)]
        refresh: bool,
        /// Print JSON instead of boxes
        #[arg(long)]
        json: bool,
    },
    /// Copy the live login into its saved account when it is newer
    #[command(after_long_help = help::SYNC)]
    Sync {
        /// Print nothing when there is nothing to do
        #[arg(long)]
        quiet: bool,
    },
    /// Install the Claude Code hooks that keep saved logins fresh
    #[command(after_long_help = help::SETUP)]
    Setup {
        /// Also install the turn-end and limit-hit signals that `ccam auto` listens to
        #[arg(long)]
        auto: bool,
    },
    /// Watch usage and move to another account before the limit (run it in tmux; stop with: ccam auto stop)
    #[command(after_long_help = help::AUTO)]
    Auto {
        #[command(subcommand)]
        action: Option<AutoAction>,
        /// Only notify which account to switch to; never switch by itself
        #[arg(long)]
        notify: bool,
        /// Act when the 5-hour or weekly usage of the live account reaches this percentage
        #[arg(long, default_value_t = 98.0, value_parser = parse_threshold)]
        threshold: f64,
        /// Also send notifications to this URL (an ntfy topic, for phone push while SSH is closed)
        #[arg(long)]
        notify_url: Option<String>,
        /// Do not show desktop notifications
        #[arg(long)]
        no_desktop: bool,
    },
    /// Send a test notification to the terminal and desktop (and the URL, if given)
    #[command(after_long_help = help::NOTIFY_TEST)]
    NotifyTest {
        /// Also send the test to this URL (an ntfy topic)
        #[arg(long)]
        notify_url: Option<String>,
    },
    /// Remove ccam's Claude Code hooks
    #[command(after_long_help = help::UNINSTALL)]
    Uninstall,
    /// Convert accounts saved by the PowerShell prototype (cs) into ccam accounts
    #[command(hide = true, after_long_help = help::IMPORT_LEGACY)]
    ImportLegacy,
    #[command(hide = true)]
    Hook {
        event: Event,
        /// Claude Code's config dir, passed explicitly because hook environments may not carry it
        #[arg(long)]
        config_dir: Option<String>,
    },
    #[command(hide = true)]
    Resume {
        name: AccountName,
        #[arg(long)]
        claude_pid: u32,
        #[arg(long)]
        ready_file: PathBuf,
        #[arg(long)]
        claude_config_dir: String,
    },
}

#[derive(Subcommand)]
enum AutoAction {
    /// Stop a running ccam auto
    Stop,
    /// Show what ccam auto is doing
    Status,
}

fn parse_threshold(s: &str) -> std::result::Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("'{s}' is not a number"))?;
    if (50.0..=100.0).contains(&v) { Ok(v) } else { Err("threshold must be between 50 and 100".into()) }
}

fn notifiers(notify_url: Option<String>, desktop: bool) -> ccam::notify::Fanout {
    use ccam::notify::{Desktop, Fanout, Notifier, Terminal, Webhook};
    let mut all: Vec<Box<dyn Notifier>> = vec![Box::new(Terminal::new(std::io::stdout()))];
    if desktop {
        all.push(Box::new(Desktop));
    }
    if let Some(url) = notify_url {
        all.push(Box::new(Webhook::new(url)));
    }
    Fanout(all)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            // Help and version are not errors; everything else is a usage mistake.
            return if e.use_stderr() { ExitCode::from(exit::USAGE) } else { ExitCode::SUCCESS };
        }
    };
    if let Command::Hook { event, config_dir } = cli.command {
        run_hook(event, config_dir);
        return ExitCode::SUCCESS;
    }
    match run(cli.command) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ccam: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

fn manager_for(paths: Paths) -> Result<Manager> {
    let store = store::for_platform(&paths)?;
    let api = Api::new(Endpoints::from_env(), Duration::from_secs(10));
    Ok(Manager::new(paths, store, Box::new(api)))
}

fn run(command: Command) -> Result<ExitCode> {
    #[cfg(windows)]
    if let Command::Resume { name, claude_pid, ready_file, claude_config_dir } = command {
        let manager = manager_for(Paths::for_config_dir(Some(claude_config_dir))?)?;
        return match ccam::restart::resume(&manager, &name, claude_pid, &ready_file) {
            Ok(code) => Ok(ExitCode::from(u8::try_from(code).unwrap_or(1))),
            Err(e) => {
                eprintln!("ccam: {e}");
                eprintln!("Press Enter to close this tab.");
                let _ = std::io::stdin().read(&mut [0u8]);
                Err(e)
            }
        };
    }

    let manager = manager_for(Paths::from_env()?)?;
    // Hooks can be disabled or skipped (`--bare`), so read-only commands also refresh the saved copy of the live
    // login. Offline and best-effort: it must not slow down or block the command the user asked for.
    if matches!(command, Command::List | Command::Usage { .. } | Command::Setup { .. } | Command::Uninstall) {
        let _ = manager.sync(Network::Offline);
    }

    match command {
        Command::List => list(&manager)?,
        Command::Save { name } => {
            let saved = manager.save(&name)?;
            println!("Saved the logged-in account as '{}'.", saved.name);
            if let Some(warning) = saved.warning {
                println!("Note: {warning}. If this is the wrong account, run: ccam remove {}", saved.name);
            }
        }
        Command::Rename { from, to } => {
            manager.rename(&from, &to)?;
            println!("Renamed '{from}' to '{to}'.");
        }
        Command::Remove { name } => {
            // Found before removing: afterwards the account is no longer in the listing to match against.
            let overview = manager.overview()?;
            let removed = overview.listing.get(&name).map(|a| a.identity.clone());
            let live_here = overview.live_account().is_some_and(|a| Some(&a.identity) == removed.as_ref());
            let elsewhere = removed.as_ref().and_then(|identity| manager.used_elsewhere(identity));
            manager.remove(&name)?;
            println!("Removed '{name}'. The live login is unchanged.");
            if live_here {
                println!("Claude Code is still logged in as it. To switch away later, save it again: ccam save {name}");
            }
            if let Some(mark) = elsewhere {
                let side = mark.side.label();
                println!("Claude on {side} is still logged in as it and cannot switch away until it is saved there.");
            }
        }
        Command::To { name, restart, open, continue_, force } => {
            if force {
                free_elsewhere(&manager, &name)?;
            }
            let then = match (open, continue_) {
                (Some(session), _) if !session.is_empty() => Then::Resume(session),
                (Some(_), _) => Then::OpenPlain,
                (None, true) => Then::Continue,
                (None, false) => Then::Nothing,
            };
            switch(&manager, &name, restart, then)?
        }
        Command::Start { refresh } => {
            println!("Sending one minimal message per account that has no running window...");
            let sender = ccam::warmup::ClaudeSender::new();
            for report in ccam::warmup::start(&manager, &sender, refresh, ccam::warmup::SETTLE)? {
                println!("{:<24} {}", report.name, ccam::warmup::describe(&report.outcome));
            }
        }
        Command::Usage { refresh, json } => show_usage(&manager, refresh, json)?,
        Command::Sync { quiet } => {
            let outcome = manager.sync(Network::Allowed)?;
            if !quiet {
                println!("{}", describe_sync(&outcome));
            }
        }
        Command::Setup { auto } => {
            let exe = std::env::current_exe().map_err(|e| Error::Restart(format!("cannot locate ccam: {e}")))?;
            let paths = manager.paths();
            let changed = hooks::install(&paths.settings_file, &exe, paths.config_dir_env.as_deref(), auto)?;
            let verb = if changed { "Installed hooks in" } else { "Hooks already installed in" };
            println!("{verb} {}", paths.settings_file.display());
            if auto {
                println!("ccam auto will now hear when a Claude turn ends and when a limit is hit.");
            }
        }
        Command::Auto { action: Some(AutoAction::Stop), .. } => {
            if ccam::autorun::stop(&manager)? {
                println!("ccam auto stopped.");
            } else {
                println!("ccam auto is not running.");
            }
        }
        Command::Auto { action: Some(AutoAction::Status), .. } => match ccam::autorun::status(&manager)? {
            None => println!("ccam auto is not running. Start it with: ccam auto"),
            Some(status) => println!("{}", serde_json::to_string_pretty(&status).expect("status always serializes")),
        },
        Command::Auto { action: None, notify, threshold, notify_url, no_desktop } => {
            if std::env::var_os("CLAUDE_PID").is_some() {
                return Err(Error::Restart(
                    "run ccam auto in its own terminal or tmux pane, not inside Claude Code".into(),
                ));
            }
            let mode = if notify { ccam::auto::Mode::Notify } else { ccam::auto::Mode::Auto };
            let notifier = notifiers(notify_url, !no_desktop);
            let echo = ccam::notify::Terminal::new(std::io::stdout());
            ccam::autorun::run(&manager, &notifier, ccam::auto::Settings::new(mode, threshold), &echo)?;
        }
        Command::NotifyTest { notify_url } => {
            use ccam::notify::Notifier;
            notifiers(notify_url, true).notify("ccam test", "If you can see and hear this, notifications work.");
            println!("Sent. A desktop popup can take a few seconds to appear.");
        }
        Command::Uninstall => {
            let changed = hooks::uninstall(&manager.paths().settings_file)?;
            println!("{}", if changed { "Removed ccam's hooks." } else { "No ccam hooks were installed." });
        }
        Command::ImportLegacy => {
            let results = manager.import_legacy()?;
            if results.is_empty() {
                println!("Nothing to import.");
            }
            for (entry, result) in results {
                match result {
                    Ok(name) => println!("Imported '{name}'."),
                    Err(reason) => println!("Skipped '{entry}': {reason}"),
                }
            }
            println!(
                "Do not use cs after importing: both tools spending the same refresh tokens would leave each other's copies dead."
            );
        }
        Command::Hook { .. } | Command::Resume { .. } => unreachable!("dispatched before the manager is built"),
    }
    Ok(ExitCode::SUCCESS)
}

fn list(manager: &Manager) -> Result<()> {
    let overview = manager.overview()?;
    let live = overview.live_account().map(|a| a.name.clone());
    if overview.listing.accounts.is_empty() {
        println!("No saved accounts. Log in to Claude Code, then run: ccam save <name>");
    }
    let marks = manager.marks();
    for account in &overview.listing.accounts {
        let marker = if Some(&account.name) == live.as_ref() { "*" } else { " " };
        let plan = account.login.subscription_type().map(|p| format!("  ({p})")).unwrap_or_default();
        let elsewhere = marks
            .iter()
            .find(|m| m.identity == account.identity)
            .map(|m| format!("  in use on {}", m.side.label()))
            .unwrap_or_default();
        println!("{marker} {}{plan}{elsewhere}", account.name);
    }
    for problem in &overview.listing.problems {
        println!("! {}: {}", problem.entry, problem.reason);
    }
    match overview.live {
        LiveState::Present(_) if live.is_none() => {
            println!("\nThe logged-in account is not saved. Run: ccam save <name>");
        }
        LiveState::Unreadable => {
            println!("\nClaude Code's current login could not be read. Run /login in Claude Code.")
        }
        _ => {}
    }
    Ok(())
}

/// When another install uses `name`, switches that install to the free account with the most room, by running its
/// own ccam, so this side can take `name` without the two spending one refresh token.
fn free_elsewhere(manager: &Manager, name: &AccountName) -> Result<()> {
    let overview = manager.overview()?;
    let target = overview.listing.get(name).ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
    let marks = manager.marks();
    let Some(holder) = marks.iter().find(|m| m.identity == target.identity) else { return Ok(()) };
    let side = holder.side.label();
    manager.check_switch_here(name)?;
    let gone = format!("If that install no longer exists, delete {} and retry.", holder.file.display());

    let mut busy: Vec<&ccam::login::Identity> = marks.iter().map(|m| &m.identity).collect();
    busy.push(&target.identity);
    if let LiveState::Present(live) = &overview.live {
        busy.push(&live.identity);
    }
    let free: Vec<&AccountName> =
        overview.listing.accounts.iter().filter(|a| !busy.contains(&&a.identity)).map(|a| &a.name).collect();
    let replacement = best_of(manager, &free)
        .ok_or_else(|| Error::Restart(format!("no free account to move {side} to; switch it by hand")))?;

    let mut command = holder
        .side
        .command_from(manager.side())
        .ok_or_else(|| Error::Restart(format!("cannot run ccam on {side} from here; switch it there. {gone}")))?;
    println!("'{name}' is in use on {side}. Moving {side} to '{replacement}' first...");
    let status = command
        .args(["to", replacement.as_str()])
        .status()
        .map_err(|e| Error::Restart(format!("cannot run ccam on {side}: {e}. {gone}")))?;
    if !status.success() {
        return Err(Error::Restart(format!(
            "switching {side} to '{replacement}' failed; nothing changed here. {gone}"
        )));
    }
    Ok(())
}

/// The account with the most room in both windows. Nearly full accounts are passed over; if no usage can be read,
/// the first free account is as good a guess as any.
fn best_of(manager: &Manager, free: &[&AccountName]) -> Option<AccountName> {
    const NEARLY_FULL: f64 = 95.0;
    if let [only] = free {
        return Some((*only).clone());
    }
    let cards = usage::collect(manager, false).unwrap_or_default();
    let level = |name: &AccountName| {
        cards.iter().find(|c| &c.name == name).and_then(|c| match &c.state {
            usage::CardState::Ok { windows } => {
                windows.iter().filter(|w| w.label == "5 hour" || w.label == "7 day").map(|w| w.percent).reduce(f64::max)
            }
            _ => None,
        })
    };
    let readable = free.iter().filter_map(|n| level(n).map(|l| (*n, l))).filter(|(_, l)| *l < NEARLY_FULL);
    readable.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(n, _)| n.clone()).or_else(|| free.first().map(|n| (*n).clone()))
}

/// What to do in this terminal after the switch.
enum Then {
    Nothing,
    OpenPlain,
    Continue,
    Resume(String),
}

fn switch(manager: &Manager, name: &AccountName, restart: bool, then: Then) -> Result<()> {
    #[cfg(windows)]
    if restart && ccam::restart::available() {
        println!("Opening a new tab on '{name}'. This session closes in about 5 seconds.");
        return ccam::restart::launch(manager, name);
    }

    let switched = manager.switch_to(name)?;
    let from = switched.from.map_or_else(|| "logged out".to_string(), |f| format!("'{f}'"));
    println!("Switched from {from} to '{}'.", switched.to);

    let claude_args: Vec<String> = match then {
        Then::Nothing => {
            if restart {
                println!("Restart Claude Code to use it: claude --continue");
            } else if std::env::var_os("CLAUDE_PID").is_none() {
                println!("Open it with: claude, claude --continue, or: ccam to {name} --open");
            } else if cfg!(target_os = "macos") {
                println!("Running sessions pick up the new login within about 30 seconds. If not: claude --continue");
            } else {
                println!("Running sessions pick up the new login on their next request. If not: claude --continue");
            }
            return Ok(());
        }
        Then::OpenPlain => vec![],
        Then::Continue => vec!["--continue".into()],
        Then::Resume(session) => vec!["--resume".into(), session],
    };
    if std::env::var_os("CLAUDE_PID").is_some() {
        return Err(Error::Restart("--open runs from a normal terminal, not inside Claude Code".into()));
    }
    let status = ccam::launch::claude(&claude_args).map_err(|e| Error::Restart(format!("cannot start claude: {e}")))?;
    std::process::exit(status);
}

fn show_usage(manager: &Manager, refresh: bool, as_json: bool) -> Result<()> {
    let cards = usage::collect(manager, refresh)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&cards).expect("cards always serialize"));
        return Ok(());
    }
    if cards.is_empty() {
        println!("No saved accounts. Log in to Claude Code, then run: ccam save <name>");
        return Ok(());
    }
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    print!("{}", usage::render(&cards, jiff::Timestamp::now(), color));
    Ok(())
}

fn describe_sync(outcome: &SyncOutcome) -> String {
    match outcome {
        SyncOutcome::NotLoggedIn => "Not logged in to Claude Code.".into(),
        SyncOutcome::LiveUnreadable => "Claude Code's current login could not be read; nothing saved.".into(),
        SyncOutcome::NotSaved => "The logged-in account is not saved. Run: ccam save <name>".into(),
        SyncOutcome::Unchanged(n) => format!("'{n}' is up to date."),
        SyncOutcome::Updated(n) => format!("Saved the newer login for '{n}'."),
        SyncOutcome::SavedIsNewer(n) => format!("'{n}' already holds a newer login; left unchanged."),
        SyncOutcome::Unverified { name, reason } => {
            format!("A new login appeared but was not confirmed to belong to '{name}' ({reason}); left unchanged.")
        }
    }
}

/// Hooks must never break a Claude Code session: failures go to the switch log and the hook exits 0.
fn run_hook(event: Event, config_dir: Option<String>) {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let input: Value = serde_json::from_str(input.trim_start_matches('\u{feff}')).unwrap_or(Value::Null);

    let paths = match config_dir {
        Some(dir) => Paths::for_config_dir(Some(dir)),
        None => Paths::from_env(),
    };
    let Ok(manager) = paths.and_then(manager_for) else { return };
    match hooks::handle(event, &input, &manager) {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(e) => manager.log(&format!("hook {event:?} failed: {e}")),
    }
}
