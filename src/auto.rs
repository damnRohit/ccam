//! `ccam auto`: watch the live account's usage and move to another saved account before the limit.
//!
//! This module is the pure core: usage arithmetic, target choice and the switch decision. It has no I/O, so
//! every rule is tested directly. The loop that feeds it lives in `autorun.rs`.

use std::time::Duration;

use jiff::Timestamp;

use crate::name::AccountName;

/// One reading of an account's usage.
#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub five_hour: f64,
    pub seven_day: f64,
    pub five_resets: Option<Timestamp>,
    pub seven_resets: Option<Timestamp>,
    pub read_at: Timestamp,
}

impl Usage {
    /// Utilisation as of `now`. A window whose reset time has passed is empty, so a cached reading stays
    /// useful without a fresh request.
    pub fn at(&self, now: Timestamp) -> (f64, f64) {
        let live = |pct: f64, resets: Option<Timestamp>| if resets.is_some_and(|r| r <= now) { 0.0 } else { pct };
        (live(self.five_hour, self.five_resets), live(self.seven_day, self.seven_resets))
    }

    /// The tighter of the two windows, 0 to 100.
    pub fn level(&self, now: Timestamp) -> f64 {
        let (five, seven) = self.at(now);
        five.max(seven)
    }

    /// Room left before either window is full.
    pub fn headroom(&self, now: Timestamp) -> f64 {
        100.0 - self.level(now)
    }
}

/// Picks the account to move to, or `None` when no account has meaningfully more room.
///
/// Eligible: 5-hour use comfortably under the threshold, weekly use under it, at least 20 points more headroom
/// than the live account, and not the account just left (which would bounce straight back). Among eligible
/// accounts, headroom in 10-point bands wins; within a band, the sooner weekly reset and then the sooner 5-hour
/// reset win, so an account whose quota is about to refill anyway gets used first.
pub fn pick_target(
    live: &Usage,
    candidates: &[(AccountName, Usage)],
    last_left: Option<&AccountName>,
    threshold: f64,
    now: Timestamp,
) -> Option<AccountName> {
    let live_room = live.headroom(now);
    let far_future = Timestamp::MAX;
    candidates
        .iter()
        .filter(|(name, _)| last_left != Some(name))
        .filter(|(_, u)| {
            let (five, seven) = u.at(now);
            five < threshold - ELIGIBLE_MARGIN && seven < threshold && u.headroom(now) >= live_room + MIN_GAIN
        })
        .min_by_key(|(_, u)| {
            let band = (u.headroom(now) / 10.0).floor() as i64;
            let reset = |r: Option<Timestamp>| r.filter(|r| *r > now).unwrap_or(far_future);
            (-band, reset(u.seven_resets), reset(u.five_resets))
        })
        .map(|(name, _)| name.clone())
}

/// Whether `ccam auto` switches by itself or only tells you which account to switch to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Notify,
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub mode: Mode,
    /// Switch (or notify) at this level of the tighter window, 0 to 100.
    pub threshold: f64,
    /// From this level, other accounts' usage is fetched so a target is ready before it is needed.
    pub arm_at: f64,
    /// Minimum time between automatic switches.
    pub cooldown: Duration,
    /// If no turn ends this long after crossing the threshold, switch anyway.
    pub max_wait_for_turn: Duration,
}

impl Settings {
    pub fn new(mode: Mode, threshold: f64) -> Self {
        Self {
            mode,
            threshold,
            arm_at: (threshold - 8.0).max(50.0),
            cooldown: Duration::from_secs(15 * 60),
            max_wait_for_turn: Duration::from_secs(10 * 60),
        }
    }
}

/// What the loop remembers between decisions.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// When the live account crossed the threshold; cleared once it drops back or a switch happens.
    crossed_at: Option<Timestamp>,
    /// The target already announced in notify mode, so the same advice is not repeated every tick.
    announced: Option<AccountName>,
    last_switch: Option<Timestamp>,
    /// Switch times within the last five hours, to cap how often accounts rotate.
    recent_switches: Vec<Timestamp>,
    last_left: Option<AccountName>,
}

impl State {
    /// Records a completed switch away from `from`.
    pub fn switched(&mut self, from: AccountName, now: Timestamp) {
        self.crossed_at = None;
        self.announced = None;
        self.last_switch = Some(now);
        self.recent_switches.push(now);
        self.last_left = Some(from);
    }
}

pub struct Inputs<'a> {
    pub now: Timestamp,
    pub live: &'a Usage,
    pub candidates: &'a [(AccountName, Usage)],
    /// Claude Code reported that a request hit the usage limit.
    pub limit_hit: bool,
    /// A Claude Code turn has ended since the threshold was crossed, so a switch will not land mid-reply.
    pub turn_ended: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Below the arm level: nothing to do.
    Idle,
    /// Close to the threshold: keep the other accounts' usage fresh.
    Armed,
    /// Over the threshold, target chosen, waiting for the turn to end.
    Pending {
        target: AccountName,
    },
    Switch {
        target: AccountName,
        reason: Reason,
    },
    /// Notify mode: tell the user once which account to move to.
    Advise {
        target: AccountName,
    },
    /// Over the threshold but a recent switch blocks another one.
    CoolingDown,
    /// No other account has room. `next` is the account whose window reopens first.
    Exhausted {
        next: Option<(AccountName, Timestamp)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Crossed the threshold and a turn ended.
    Threshold,
    /// Claude Code reported the limit was hit.
    LimitHit,
    /// Crossed the threshold and no turn ended within the wait.
    Timeout,
}

const FIVE_HOURS: Duration = Duration::from_secs(5 * 3600);

/// Once advice has been given in notify mode, the live account must drop this far below the threshold before
/// the same advice is given again, so a level hovering around the threshold does not repeat it.
const REARM_BELOW: f64 = 10.0;

pub fn decide(inputs: &Inputs, state: &mut State, settings: &Settings) -> Decision {
    let now = inputs.now;
    let level = inputs.live.level(now);
    if level < settings.threshold && !inputs.limit_hit {
        state.crossed_at = None;
        if level < settings.threshold - REARM_BELOW {
            state.announced = None;
        }
        return if level >= settings.arm_at { Decision::Armed } else { Decision::Idle };
    }
    let crossed_at = *state.crossed_at.get_or_insert(now);

    let Some(target) = pick_target(inputs.live, inputs.candidates, state.last_left.as_ref(), settings.threshold, now)
    else {
        return Decision::Exhausted { next: first_to_reopen(inputs.candidates, settings.threshold, now) };
    };

    if settings.mode == Mode::Notify {
        if state.announced.as_ref() == Some(&target) {
            return Decision::Pending { target };
        }
        state.announced = Some(target.clone());
        return Decision::Advise { target };
    }

    // A hard limit overrides the cooldown (the session is stuck otherwise) but not the rotation cap, which
    // exists to stop a runaway loop from cycling through every account.
    state.recent_switches.retain(|t| elapsed(*t, now) < FIVE_HOURS);
    let capped = state.recent_switches.len() > inputs.candidates.len();
    let cooling = state.last_switch.is_some_and(|t| elapsed(t, now) < settings.cooldown);
    if capped || (cooling && !inputs.limit_hit) {
        return Decision::CoolingDown;
    }

    let reason = if inputs.limit_hit {
        Reason::LimitHit
    } else if inputs.turn_ended {
        Reason::Threshold
    } else if elapsed(crossed_at, now) >= settings.max_wait_for_turn {
        Reason::Timeout
    } else {
        return Decision::Pending { target };
    };
    Decision::Switch { target, reason }
}

/// When to ask the usage endpoint about the live account.
///
/// The endpoint is rate limited per account (about 30 requests an hour observed by claude-swap), and usage only
/// moves while Claude is used, so checks are rare at low usage, frequent near the threshold, sooner after a
/// Claude Code turn ends (the only time usage changed), and back off hard after a 429.
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    last_poll: Option<Timestamp>,
    blocked_until: Option<Timestamp>,
    backoff: Duration,
}

const BACKOFF_START: Duration = Duration::from_secs(15 * 60);
const BACKOFF_MAX: Duration = Duration::from_secs(75 * 60);

impl Schedule {
    /// `level` is the last known level of the live account, `None` if never read. `turn_ended` is true when a
    /// Claude Code turn has ended since the last poll.
    pub fn due(&self, now: Timestamp, level: Option<f64>, turn_ended: bool, settings: &Settings) -> bool {
        if self.blocked_until.is_some_and(|until| now < until) {
            return false;
        }
        let (Some(last), Some(level)) = (self.last_poll, level) else { return true };
        // (quiet interval, interval after a finished turn), in minutes.
        let (quiet, after_turn) = match level {
            l if l >= settings.arm_at => (2, 1),
            l if l >= 80.0 => (5, 2),
            l if l >= 50.0 => (15, 5),
            _ => (30, 10),
        };
        let wait = if turn_ended { after_turn } else { quiet };
        elapsed(last, now) >= Duration::from_secs(wait * 60)
    }

    pub fn polled(&mut self, now: Timestamp) {
        self.last_poll = Some(now);
        self.blocked_until = None;
        self.backoff = Duration::ZERO;
    }

    /// Records a 429: no requests until the back-off passes, doubling on each repeat.
    pub fn rate_limited(&mut self, now: Timestamp) {
        self.backoff = if self.backoff.is_zero() { BACKOFF_START } else { (self.backoff * 2).min(BACKOFF_MAX) };
        let backoff = jiff::SignedDuration::try_from(self.backoff).unwrap_or(jiff::SignedDuration::MAX);
        self.blocked_until = Some(now.saturating_add(backoff).unwrap_or(now));
    }
}

fn elapsed(since: Timestamp, now: Timestamp) -> Duration {
    Duration::from_millis(u64::try_from(now.as_millisecond() - since.as_millisecond()).unwrap_or(0))
}

/// The candidate whose blocking windows all reset soonest, with that time.
fn first_to_reopen(
    candidates: &[(AccountName, Usage)],
    threshold: f64,
    now: Timestamp,
) -> Option<(AccountName, Timestamp)> {
    candidates
        .iter()
        .filter_map(|(name, u)| {
            let (five, seven) = u.at(now);
            let mut blocking = Vec::new();
            if five >= threshold - ELIGIBLE_MARGIN {
                blocking.push(u.five_resets?);
            }
            if seven >= threshold {
                blocking.push(u.seven_resets?);
            }
            blocking.into_iter().max().map(|reopens| (name.clone(), reopens))
        })
        .min_by_key(|(_, reopens)| *reopens)
}

/// A candidate's 5-hour use must be this far under the threshold, so the new account is not itself about to
/// trip the switch again.
const ELIGIBLE_MARGIN: f64 = 20.0;
/// A switch must buy at least this many points of headroom to be worth a full prompt-cache rebuild.
const MIN_GAIN: f64 = 20.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    const NOW: &str = "2026-09-24T12:00:00Z";

    fn usage(five: f64, seven: f64) -> Usage {
        Usage { five_hour: five, seven_day: seven, five_resets: None, seven_resets: None, read_at: ts(NOW) }
    }

    fn name(s: &str) -> AccountName {
        s.parse().unwrap()
    }

    fn settings(mode: Mode) -> Settings {
        Settings::new(mode, 98.0)
    }

    fn at(minutes: i64) -> Timestamp {
        ts(NOW) + jiff::SignedDuration::from_mins(minutes)
    }

    fn inputs<'a>(now: Timestamp, live: &'a Usage, candidates: &'a [(AccountName, Usage)]) -> Inputs<'a> {
        Inputs { now, live, candidates, limit_hit: false, turn_ended: false }
    }

    #[test]
    fn below_the_arm_level_nothing_happens_and_close_to_it_the_cache_is_armed() {
        let mut state = State::default();
        let others = [(name("b"), usage(0.0, 0.0))];
        let s = settings(Mode::Auto);
        assert_eq!(decide(&inputs(at(0), &usage(50.0, 20.0), &others), &mut state, &s), Decision::Idle);
        assert_eq!(decide(&inputs(at(0), &usage(91.0, 20.0), &others), &mut state, &s), Decision::Armed);
    }

    #[test]
    fn over_the_threshold_it_waits_for_the_turn_to_end_then_switches() {
        let mut state = State::default();
        let others = [(name("b"), usage(0.0, 0.0))];
        let live = usage(98.5, 20.0);
        let s = settings(Mode::Auto);

        assert_eq!(decide(&inputs(at(0), &live, &others), &mut state, &s), Decision::Pending { target: name("b") });
        let ended = Inputs { turn_ended: true, ..inputs(at(1), &live, &others) };
        assert_eq!(decide(&ended, &mut state, &s), Decision::Switch { target: name("b"), reason: Reason::Threshold });
    }

    #[test]
    fn a_hit_limit_switches_at_once_and_a_turn_that_never_ends_switches_after_the_wait() {
        let others = [(name("b"), usage(0.0, 0.0))];
        let s = settings(Mode::Auto);

        let mut state = State::default();
        let full = usage(100.0, 20.0);
        let hit = Inputs { limit_hit: true, ..inputs(at(0), &full, &others) };
        assert_eq!(decide(&hit, &mut state, &s), Decision::Switch { target: name("b"), reason: Reason::LimitHit });

        let mut state = State::default();
        let live = usage(98.5, 20.0);
        decide(&inputs(at(0), &live, &others), &mut state, &s);
        assert!(matches!(decide(&inputs(at(9), &live, &others), &mut state, &s), Decision::Pending { .. }));
        assert_eq!(
            decide(&inputs(at(10), &live, &others), &mut state, &s),
            Decision::Switch { target: name("b"), reason: Reason::Timeout }
        );
    }

    #[test]
    fn notify_mode_advises_once_per_crossing_and_never_switches() {
        let mut state = State::default();
        let others = [(name("b"), usage(0.0, 0.0))];
        let live = usage(99.0, 20.0);
        let s = settings(Mode::Notify);

        let ended = Inputs { turn_ended: true, limit_hit: true, ..inputs(at(0), &live, &others) };
        assert_eq!(decide(&ended, &mut state, &s), Decision::Advise { target: name("b") });
        assert_eq!(decide(&ended, &mut state, &s), Decision::Pending { target: name("b") });

        // Dropping well below the threshold re-arms the advice for the next crossing.
        let low = usage(40.0, 20.0);
        decide(&inputs(at(5), &low, &others), &mut state, &s);
        assert_eq!(decide(&ended, &mut state, &s), Decision::Advise { target: name("b") });
    }

    #[test]
    fn a_recent_switch_blocks_another_until_the_cooldown_passes_unless_the_limit_is_hit() {
        let others = [(name("c"), usage(0.0, 0.0))];
        let live = usage(99.0, 20.0);
        let s = settings(Mode::Auto);
        let mut state = State::default();
        state.switched(name("a"), at(0));

        let ended = |m| Inputs { turn_ended: true, ..inputs(at(m), &live, &others) };
        assert_eq!(decide(&ended(5), &mut state, &s), Decision::CoolingDown);
        let hit = Inputs { limit_hit: true, ..ended(6) };
        assert!(matches!(decide(&hit, &mut state, &s), Decision::Switch { reason: Reason::LimitHit, .. }));
        assert!(matches!(decide(&ended(16), &mut state, &s), Decision::Switch { .. }));
    }

    #[test]
    fn rotation_stops_after_one_switch_per_account_within_five_hours() {
        let others = [(name("b"), usage(0.0, 0.0))];
        let live = usage(99.0, 20.0);
        let s = settings(Mode::Auto);
        let mut state = State::default();
        // Two accounts in total (live plus one candidate): two switches within five hours is the cap.
        state.switched(name("x"), at(-120));
        state.switched(name("y"), at(-60));

        let ended = Inputs { turn_ended: true, ..inputs(at(0), &live, &others) };
        assert_eq!(decide(&ended, &mut state, &s), Decision::CoolingDown);
        let later = Inputs { turn_ended: true, ..inputs(at(190), &live, &others) };
        assert!(matches!(decide(&later, &mut state, &s), Decision::Switch { .. }));
    }

    #[test]
    fn when_every_account_is_full_it_reports_which_one_reopens_first() {
        let mut soon = usage(99.0, 10.0);
        soon.five_resets = Some(at(40));
        let mut late = usage(99.0, 10.0);
        late.five_resets = Some(at(200));
        let others = [(name("late"), late), (name("soon"), soon)];
        let mut state = State::default();

        let full = usage(100.0, 10.0);
        let hit = Inputs { limit_hit: true, ..inputs(at(0), &full, &others) };
        assert_eq!(
            decide(&hit, &mut state, &settings(Mode::Auto)),
            Decision::Exhausted { next: Some((name("soon"), at(40))) }
        );
    }

    #[test]
    fn the_first_check_is_immediate_and_low_usage_is_checked_rarely() {
        let s = settings(Mode::Auto);
        let mut sched = Schedule::default();
        assert!(sched.due(at(0), None, false, &s));
        sched.polled(at(0));
        assert!(!sched.due(at(29), Some(20.0), false, &s));
        assert!(sched.due(at(30), Some(20.0), false, &s));
    }

    #[test]
    fn a_finished_turn_brings_the_next_check_forward() {
        let s = settings(Mode::Auto);
        let mut sched = Schedule::default();
        sched.polled(at(0));
        assert!(!sched.due(at(9), Some(20.0), true, &s));
        assert!(sched.due(at(10), Some(20.0), true, &s));
        // Near the threshold a finished turn is checked within a minute.
        assert!(sched.due(at(1), Some(95.0), true, &s));
        assert!(!sched.due(at(1), Some(95.0), false, &s));
        assert!(sched.due(at(2), Some(95.0), false, &s));
    }

    #[test]
    fn a_rate_limit_blocks_checks_and_backs_off_further_on_repeats() {
        let s = settings(Mode::Auto);
        let mut sched = Schedule::default();
        sched.rate_limited(at(0));
        assert!(!sched.due(at(14), Some(99.0), true, &s));
        assert!(sched.due(at(15), Some(99.0), true, &s));
        sched.rate_limited(at(15));
        assert!(!sched.due(at(44), Some(99.0), true, &s));
        assert!(sched.due(at(45), Some(99.0), true, &s));
        // A successful check resets the back-off.
        sched.polled(at(45));
        sched.rate_limited(at(50));
        assert!(sched.due(at(65), Some(99.0), true, &s));
    }

    #[test]
    fn a_window_past_its_reset_counts_as_empty() {
        let mut u = usage(97.0, 60.0);
        u.five_resets = Some(ts("2026-09-24T11:59:00Z"));
        assert_eq!(u.at(ts(NOW)), (0.0, 60.0));
        assert_eq!(u.headroom(ts(NOW)), 40.0);
    }

    #[test]
    fn picks_the_account_with_the_most_room_in_the_tighter_window() {
        let live = usage(98.0, 40.0);
        let candidates =
            [(name("b"), usage(12.0, 33.0)), (name("c"), usage(0.0, 71.0)), (name("d"), usage(50.0, 50.0))];
        assert_eq!(pick_target(&live, &candidates, None, 98.0, ts(NOW)), Some(name("b")));
    }

    #[test]
    fn never_bounces_back_to_the_account_just_left() {
        let live = usage(98.0, 10.0);
        let candidates = [(name("b"), usage(0.0, 0.0)), (name("c"), usage(20.0, 20.0))];
        assert_eq!(pick_target(&live, &candidates, Some(&name("b")), 98.0, ts(NOW)), Some(name("c")));
    }

    #[test]
    fn nothing_qualifies_when_every_account_is_nearly_full() {
        let live = usage(98.0, 50.0);
        let candidates = [(name("b"), usage(85.0, 10.0)), (name("c"), usage(10.0, 99.0))];
        assert_eq!(pick_target(&live, &candidates, None, 98.0, ts(NOW)), None);
    }

    #[test]
    fn a_candidate_needs_clearly_more_room_than_the_live_account() {
        // Live is capped by its weekly window at 60% used: moving to an account with 45% headroom gains nothing.
        let live = usage(98.0, 60.0);
        let marginal = [(name("b"), usage(55.0, 20.0))];
        assert_eq!(pick_target(&usage(40.0, 60.0), &marginal, None, 98.0, ts(NOW)), None);
        assert_eq!(pick_target(&live, &marginal, None, 98.0, ts(NOW)), Some(name("b")));
    }

    #[test]
    fn within_the_same_headroom_band_the_sooner_weekly_reset_wins() {
        let live = usage(99.0, 20.0);
        // 69 and 62 points of headroom: the same 10-point band, so reset time decides.
        let mut later = usage(31.0, 31.0);
        later.seven_resets = Some(ts("2026-09-28T00:00:00Z"));
        let mut sooner = usage(38.0, 38.0);
        sooner.seven_resets = Some(ts("2026-09-25T00:00:00Z"));
        let candidates = [(name("later"), later), (name("sooner"), sooner)];
        assert_eq!(pick_target(&live, &candidates, None, 98.0, ts(NOW)), Some(name("sooner")));
    }

    #[test]
    fn a_stale_cached_reading_is_judged_by_its_reset_times() {
        // Cached at 100% an hour ago, but that 5-hour window has since reset.
        let live = usage(98.0, 10.0);
        let mut refilled = usage(100.0, 30.0);
        refilled.five_resets = Some(ts("2026-09-24T11:30:00Z"));
        assert_eq!(pick_target(&live, &[(name("b"), refilled)], None, 98.0, ts(NOW)), Some(name("b")));
    }
}
