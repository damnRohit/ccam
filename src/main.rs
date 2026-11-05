use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::Value;

use ccam::api::{Api, Endpoints};
use ccam::error::{Error, Result, exit};
use ccam::hooks::{self, Event};
use ccam::lane::{self, LaneId};
use ccam::lock::{self, LockTiming};
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
        /// Lane whose login to save, when saving from outside the lane
        #[arg(long)]
        lane: Option<LaneId>,
    },
    /// Add an account: sign in to it in your browser and save it under <name>, without touching the main login
    #[command(after_long_help = help::LOGIN)]
    Login {
        /// Name to save the new account under, e.g. work2
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
        /// Also show each account's email address (shown only here, never logged)
        #[arg(long)]
        email: bool,
        /// Also show plan, billing, status, trial, extra usage and guest passes for each account (one or two extra requests per account; never logged)
        #[arg(long)]
        info: bool,
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
    /// Open Claude Code in a lane: a config dir that keeps its own account while other terminals switch
    #[command(after_long_help = help::RUN)]
    Run {
        /// Saved account for the lane; joins the lane that already holds it
        #[arg(required_unless_present = "lane")]
        name: Option<AccountName>,
        /// Lane to join, or to create under this name or number
        #[arg(long)]
        lane: Option<LaneId>,
        /// Arguments for claude, e.g. --continue
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Check a saved account out into a lane without opening Claude Code, e.g. for T3 Code
    #[command(after_long_help = help::LANE)]
    Lane {
        /// Saved account for the lane; reuses the lane that already holds it
        name: AccountName,
        /// Lane to use, or to create under this name or number
        #[arg(long)]
        lane: Option<LaneId>,
    },
    /// Move a lane, and every session in it, to another saved account
    #[command(after_long_help = help::MOVE)]
    Move {
        /// Lane number or name (see `ccam sessions`)
        lane: LaneId,
        /// Saved account to move it to
        name: AccountName,
        /// Do not ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Give a lane's account back so any terminal can use it
    #[command(after_long_help = help::RELEASE)]
    Release {
        /// Lane number or name (see `ccam sessions`)
        lane: LaneId,
        /// Release even while sessions run in the lane; they are logged out
        #[arg(long)]
        force: bool,
        /// Also delete the lane folder after releasing its account
        #[arg(long)]
        delete: bool,
    },
    /// Remove empty lanes that hold no account and have no running sessions
    #[command(after_long_help = help::PRUNE)]
    Prune,
    /// List running Claude Code sessions per lane and the account each lane holds
    #[command(after_long_help = help::SESSIONS)]
    Sessions,
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
        Command::Save { name, lane } => {
            let target_manager = match lane {
                Some(id) => manager_for(lane::paths(manager.paths(), &id)?)?,
                None => manager,
            };
            let saved = target_manager.save(&name)?;
            println!("Saved the logged-in account as '{}'.", saved.name);
            if let Some(warning) = saved.warning {
                println!("Note: {warning}. If this is the wrong account, run: ccam remove {}", saved.name);
            }
        }
        Command::Login { name } => login(&manager, &name)?,
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
        Command::Usage { refresh, json, email, info } => show_usage(&manager, refresh, json, email, info)?,
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
        Command::Run { name, lane, args } => run_lane(&manager, name, lane, &args)?,
        Command::Lane { name, lane } => {
            let (id, _, account) = enter_lane(&manager, Some(name), lane)?;
            println!("Lane {id} holds '{account}'.");
            match sync_t3(&manager) {
                T3::NotInstalled => println!("T3 Code not found; skipped adding the lane to T3."),
                T3::Synced => println!(
                    "T3 Code: lane {id} is in the model picker as '{}'.",
                    ccam::t3::display_name(&id, Some(&account))
                ),
                T3::Failed => {}
            }
        }
        Command::Move { lane, name, yes } => reconciled(&manager, move_lane(&manager, &lane, &name, yes))?,
        Command::Release { lane, force, delete } => reconciled(&manager, release_lane(&manager, &lane, force, delete))?,
        Command::Prune => reconciled(&manager, prune_lanes(&manager))?,
        Command::Sessions => print_sessions(&manager)?,
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

/// Signs in with Claude Code's own `auth login` in a throwaway config dir and saves the result under `name`.
///
/// The dir is deleted afterwards, never logged out: logout revokes the refresh token the vault now holds, and any
/// other `claude` command on it could spend that token on a renewal. It gets no presence record and no vault link.
fn login(manager: &Manager, name: &AccountName) -> Result<()> {
    let ccam_dir = manager.paths().config_home.join("ccam");
    // Held until the sign-in dir is deleted, so one login's sweep never deletes another's dir. Outside `login/`,
    // which the sweep empties. A sign-in can take minutes, so a second one fails fast instead of waiting.
    let timing = LockTiming { wait: Duration::from_secs(1), ..LockTiming::VAULT };
    let _signing_in = lock::acquire(&ccam_dir.join("login.lock"), timing).map_err(|e| match e {
        Error::LockTimeout { .. } => Error::Login("another ccam login is signing in; finish or cancel it first".into()),
        other => other,
    })?;
    manager.check_login_free(name)?;
    let root = ccam_dir.join("login");
    // A crashed run leaves a dir holding live tokens, and maybe Claude Code's `<dir>.lock` beside it.
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = root.join(format!("{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| Error::Login(format!("cannot create {}: {e}", dir.display())))?;

    let adopted = sign_in(manager, name, &dir);
    for path in [dir.clone(), PathBuf::from(format!("{}.lock", dir.display()))] {
        if path.exists()
            && let Err(e) = std::fs::remove_dir_all(&path)
        {
            eprintln!("ccam: could not delete {} ({e}); it holds a login, delete it by hand", path.display());
        }
    }
    let adopted = adopted?;

    if adopted.refreshed {
        println!("Refreshed the saved login of '{}'.", adopted.name);
    } else {
        println!("Saved '{0}'. Switch to it with: ccam to {0}", adopted.name);
    }
    Ok(())
}

fn sign_in(manager: &Manager, name: &AccountName, dir: &Path) -> Result<ccam::manager::Adopted> {
    println!("Signing in for '{name}'. Your browser opens; sign in to the account you want saved as '{name}'.");
    let status = ccam::launch::command(Some(dir))
        .args(["auth", "login"])
        .current_dir(dir)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .status()
        .map_err(|e| Error::Login(format!("cannot start claude: {e}; nothing was saved")))?;
    if !status.success() {
        let how = status.code().map_or_else(|| "claude was stopped".to_string(), |c| format!("claude exited with {c}"));
        return Err(Error::Login(format!("the sign-in did not finish ({how}); nothing was saved")));
    }
    let fresh = manager_for(Paths::for_config_dir(Some(dir.to_string_lossy().into_owned()))?)?;
    let adopted = manager.adopt(name, &fresh);
    // The Keychain item is keyed by the dir's name, so deleting the dir would leave it behind.
    #[cfg(target_os = "macos")]
    if let Err(e) = store::for_platform(fresh.paths()).and_then(|s| s.clear()) {
        eprintln!("ccam: could not delete the sign-in's Keychain item ({e})");
    }
    adopted
}

/// When another install uses `name`, switches that install to the free account with the most room, by running its
/// own ccam, so this side can take `name` without the two spending one refresh token.
fn free_elsewhere(manager: &Manager, name: &AccountName) -> Result<()> {
    let overview = manager.overview()?;
    let target = overview.listing.get(name).ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
    let marks = manager.marks();
    let Some(holder) = marks.iter().find(|m| m.identity == target.identity) else { return Ok(()) };
    if let Some(lane) = &holder.side.lane {
        return Err(Error::InLane { name: target.name.clone(), lane: lane.clone() });
    }
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
    let cards = usage::collect(manager, false, false).unwrap_or_default();
    let level = |name: &AccountName| {
        cards.iter().find(|c| &c.name == name).and_then(|c| match &c.state {
            usage::CardState::Ok { windows, .. } => {
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
    let status =
        ccam::launch::claude(&claude_args, None).map_err(|e| Error::Restart(format!("cannot start claude: {e}")))?;
    std::process::exit(status);
}

/// Opens Claude Code in a lane, so this terminal stays on the lane's account whatever other terminals switch to,
/// and its prompt cache is never broken by their switches.
fn run_lane(manager: &Manager, name: Option<AccountName>, lane: Option<LaneId>, args: &[String]) -> Result<()> {
    if std::env::var_os("CLAUDE_PID").is_some() {
        return Err(Error::Lane("ccam run starts Claude Code; run it from a normal terminal".into()));
    }
    // T3 Code runs its own Claude Code, so only run checks the version.
    let version = claude_version()?;
    let (id, in_lane, account) = enter_lane(manager, name, lane)?;
    sync_t3(manager);
    let main = manager.paths();

    let cwd = std::env::current_dir().map_err(|e| Error::Lane(format!("cannot read the current directory: {e}")))?;
    let project = lane::project_root(&cwd);
    let mut claude_args = Vec::new();
    let memory = match lane::launch_settings(main, in_lane.paths(), &project)? {
        Some(settings) => {
            claude_args.extend(["--settings".to_string(), settings.to_string_lossy().into_owned()]);
            format!("settings {}", settings.display())
        }
        None => format!("memory not redirected: no project folder found for {}", project.display()),
    };
    claude_args.extend_from_slice(args);
    let flags = flag_names(args);
    in_lane.log(&format!("run in {} on '{account}' (claude {version}, args {flags}, {memory})", cwd.display()));

    println!("Lane {id}: Claude Code on '{account}' in this terminal. Other terminals keep their own account.");
    let status = ccam::launch::claude(&claude_args, Some(&in_lane.paths().config_home))
        .map_err(|e| Error::Lane(format!("cannot start claude: {e}")))?;
    in_lane.log(&format!("claude exited with {status}"));
    std::process::exit(status);
}

/// The flags among claude's arguments, as `[--continue, -p]`. Values and positional arguments are left out: a
/// prompt or a file name can be private, and the switch log is not. Anything that does not look like a flag name
/// (say, a prompt that starts with `-`) is left out too.
fn flag_names(args: &[String]) -> String {
    let is_flag = |name: &str| {
        let bare = name.strip_prefix("--").or_else(|| name.strip_prefix('-')).unwrap_or_default();
        bare.starts_with(|c: char| c.is_ascii_alphanumeric())
            && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let flags: Vec<&str> =
        args.iter().map(|a| a.split_once('=').map_or(a.as_str(), |(name, _)| name)).filter(|n| is_flag(n)).collect();
    format!("[{}]", flags.join(", "))
}

/// Picks the lane for `name`, or the existing lane `lane`, prepares it and checks the account out into it.
/// `ccam run` and `ccam lane` both go through here, so they pick and refuse alike. Returns the lane, a manager
/// inside it, and the account it holds.
fn enter_lane(
    manager: &Manager,
    name: Option<AccountName>,
    lane: Option<LaneId>,
) -> Result<(LaneId, Manager, AccountName)> {
    if let Some(lane) = &manager.side().lane {
        return Err(Error::Lane(format!(
            "this terminal is in lane {lane}; open a new terminal, or unset CLAUDE_CONFIG_DIR first"
        )));
    }
    // Records which account the main login is on, so no lane can take it.
    let _ = manager.sync(Network::Offline);
    let main = manager.paths();

    let (id, wanted) = match name {
        None => {
            let lane = lane.expect("clap requires a name or --lane");
            (existing_lane(main, &lane)?, None)
        }
        Some(name) => {
            let overview = manager.overview()?;
            let account = overview.listing.get(&name).ok_or_else(|| Error::UnknownAccount(name.to_string()))?;
            if overview.live_account().is_some_and(|live| live.identity == account.identity) {
                return Err(Error::Lane(format!(
                    "'{0}' is the main login; open it with plain `claude`, or switch the main login away first (ccam to <other>)",
                    account.name
                )));
            }
            let holder = manager.used_elsewhere(&account.identity).and_then(|mark| mark.side.lane);
            let holder = holder.and_then(|l| l.parse::<LaneId>().ok()).and_then(|l| lane::find(main, &l));
            match (holder, lane) {
                (Some(holder), Some(asked)) if holder != asked => {
                    return Err(Error::InLane { name: account.name.clone(), lane: holder.to_string() });
                }
                (Some(holder), _) => (holder, Some(account.name.clone())),
                (None, Some(asked)) => (lane::find(main, &asked).unwrap_or(asked), Some(account.name.clone())),
                (None, None) => (available_lane(manager)?, Some(account.name.clone())),
            }
        }
    };

    let in_lane = manager_for(lane::paths(main, &id)?)?;
    let exe = std::env::current_exe().map_err(|e| Error::Lane(format!("cannot locate ccam: {e}")))?;
    lane::prepare(main, in_lane.paths(), &exe)?;
    let holds = in_lane.overview()?.live_account().map(|a| a.name.clone());
    let account = match (holds, wanted) {
        (None, None) => {
            return Err(Error::Lane(format!("lane {id} holds no account; run: ccam run <account> --lane {id}")));
        }
        (Some(held), Some(wanted)) if held != wanted => {
            return Err(Error::Lane(format!("lane {id} is on '{held}'; to change it, run: ccam move {id} {wanted}")));
        }
        (None, Some(wanted)) => in_lane.switch_to(&wanted)?.to,
        (Some(held), _) => {
            if let SyncOutcome::SavedIsNewer(name) = in_lane.sync(Network::Allowed)? {
                return Err(Error::GenerationConflict(name));
            }
            held
        }
    };
    Ok((id, in_lane, account))
}

/// What became of T3 Code's lane instances after a lane change.
enum T3 {
    NotInstalled,
    Synced,
    Failed,
}

/// Makes T3 Code's lane instances match the lanes. A failure is printed and logged but never fails the command:
/// the lane change it follows has already happened, and the next lane command retries.
fn sync_t3(manager: &Manager) -> T3 {
    let Some(file) = ccam::t3::settings_file() else { return T3::NotInstalled };
    match t3_lanes(manager).and_then(|lanes| ccam::t3::sync(&file, &lanes)) {
        Ok(ccam::t3::Sync::Unchanged) => T3::Synced,
        Ok(ccam::t3::Sync::Updated) => {
            manager.log("t3: updated");
            T3::Synced
        }
        Err(e) => {
            eprintln!("ccam: could not update T3 Code: {e}");
            manager.log(&format!("t3: {e}"));
            T3::Failed
        }
    }
}

fn t3_lanes(manager: &Manager) -> Result<Vec<ccam::t3::LaneInstance>> {
    let main = manager.paths();
    let instance = |id: LaneId| {
        let in_lane = manager_for(lane::paths(main, &id)?)?;
        let account = in_lane.overview()?.live_account().map(|a| a.name.clone());
        Ok(ccam::t3::LaneInstance { lane: id, home: in_lane.paths().config_home.clone(), account })
    };
    lane::all(main).into_iter().map(instance).collect()
}

/// `done`, after bringing T3 Code up to date. A lane command that failed part way may still have changed lanes.
fn reconciled(manager: &Manager, done: Result<()>) -> Result<()> {
    sync_t3(manager);
    done
}

/// An existing lane that is free (holds no account and has no running sessions), or the next free number.
fn available_lane(manager: &Manager) -> Result<LaneId> {
    let main = manager.paths();
    for id in lane::all(main) {
        if let Ok(paths) = lane::paths(main, &id)
            && let Ok(in_lane) = manager_for(paths)
            && let Ok(overview) = in_lane.overview()
            && matches!(overview.live, LiveState::LoggedOut)
            && lane_sessions(in_lane.paths()).is_ok_and(|s| s.is_empty())
        {
            return Ok(id);
        }
    }
    Ok(lane::next_free(&lane::all(main)))
}

/// Rewrites a lane's login, which moves every session in it on its next request, and no other session.
fn move_lane(manager: &Manager, id: &LaneId, name: &AccountName, yes: bool) -> Result<()> {
    let id = existing_lane(manager.paths(), id)?;
    let in_lane = manager_for(lane::paths(manager.paths(), &id)?)?;
    if let SyncOutcome::Unverified { name, reason } = in_lane.sync(Network::Allowed)? {
        return Err(Error::LiveUnverified { name, reason });
    }
    // Checked before asking, and worded for lanes: `to --force` cannot move the main login out of the way.
    in_lane.check_switch(name).map_err(|e| match e {
        Error::InUseElsewhere { name, side } => {
            Error::Lane(format!("'{name}' is in use by Claude on {side}; move {side} to another account first"))
        }
        e => e,
    })?;
    let from = in_lane.overview()?.live_account().map(|a| a.name.clone());

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(Error::Lane(format!(
                "ccam move asks before moving; run it in a terminal, or add --yes: ccam move {id} {name} --yes"
            )));
        }
        let running = lane_sessions(in_lane.paths()).map(|s| s.len().to_string()).unwrap_or_else(|_| "Any".into());
        let from = from.as_ref().map_or("no account".to_string(), |f| format!("'{f}'"));
        eprint!(
            "Move lane {id} from {from} to '{name}'? {running} running session(s) in it switch on their next message and rebuild their prompt cache once. [y/N] "
        );
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Nothing changed.");
            return Ok(());
        }
    }

    let switched = in_lane.switch_to(name)?;
    match &switched.from {
        Some(from) => println!("Moved lane {id} from '{from}' to '{}'.", switched.to),
        None => println!("Lane {id} now holds '{}'.", switched.to),
    }
    println!("Its sessions use '{}' from their next message.", switched.to);
    Ok(())
}

/// Checks a lane's account back into the vault and logs the lane out, so any terminal can use the account.
fn release_lane(manager: &Manager, id: &LaneId, force: bool, delete: bool) -> Result<()> {
    let id = existing_lane(manager.paths(), id)?;
    let in_lane = manager_for(lane::paths(manager.paths(), &id)?)?;
    let running = match lane_sessions(in_lane.paths()) {
        Ok(sessions) => sessions.len(),
        Err(e) if force => {
            in_lane.log(&format!("release --force: cannot list sessions: {e}"));
            0
        }
        Err(e) => {
            return Err(Error::Lane(format!(
                "cannot list lane {id}'s sessions ({e}); close them, then run: ccam release {id} --force"
            )));
        }
    };
    if running > 0 && !force {
        return Err(Error::Lane(format!(
            "lane {id} has {running} running session(s) (see ccam sessions); close them first, or run: ccam release {id} --force (they are logged out)"
        )));
    }
    match in_lane.release() {
        Err(Error::NotLoggedIn) => println!("Lane {id} holds no account."),
        Err(e) => return Err(e),
        Ok(name) => {
            if force {
                let plural = if running == 1 { "" } else { "s" };
                in_lane.log(&format!("release '{name}' (--force, {running} running session{plural})"));
            }
            println!("Released '{name}' from lane {id}. Any terminal can use it again.");
        }
    }
    if delete {
        if !manager.remove_lane(&in_lane, &id)? {
            return Err(Error::Lane(format!("lane {id} acquired a login before deletion; nothing was deleted")));
        }
        println!("Removed lane {id}.");
    }
    Ok(())
}

/// Removes empty lane directories that hold no account and have no running sessions.
fn prune_lanes(manager: &Manager) -> Result<()> {
    let main = manager.paths();
    let mut removed = 0;
    for id in lane::all(main) {
        let in_lane = manager_for(lane::paths(main, &id)?)?;
        let overview = in_lane.overview()?;
        let running = lane_sessions(in_lane.paths()).map(|s| s.len()).unwrap_or(1);
        if matches!(overview.live, LiveState::LoggedOut) && running == 0 && manager.remove_lane(&in_lane, &id)? {
            println!("Removed empty lane {id}.");
            removed += 1;
        }
    }
    if removed == 0 {
        println!("No empty lanes to remove.");
    }
    Ok(())
}

/// The main login and every lane: the account each holds and the sessions running on it.
fn print_sessions(manager: &Manager) -> Result<()> {
    let main = manager.paths();
    let account_label = |overview: ccam::manager::Overview| match overview.live_account() {
        Some(account) => format!("'{}'", account.name),
        None => match overview.live {
            LiveState::LoggedOut => "no account".to_string(),
            LiveState::Unreadable => "login unreadable".to_string(),
            LiveState::Present(_) => "unsaved login".to_string(),
        },
    };
    let mut rows = vec![("main".to_string(), account_label(manager.overview()?), lane_sessions(main))];
    for id in lane::all(main) {
        let in_lane = manager_for(lane::paths(main, &id)?)?;
        let holds = account_label(in_lane.overview()?);
        rows.push((format!("lane {id}"), holds, lane_sessions(in_lane.paths())));
    }
    for (label, holds, sessions) in rows {
        match sessions {
            Ok(sessions) if sessions.is_empty() => println!("{label}  {holds}  no sessions"),
            Ok(sessions) => {
                println!("{label}  {holds}  {} session{}", sessions.len(), if sessions.len() == 1 { "" } else { "s" });
                for s in sessions {
                    let pid = s.pid.map_or(String::new(), |p| format!("  pid {p}"));
                    println!("    {:<8}  {}  {}{pid}", s.status, s.cwd, s.session_id);
                }
            }
            Err(e) => println!("{label}  {holds}  sessions unknown: {e}"),
        }
    }
    Ok(())
}

fn existing_lane(main: &Paths, id: &LaneId) -> Result<LaneId> {
    lane::find(main, id).ok_or_else(|| Error::Lane(format!("no lane {id}; see: ccam sessions")))
}

/// Sessions running under a config dir, from `claude agents --json`, the documented way to read them.
fn lane_sessions(paths: &Paths) -> std::result::Result<Vec<lane::Session>, String> {
    let dir = paths.config_dir_env.as_ref().map(PathBuf::from);
    let output = ccam::launch::command(dir.as_deref())
        .args(["agents", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run claude: {e}"))?;
    if !output.status.success() {
        return Err(format!("claude agents failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    lane::parse_sessions(&String::from_utf8_lossy(&output.stdout))
}

/// Claude Code's version, refusing one too old for lanes.
fn claude_version() -> Result<String> {
    let output = ccam::launch::command(None)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| Error::Lane(format!("cannot run claude: {e}")))?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (major, minor, patch) = lane::MIN_CLAUDE;
    match lane::parse_version(&text) {
        Some(found) if found >= lane::MIN_CLAUDE => Ok(text.split_whitespace().next().unwrap_or_default().to_string()),
        _ => Err(Error::Lane(format!(
            "lanes need Claude Code {major}.{minor}.{patch} or later (found {text:?}); run: claude update"
        ))),
    }
}

fn show_usage(manager: &Manager, refresh: bool, as_json: bool, email: bool, info: bool) -> Result<()> {
    let mut cards = usage::collect(manager, refresh, info)?;
    if email || info {
        let listing = manager.overview()?.listing;
        for card in &mut cards {
            let account = listing.get(&card.name);
            card.email = account.and_then(|a| a.oauth_account["emailAddress"].as_str()).map(str::to_string);
        }
    }
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
