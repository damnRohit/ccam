# `ccam auto`: limit-triggered automatic switching

Researched 2026-09-24 against Claude Code **2.1.281** (`claude --version`, local). No prompt was sent; the only local commands were `claude --version`, `claude --help` and `claude auth --help`. Every claim has a URL. **(inferred)** marks my reading, **(third-party)** a non-Anthropic source, **(local)** something checked on this machine.

This file does not repeat `docs/SPEC.md` (switch invariants, locks, vault), `docs/research/internals.md` (hook table, rotation, in-place pickup) or `docs/research/window-warmup.md` (Terms text already quoted, `/api/oauth/usage` shape). It cites them by section.

Doc URLs used below:
- H = https://code.claude.com/docs/en/hooks
- SL = https://code.claude.com/docs/en/statusline
- ERR = https://code.claude.com/docs/en/errors
- IM = https://code.claude.com/docs/en/interactive-mode
- SR = https://code.claude.com/docs/en/settings-reference
- HL = https://code.claude.com/docs/en/headless
- CMD = https://code.claude.com/docs/en/commands
- PC = https://code.claude.com/docs/en/prompt-caching
- COST = https://code.claude.com/docs/en/costs
- SDK = https://code.claude.com/docs/en/agent-sdk/typescript
- LC = https://code.claude.com/docs/en/legal-and-compliance
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- CS = https://raw.githubusercontent.com/realiti4/claude-swap/HEAD/src/claude_swap/ (read at HEAD on 2026-09-24)
- CT = https://www.anthropic.com/legal/consumer-terms (effective October 8, 2025, re-fetched today)
- AUP = https://www.anthropic.com/legal/aup (effective September 15, 2025, re-fetched today)
- HC-LIM = https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work

---

## 1. Event sources instead of polling

| Signal | What it carries | When it fires | Fires only with activity? |
|---|---|---|---|
| Status line `rate_limits.five_hour` / `.seven_day` | `used_percentage` (0-100), `resets_at` (Unix s). Pro/Max only, "only after the first API response in the session"; each window may be absent; dropped once `resets_at` passes (SL). Added 2.1.80 (CL). | Once at session start/resume, then on: new assistant message, `/compact` done, permission-mode change, vim toggle, `command` change, `refreshInterval` tick if set, a `rate_limits` window reaching `resets_at`, a warm prompt cache reaching `expires_at`. Debounced 300 ms; a new trigger cancels an in-flight script (SL "When it updates"). | **Yes**, if `refreshInterval` is left unset: "Leave it unset to run only on events", and "event-driven triggers can go quiet when the main session is idle" (SL). The reset and cache-expiry triggers are the only idle-time runs, at most a few per window. |
| `StopFailure` matcher `rate_limit` | `error`, optional `error_details` (e.g. `"429 Too Many Requests"`), `last_assistant_message` (e.g. `"API Error: Rate limit reached"`). No decision control; output ignored (H#stopfailure). Added 2.1.78 (CL). | "Runs instead of Stop when the turn ends due to an API error" (H). | Yes. |
| `Stop` | `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons`. No usage data (H#stop). | "When the main Claude Code agent has finished responding. Does not run if the stoppage occurred due to a user interrupt" (H). | Yes. This is the "turn ended" signal (section 3). |
| `Notification` `quota_auto_resume_fired` / `_stale` / `_disabled` | `message`, `notification_type` (H#notification). v2.1.234+. | Only around Claude Code's own wait-at-limit (section 4). `idle_prompt` is suppressed during that wait (H). | Yes, and only after a limit was already hit. |
| Interactive warning | `You've used 85% of your session limit · resets 3:45pm` (ERR#youve-hit-your-session-limit). | Text in the TUI. No hook exposes it; the docs point at the status line instead (ERR). | Yes. |
| Headless `system/api_retry` | `error: "rate_limit"`, `error_status`, `retry_delay_ms` (HL "Handle API retries"). | `-p --output-format stream-json` only. | Yes. Irrelevant for an interactive tmux session. |
| Agent SDK `rate_limit_event` | `status: allowed \| allowed_warning \| rejected`, `resetsAt`, `utilization`, `errorCode: "credits_required"` (SDK `SDKRateLimitEvent`; added 2.1.45 per CL). | SDK consumers only. | Yes. Not reachable from an interactive session. |
| `anthropic-ratelimit-unified-*` response headers | ERR confirms Claude Code reads "the unified quota headers a real limit response carries" to tell a plan limit from a server throttle (ERR "Server is temporarily limiting requests"). Header names are undocumented; `anthropic-ratelimit-unified-5h-reset` is third-party (window-warmup.md section 1). | Every model response. | **No hook exposes headers** (H: no event lists header fields). **(inferred)** The status line `rate_limits` and SDK `rate_limit_event` are the documented projections of these headers. |
| `/usage` | Plan bars and reset times. Runs immediately even mid-turn (CMD). Shows last-known bars "as of" when the usage endpoint is rate-limited (CL 2.1.208). | Interactive. | n/a |
| `claude usage` CLI | **Does not exist.** `claude --help` on 2.1.281 lists `agents, attach, auth, auto-mode, doctor, gateway, import, install, logs, mcp, plugin, project, respawn, rm, setup-token, stop, ultrareview, update` (local). `claude auth status --json` reports login state, not usage (internals.md 2.4). SDK doc: `/usage` sent as a prompt returns plain text in an assistant message (SDK `SDKLocalCommandOutputMessage`). **(inferred)** Not a usable JSON source, and it spawns a whole Claude Code process. |

**Conclusion.** Every signal above except the usage endpoint fires only when Claude Code makes model requests, which is exactly when usage can change on this machine. The status line is the one that carries numbers. `Stop` is the one that marks a safe switch point. `StopFailure:rate_limit` is the one that says the threshold was missed.

**Blind spots (inferred).** The status line does not run in `-p` mode, VS Code, Desktop, or claude.ai, yet all of those draw from the same pool (HC-LIM: "your usage of all different Claude product surfaces (claude.ai, Claude Code, Claude Desktop) counts towards the same usage limit"). If you also use those surfaces, a low-rate endpoint poll is still needed as a backstop.

**Attribution caveat (inferred).** The status-line JSON carries no account identity (SL field table). A sample written right after a swap can still describe the old account, because the response that produced it was already in flight. Treat samples as **triggers**, never as the basis for a decision (section 6).

## 2. The usage endpoint and how claude-swap handles it

### 2.1 Rate limit on `/api/oauth/usage`
- **First-party evidence that it is rate-limited:** CL 2.1.208 "`/usage` now shows your last-known usage bars with an 'as of' note when the usage endpoint is rate-limited"; CL 2.1.261 fixed `/usage` "dropping a model-specific weekly limit row when the usage endpoint is rate limited"; CL 2.1.121 fixed "`/usage` returning 'rate limited' after a stale OAuth token". Anthropic documents no budget.
- **Third-party measurement** (CS `poll_policy.py` docstring, measured 2026-07-11 to 07-29): "a ~60-minute window of ~28-30 requests per identity × UA-class". "It is NOT a bucket with a refill rate: capacity returns only as old requests age out of the trailing hour, so a burst saturates the identity for up to a full hour". The scope is the account/org in one regime and the access token in another. Plan for the account-scoped case: "re-authenticating cannot be relied on to clear a block". Target: "an average of at most ~1 request / 3 minutes (20/hour vs the ~28-30/hour cap)".
- Retry-After semantics (CS `usage_store.py`): `Retry-After: 0` means the saturated edge, so probe no more than every 300 s. `N>0` is a fixed deadline that counts down and is "NOT re-armed by probing"; honour it plus `RETRY_AFTER_MARGIN_S = 900.0` (20 of 35 observed lapses re-blocked within 900 s of their deadline), capped at `RETRY_AFTER_FLOOR_CAP_S = 4500.0`.
- **ccam today (local):** `src/api.rs` maps 429 to `ApiError::RateLimited` and drops `Retry-After`, with User-Agent `ccam/<version>`. A long-running loop has to keep `Retry-After`.

### 2.2 claude-swap `cswap auto`, precisely

**Polling intervals and backoff** (CS `poll_policy.py`, `usage_store.py`, `autoswitch.py`, `settings.py`):

```python
SERVE_TTL_S = 180.0                 # entry younger than this is served from store, no fetch
MIN_INTERVAL_S = 180.0              # normal floor
URGENT_INTERVAL_S = 60.0            # active acct, moving, within ESCALATION_MARGIN_PCT of threshold
ACTIVE_MAX_INTERVAL_S = 300.0
CANDIDATE_DEFAULT_INTERVAL_S = 300.0
CANDIDATE_MAX_INTERVAL_S = 600.0
EXHAUSTED_INTERVAL_S = 600.0        # keep probing exhausted accounts (early quota grants)
MOVEMENT_DELTA_PCT = 1.0
JITTER_FRAC = 0.1
EDGE_BACKOFF_S = 300.0              # after 429 with Retry-After: 0
POST_429_MIN_INTERVAL_S = 360.0
RECENT_429_WINDOW_S = 3600.0
POST_429_BACKOFF_MULT = 1.5         # AIMD across machines sharing a budget
POST_429_MAX_INTERVAL_S = 1800.0
ESCALATION_MARGIN_PCT = 15.0
RESET_SLACK_S = 60.0
BACKOFF_BASE_S = 30.0; BACKOFF_CAP_S = 600.0   # non-429 failures: 30 s * 2^(n-1)
interval_seconds = 60.0             # engine tick (settings default)
```

- After each fetch (`plan_after_fetch`): usage that moved at least 1 point halves the interval (floor 180 s). Usage that did not move multiplies it by 1.5 up to the ceiling. The active account drops to 60 s when it is moving and within 15 points of the threshold. A 429 in the last hour floors the interval at 360 s and grows it ×1.5 up to 1800 s. The next poll is never later than a known reset + 60 s.
- Per tick it fetches the active account (if due) plus **one** candidate: the stalest, never-fetched first. It refetches **all** candidates only when active utilization is ≥ threshold − 15 or unknown (`_collect_scheduled_usage`).

**Threshold logic** (CS `autoswitch.py`, `settings.py`):
- `threshold: float = 90.0` (range 50-99.9). The docstring explains why it is not 95: "leaves margin for the macOS ~30s Keychain pickup tail and for heavy subagent turns burning past the mark before a swap lands".
- Binding utilization = max over 5h, 7d, and optionally named per-model weekly windows (`oauth.account_headroom` = `100 - max(pct)`).
- Triggers: `proactive` (≥ threshold), `at-limit` (headroom ≤ 0), `failover` (active usage unreadable for `unhealthy_ticks = 3` ticks), `consume-first` (optional strategy).
- `cooldown_seconds = 300.0` applies to proactive switches only. It is bypassed at the hard limit.

**Target choice** (`_rank_candidates`), default strategy `best`:
1. Drop candidates with unknown headroom, headroom ≤ 0, quarantined, or the no-return account (the one just left, until it has "recovered" by `SPENT_HEADROOM_PCT` or `RECOVERY_HYSTERESIS_S`).
2. On a proactive trigger the target must be below the threshold itself, and `h_target - h_active >= hysteresis_pct` (`10.0`).
3. Sort key `(-headroom,)`, so most headroom wins and slot order breaks ties.
4. **Everyone ≥ threshold** (`_every_account_above_threshold`): the goal becomes "soonest back". Per candidate, `_recovery_is_useful` picks the axis. It ranks by recovery time (`key (0, recovery_ts, -h)`, with `recovery_ts < active_recovery_ts - RECOVERY_HYSTERESIS_S (300 s)`) when both active and best candidate have ≤ `SPENT_HEADROOM_PCT = 3.0` headroom, or when either side's binding reset is within `RECOVERY_HORIZON_S = 4*3600`. Otherwise it ranks by headroom with a ratio margin `h >= active_h * HORIZON_HEADROOM_RATIO (2.0)`, key `(1, -h, recovery_ts)`.
5. `consume-first`: key `(seven_day_reset_ts, -h)`, and below the threshold it only moves to strictly sooner-resetting accounts.
6. If every candidate is ≤ 0: emit `all-exhausted` and sleep until the earliest proven recovery + 60 s, capped at `MAX_SLEEP_S = 600`.
- The code comments document real flapping incidents ("47 credential rewrites over 3.9h on frozen inputs"). They are the reason for the no-return bar and for the ratio and seconds margins.

**Mid-request avoidance: none.** The engine switches on its tick whatever Claude Code is doing. It relies on the proactive threshold and on Claude Code re-reading credentials on the next request ("New account is active on your next message", CS `switcher.py`). Before activation it freshens the target token if it expires within `FRESHEN_BUFFER_MS = 10*60*1000`, "twice Claude Code's own 5-minute refresh buffer, so its post-lock 'abort refresh if not expired' re-read holds". It skips a target owned by a live `cswap run` session (`skip-live-session`).

**Caching:**
- A persisted per-account usage store holds `nextPollAt`/`pollIntervalS`, so every surface shares one cadence.
- `STALE_OK_S = 300.0` is the age up to which a row is decision-grade. `TRUST_MAX_AGE_S = 3600.0` is the hard ceiling.
- After a 429, rows are trusted until their window's reset, because "usage only rises within a window (monotone until the window resets), so last_good is a valid lower bound" (`RATE_LIMIT_TRUST_MAX_AGE_S = 7200.0` fallback).
- A fetch claim lease `CLAIM_TTL_S = 90.0` stops concurrent collectors from double-fetching.
- `cache.py` itself is only a generic timestamped JSON TTL file.

**The account it leaves:** `switch_to` runs the "backup-current step", saving the live credential into its slot before activating the target (CS `switcher.py`). The engine records `lastSwitchFrom`, `leftHeadroom`, `leftRecoveryAt` and `leftTrigger` in `autoswitch_state.json` so the no-return bar has a release that "burn cannot fake". Candidates with `invalid_grant` or an identity conflict are quarantined, and quarantine is released automatically when the credential is replaced.

**Stop and daemon handling:**
- Foreground loop only. `threading.Event` `_stop`/`_wake` drive `self._wake.wait(delay)`, and SIGTERM calls `engine.stop()` ("SIGTERM (systemd stop) exits the loop cleanly").
- Ctrl-C exits 130. `--once` gives cron exit codes (0 switched, 1 error, 2 no action, 3 blocked).
- There is no pid file and no `auto stop` command. Concurrent engines serialize on a file lock around `autoswitch_state.json` (`_perform` "Hold the state lock across the whole recheck -> switch -> record sequence").
- The only daemonization is a macOS LaunchAgent, and that is for the menu bar (`launch_agent.py`).

**claude-swap does not use hooks or the status line** (no match for `statusline`, `rate_limits` or `StopFailure` in its sources). It is a pure poller.

## 3. Switch timing safety

**What happens to an in-flight request (inferred from the sources below):**
- Claude Code builds each HTTP request with the token it holds. An in-place swap changes which token the *next* request uses: "on Windows and Linux a running session reads the new login on its next request" (SPEC section 5; internals.md section 5; CL 2.1.178 "credentials were refreshed outside the session"; CL 2.1.271 "the policy not refreshing until the hourly check when the credential changes mid-session").
- So a request already streaming finishes on the old account. In a long agentic turn, every later model call in that same turn (after each tool result) goes to the new account. Nothing is lost, but one turn is billed across two accounts.
- **Cache cost of any switch.** "Caches are isolated between organizations" (PC). Each Pro login is its own organization **(inferred)**, so the first request after a switch re-reads the whole conversation uncached on the new account. That happens whether the switch lands mid-turn or between turns. Each switch costs one full-context cache write, so flapping is expensive and hysteresis matters.
- **Refresh race.** Swapping while Claude Code refreshes is covered by ccam's lock protocol (SPEC section 5 invariant 4) and by freshening the target token (section 2.2 `FRESHEN_BUFFER_MS`). Nothing about timing relative to turns changes that.
- **Background work.** `Stop` fires for the main agent, but `background_tasks` may still be in flight (H#stop input). A "between turns" switch is not guaranteed request-free when background subagents run.

**Is between turns safer?** It is marginally cleaner: the turn's cache-hit stream stays on one account and no request races the swap. It is not required for correctness. The real trade-off is overshoot. At a 98% threshold a heavy turn can pass 100% before `Stop` fires. Then the turn ends in `StopFailure:rate_limit` and Claude Code starts its wait-at-limit (section 4).

**Can `Stop` be the trigger instead of a timer?** Yes. It fires exactly once per completed main-agent turn and never when idle (H#stop). Registered as an exec-form `async: true` command hook it costs one short `ccam` process per turn and blocks nothing (internals.md 1.5). It carries no usage numbers, so pair it with the status line sample (numbers) and a single endpoint confirmation (authority).

## 4. Claude Code's own quota auto-resume and usage credits

- **Auto-continue** (IM "Wait for a usage limit to reset"; SR `autoContinueAtUsageLimit`; CL 2.1.234): on by default for claude.ai subscribers. When a limit stops a task, the session waits and at the reset "sends Claude a fixed prompt to pick the task up where it stopped".
  - It re-arms "at most twice in a row".
  - It does not start on its own for a reset more than 24 h away, which covers most weekly limits.
- **Early resume only on in-app actions.** "when you finish adding usage credits with `/usage-credits`, sign back in after `/upgrade`, or switch models with `/model` during the wait, Claude Code checks whether usage is available again ... It doesn't check after an upgrade or purchase you make in a browser on your own" (IM). **(inferred)** An external credential swap by ccam is the same kind of out-of-band change, so the wait keeps counting down to the *old* account's reset.
  - The wait ends when "You send a prompt: Claude Code runs your prompt instead of waiting", and that prompt goes out on the new login.
  - "You switch accounts with `/login`" also ends it, but ccam does not use `/login`.
  - Consequence for a phone user: after a switch that happens *after* a hard limit, they must send any message (for example "continue") or press Esc. ccam must say so in its log. It must not type into the tmux pane itself: that would be a non-human prompt (section 5).
- **Notification types** (H#notification): `quota_auto_resume_fired` (continued at reset or early), `quota_auto_resume_stale` (reset while the machine slept > ~30 min; waits for Enter), `quota_auto_resume_disabled` (wait ended without continuing). **(inferred)** They matter to ccam only as "a limit was hit earlier and the wait just ended", which confirms a missed threshold. They are useless as a trigger because they fire after the damage.
- **Usage credits / overage** (COST "Add usage credits to your subscription"; ERR): with usage credits on, a Pro/Max account can keep working past the plan limit on metered credits, and the prompt cache TTL "drops to five minutes once you're drawing on usage credits" (COST). **(inferred)** If a saved account has usage credits enabled, crossing its limit silently starts spending money instead of failing. ccam cannot see this in the `five_hour`/`seven_day` fields it reads. It should treat ≥ 100% as "stop using this account" regardless, and the docs should tell users that auto-switching and usage credits interact.
- **Server throttle vs plan limit.** "Server is temporarily limiting requests (not your usage limit)" is retried with backoff since 2.1.199 (ERR). **(inferred)** `StopFailure:rate_limit` can still fire for it after retries. Confirm a plan limit through the usage endpoint before switching on `StopFailure`.

## 5. Terms

**Quoted exactly (re-fetched today; same text as window-warmup.md section 5):**
- CT section 3, prohibited uses: "Except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it, to access the Services through automated or non-human means, whether through a bot, script, or otherwise."
- CT section 3: "You also must not abuse, harm, interfere with, or disrupt our Services, including, for example, introducing viruses or malware, spamming or DDoSing Services, or bypassing any of our systems or protective measures."
- CT section 2: "You may not share your Account login information, Anthropic API key, or Account credentials with anyone else. You also may not make your Account available to anyone else."
- CT section on changes: "We may sometimes add or remove features, increase or decrease capacity limits, offer new Services, or stop offering certain Services."
- AUP, "Do Not Abuse our Platform": "Coordinate malicious activity across multiple accounts to avoid detection or circumvent product guardrails or generating identical or similar inputs that otherwise violate our Usage Policy"; "Utilize automation in account creation or to engage in spammy behavior"; "Circumvent a ban through the use of a different account, such as the creation of a new account, use of an existing account, or providing access to a person or entity that was previously banned".
- LC: "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK." Also "OAuth authentication ... is designed to support ordinary use of Claude Code and other native Anthropic applications", and "Anthropic reserves the right to take measures to enforce these restrictions and may do so without prior notice."
- HC-LIM: "If you hit your usage limit, you'll need to wait for it to reset, upgrade your plan, or purchase usage credits." and "your usage of all different Claude product surfaces (claude.ai, Claude Code, Claude Desktop) counts towards the same usage limit." Per a fetch of the page today, it does not mention multiple accounts or subscriptions.
- HC log-in article (https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account): "Use of third-party tools that misrepresent their identity to Anthropic's servers, attempt to route third-party traffic against subscription limits, or otherwise violate applicable terms or policies is prohibited" (quoted in window-warmup.md section 4).

**Anthropic statements on multiple accounts or rotation, 2025-2026:**
- I found no first-party statement that addresses one person rotating several of their own subscriptions.
- July 2025 weekly limits: TechCrunch reports Anthropic targeting subscribers running Claude Code "continuously in the background, 24/7" and "sharing accounts and reselling access". The only verbatim spokesperson quote there is "Claude Code has experienced unprecedented demand since launch. Most users won't notice a difference." (https://techcrunch.com/2025/07/28/anthropic-unveils-new-rate-limits-to-curb-claude-code-power-users/). The motivations are reporter paraphrase, not a quote.
- April 2026 enforcement targeted third-party harnesses using subscription OAuth, not account counts (Boris Cherny, https://x.com/bcherny/status/2040206440556826908; window-warmup.md section 4).
- anthropics/claude-code issues:
  - [#64376](https://github.com/anthropics/claude-code/issues/64376) "Multiple subscription account" (switch accounts at the limit) was closed by the inactivity bot with no staff reply.
  - [#52784](https://github.com/anthropics/claude-code/issues/52784): a user with 4 Max accounts hitting server throttles. Bot-closed, no staff reply.
  - [#54464](https://github.com/anthropics/claude-code/issues/54464): usage appearing shared across two accounts. Bot-closed.
  - [#94195](https://github.com/anthropics/claude-code/issues/94195): open, switch to a second account showing the wrong tier.
  - [#90152](https://github.com/anthropics/claude-code/issues/90152): open, asks to pool limits between accounts, which confirms limits are "strictly per-account".
  - [#261](https://github.com/anthropics/claude-code/issues/261) (2025, closed completed): the `CLAUDE_CONFIG_DIR` workaround reply came from an account now shown as "ghost", so I can't confirm the author was Anthropic staff. The use case was work vs personal, not limits.
  - None of these issues contains an Anthropic statement about bans for rotation.
- Secondary sources claim "multiple accounts are fine" and that bans target OAuth in third-party harnesses (https://dev.to/vainamoinen/two-multi-account-claude-code-architectures-one-anthropic-accepts-one-they-ban-2om7, https://metricnexus.ai/blog/anthropic-banning-multiple-claude-accounts). **(third-party)** Neither cites a first-party text for the claim.

**How `ccam auto` differs in risk from manual `ccam to`:**
- *Same:* both run the unmodified Claude Code binary and move tokens between the user's own accounts on the user's own machine. Neither shares credentials with another person.
- *Automation.* `ccam to` is a human decision per switch, equivalent to `/logout` plus `/login`. `auto` is a script that performs account changes and polls an undocumented endpoint for days without a human in the loop. The CT automation clause is about "access[ing] the Services", and `auto` sends no prompts. **(inferred)** The unattended endpoint polling and credential swapping are still "a bot, script, or otherwise" acting on the accounts. Anthropic has not explicitly permitted them, unlike `CLAUDE_CODE_OAUTH_TOKEN` for scripts (window-warmup.md section 5).
- *Purpose.* A manual switch has many reasons. A switch fired by utilization ≥ 98% has one: keep working past a per-account limit. That is the reading most exposed to "bypassing any of our systems or protective measures" and to LC's "ordinary, individual usage". The Help Center lists wait, upgrade, or buy credits as the answers to a limit, not switching.
- *Signal.* **(inferred)** One machine and IP where account A stops at 98-99% and account B starts within seconds, repeating every window, is a distinctive server-side pattern. Manual switching is irregular.
- *Duration.* A multi-day loop that keeps agentic work running across windows resembles the "continuously in the background, 24/7" usage the 2025 limits were reported to target (TechCrunch paraphrase).
- ccam's own `docs/SPEC.md` section 1 lists this feature as a non-goal for exactly this reason. Implementing `auto` means changing that non-goal on purpose and saying so in the README.
- Only Anthropic can say whether this is allowed. This is a reading of public text, not legal advice.

## 6. Design recommendation: the lightest robust loop

### 6.1 Trigger: event-driven, endpoint-confirmed

Candidate A, adaptive polling only (the claude-swap model):
- Needs no hooks.
- Costs about 12-20 requests per hour per account while active, and cannot tell idle from busy except through movement.
- Switches mid-turn.

Candidate B, event-driven (recommended):
1. `ccam setup --auto` installs, idempotently and in exec form:
   - **Status line wrapper** `ccam hook statusline`. It reads stdin JSON and, when `rate_limits` is present, atomically writes `<vault>/auto/sample.json` = `{ts, session_id, five_hour{pct,resets_at}, seven_day{...}}`, and only when the values changed. Then it runs the user's previous `statusLine` command, saved at setup, with the same stdin, and prints its output. With no previous command it prints a short `ccam` segment. Never set `refreshInterval`.
   - **`Stop`** hook (async): touches `<vault>/auto/turn-end` with `{ts, session_id, background_tasks: n}`.
   - **`StopFailure`** matcher `rate_limit` (async): writes `<vault>/auto/limit-hit`.
2. `ccam auto` sleeps and `stat`s those three files every 5 s. That is local only, a few syscalls, and needs no inotify crate.
3. A new sample at or above `arm` (default 90%) → **arm**: refresh the candidate cache (6.3).
4. A sample at or above `threshold` (default 98%) → **pending**. At the next `turn-end` newer than the sample, confirm with **one** `GET /api/oauth/usage` for the live account. If still at or above the threshold, switch.
5. `limit-hit`, or a confirmed ≥ 100% → confirm with one request, then switch immediately without waiting for `Stop`. Log "send any message to continue", because Claude Code's own wait ignores external swaps (section 4).
6. Pending for more than 10 min with no `turn-end` (a very long turn): confirm and switch anyway.

Why B: with no Claude Code activity it makes **zero network requests**. Decisions still rest on the authoritative, identity-bound endpoint, never on an unattributed sample (section 1 caveat).

**Backstop poll**, to cover usage from claude.ai, Desktop, VS Code or `-p` that emits no samples. Live account only:
- Interval by the last known utilization `u` of the binding window: `u < 50`: 30 min; `50 ≤ u < 80`: 15 min; `80 ≤ u < arm`: 5 min; armed: 2 min, never below 60 s.
- When two consecutive polls show < 1 point of movement, double the interval up to 30 min.
- Always poll once at a known `resets_at` + 60 s.
- Idle worst case is 2 requests per hour. The ceiling is 30 per hour only while burning in the armed band, and that band ends in a switch.
- The backstop can be disabled (`--no-poll`) for users who only use Claude Code.

**429 handling:**
- Keep `Retry-After`, and wait `Retry-After + 900 s` (cap 4500 s) when it is > 0.
- On `Retry-After: 0`, wait at least 300 s.
- After any 429, floor all intervals at 360 s for an hour (numbers from CS, section 2.1).
- A 429 never counts as "limit reached". Keep the last-good value, which stays valid until its window resets (CS `usage_store.py` monotonic argument).

### 6.2 Target scoring

Per candidate `c`, from the cache, for each window `w` in {5h, 7d} (plus configured per-model limits):
- `u_w` = cached pct, or **0 if cached `resets_at` ≤ now**. The window rolled over, so this is known without a fetch **(inferred from monotonicity, CS `usage_store.py`)**.
- `h_w = 100 - u_w`; `H = min_w h_w` (binding headroom); `r` = `resets_at` of the binding window.

Eligible if:
- not quarantined, and the token is alive or refreshable (SPEC section 5 rules);
- cache age ≤ 60 min, or the window rolled over since;
- `u_5h < threshold - 20` and `u_7d < threshold`;
- `H ≥ H_active + 20` (the hysteresis, twice claude-swap's 10 because each switch costs a full cache rebuild);
- and it is not the account left by the last automatic switch, unless that account's binding window has reset since.

Score (ascending sort key):

```text
key(c) = ( -floor(H / 10),           # headroom bucket: 10-point bands, more is better
           r_7d(c),                  # within a band, weekly window resetting soonest first (perishable)
           r_5h(c) or +inf )         # then the 5h window resetting soonest
```

Headroom dominates. Reset time breaks near-ties, which is what "prefer by remaining 5-hour and weekly headroom and by how soon windows reset" asks for without letting a nearly empty account win on reset alone. An account with no running 5-hour window (`resets_at` null) has `h_5h = 100` and sorts well on headroom, which is correct.

**Fresh check at switch time:** if the chosen target's cache is older than 5 min, fetch that one account (1 request), re-score, and freshen its token (SPEC `usage --refresh` path, refresh if it expires within 10 min).

### 6.3 Cache refresh policy
- Unarmed: **no candidate requests at all**, only the rollover trick above.
- On arming: fetch every candidate once, sequentially with 2 s spacing, not concurrently.
- While armed: re-fetch the single stalest candidate at most every 10 min.
- At switch: the targeted refresh (6.2).
- An exhausted candidate is re-polled no sooner than its `resets_at` + 60 s, or every 30 min if unknown.
- With N accounts that is N requests per arm episode plus about 6 per hour while armed, spread across N identities, so far under the ~28-30/hour/identity measured budget.

### 6.4 Hysteresis and cooldowns
- The `arm` level (90) sits below the `threshold` (98). Disarm only when the live binding utilization drops below `arm - 10`, meaning a reset or a switch happened.
- Cooldown: 15 min between automatic switches, bypassed only for a confirmed ≥ 100% or `limit-hit`.
- No-return: never switch back to the account just left until its binding window has reset.
- Circuit breaker: at most `N_accounts` automatic switches per rolling 5 h. When exceeded, stop switching, log `breaker open`, and wait for the user.

### 6.5 When every account is near its limit
- If no candidate is eligible, do not switch at the threshold. Stay, so no cache is rebuilt for nothing.
- At the hard limit (`limit-hit` or confirmed ≥ 100%), move to the account whose binding window resets soonest, only if it is back at least 5 min sooner than the active account (claude-swap's `RECOVERY_HYSTERESIS_S` idea) and has H > 0.
- Otherwise log `all exhausted; next usable: <name> at HH:MM` and sleep until that reset + 60 s, still waking every 5 s for stop requests and events.
- Never keep re-probing exhausted accounts faster than every 30 min.

### 6.6 `ccam auto stop` and single instance
- `ccam auto` creates a lock directory `<vault>/auto.lock` with `mkdir`, like `src/lock.rs`. It contains `pid`, the process start time (Linux: `/proc/<pid>/stat` field 22), and the start timestamp. A second `ccam auto` refuses: `already running (pid N)`. A lock whose pid is dead, or whose start time does not match (PID reuse), is stale and gets replaced.
- `ccam auto stop` writes `<vault>/auto/stop` and waits up to 10 s for the lock to disappear. The loop sees the file on its next 5 s wake, finishes any switch in progress under the normal locks, removes both files, and exits 0. If it does not respond, `stop --force` sends SIGTERM on Unix (or `TerminateProcess` on Windows) after verifying pid and start time. The SIGTERM handler sets the same stop flag.
- Never kill during a switch: the switch runs under the vault and Claude Code locks, and the stop flag is only checked between steps.
- `auto` also takes no switch while a manual `ccam to` holds `.vault.lock`, which comes for free from the existing lock order.

### 6.7 Footprint
- One thread, `std::thread::sleep(5 s)`, `std::fs::metadata`, blocking `ureq`. No async runtime and no new crates.
- State in memory: a small struct per account. Persisted state goes to `<vault>/auto/state.json`: the cache, last switch, left account, breaker counters. It is written atomically only when it changes.
- **(inferred)** Expect about 2-5 MB RSS and ~0% CPU. Idle traffic is zero with `--no-poll`, otherwise ≤ 2 requests per hour.

### 6.8 What the phone user sees
- **stdout in tmux** and `switch.log` (append-only, no emails, no tokens), one line per state change, never per wake:

```text
14:02:11 armed      live=a 5h 91% 7d 40%  cache: b 5h 12% 7d 33%, c 5h 0% 7d 71%
14:31:40 pending    live=a 5h 98.2% (status line) waiting for turn end
14:32:05 switched   a -> b  trigger=threshold 5h 98.6% (confirmed)  b: 5h 12% 7d 33%
16:10:00 limit-hit  live=b; switched b -> c; send any message to continue
19:55:02 exhausted  next usable: a at 20:01
```

- **`ccam auto status`** (works over SSH from the phone): running or not, pid, uptime; live account with last confirmed and last sampled pct and their ages; armed or pending state; each candidate's cached 5h and 7d pct, `resets_at`, age, and eligibility; last switch (time, from, to, reason); cooldown and breaker state; requests made in the last hour.
- The status-line segment can show `ccam: a 91% → b` inside the Claude Code pane, where the user is already looking.

## Open questions to verify manually
1. Does a status-line sample update within one response after an in-place swap? Expected yes. Check that `used_percentage` changes to the new account's value.
2. Does `StopFailure:rate_limit` fire for the plan limit when `autoContinueAtUsageLimit` is on, before the wait starts?
3. Does the auto-resume continuation prompt at the reset go out on the swapped login? (It should: it is an ordinary request.)
4. Measure the real per-switch cache-rebuild cost once, from `/usage` or the status line `cost` fields.
