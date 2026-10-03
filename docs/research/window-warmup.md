# Starting every account's 5-hour window at once

Researched 2026-09-19. Local checks ran against Claude Code **2.1.278** (`claude --version`; the brief said 2.1.274, the binary has since updated). No prompt was sent to any account. The only local run was `claude auth status --json` with a dummy token in a throwaway config dir. Every claim has a URL. **(inferred)** marks my reading, **(third-party)** marks a non-Anthropic source, **(local)** marks something checked on this machine.

Doc URLs used below:
- A = https://code.claude.com/docs/en/authentication
- E = https://code.claude.com/docs/en/env-vars
- CLI = https://code.claude.com/docs/en/cli-reference
- HL = https://code.claude.com/docs/en/headless
- SL = https://code.claude.com/docs/en/statusline
- LC = https://code.claude.com/docs/en/legal-and-compliance
- CO = https://code.claude.com/docs/en/costs
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- CT = https://www.anthropic.com/legal/consumer-terms (effective October 8, 2025)
- AUP = https://www.anthropic.com/legal/aup (effective September 15, 2025)
- HC-SDK = https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- HC-LOGIN = https://support.claude.com/en/articles/13189465-log-in-to-your-claude-account

---

## 1. When the window starts and resets

**What Anthropic documents today**
- "Your session-based usage limit will reset every five hours." (https://support.claude.com/en/articles/8325606-what-is-the-pro-plan, https://support.claude.com/en/articles/11049741-what-is-the-max-plan)
- Settings > Usage shows "Current session: How much of your plan's five-hour session limit you've used thus far, plus the amount of time remaining in the session." (https://support.claude.com/en/articles/9797557-usage-limit-best-practices)
- The limit message is "5-hour limit reached - resets [time]" (https://support.claude.com/en/articles/12466728-troubleshoot-claude-error-messages).
- None of the current Help Center articles says what starts a session. The retired Max plan article (https://support.anthropic.com/en/articles/11014257-about-claude-max-plan-usage, now 404) said the 5-hour "sessions ... start with your first message to Claude". That wording survives only in secondary sources such as https://portkey.ai/blog/claude-code-limits/. **(inferred)** "starts at the first message" is still the working model, but no live primary source states it.

**Anchor precision (third-party)**
- The window used to be floored to the clock hour. Since about April 2026 it starts "at the exact minute of your first message" (https://github.com/vdsmon/claude-warmup README, "Edit (2026-04-30)").
- A message sent inside a running window counts toward that window and does not start a new one. That is why https://github.com/viseshb/claude-codex-warmer checks the usage meter and skips accounts that already have a window.
- A request made while the account is rate-limited "still anchors the window" (vdsmon README, FAQ).

**Which surfaces count**
- One pool across surfaces: "your usage of all different Claude product surfaces (claude.ai, Claude Code, Claude Desktop) counts towards the same usage limit" (https://support.claude.com/en/articles/11647753-how-do-usage-and-length-limits-work). Also: "usage limits that are shared across Claude and Claude Code" (https://support.claude.com/en/articles/11145838-use-claude-code-with-your-pro-or-max-plan).
- Headless use counts today. As of June 15, 2026: "We're pausing the changes to Claude Agent SDK usage ... For now, nothing has changed: Claude Agent SDK, `claude -p`, and third-party app usage still draw from your subscription's usage limits" (HC-SDK).
- **This is a pending risk.** The paused plan would move "`claude -p` command in Claude Code (non-interactive mode)" to a separate monthly credit, while "Interactive Claude Code in the terminal or IDE" and the web, desktop and mobile apps stay on subscription limits (HC-SDK). **(inferred)** If that change ships, a `claude -p` warmup may stop opening the 5-hour window at all. `ccam start` must therefore check that a window actually opened (section 7).
- **(inferred)** Model and message size do not matter for starting the window: any accepted request should anchor it. The community tools all anchor with a single Haiku "hi" (vdsmon, viseshb, https://github.com/SigmaGrindset/session-scheduler).

**Querying the window**
- **Documented:** the status line JSON has `rate_limits.five_hour.resets_at` in "Unix epoch seconds when the 5-hour ... rate limit window resets". It appears "only for claude.ai Pro and Max subscribers ... and only after the first API response in the session", and "Claude Code drops a window once its `resets_at` time passes" (SL). `/usage` shows the same thing interactively. **(inferred)** Window start = `resets_at` minus 5 h.
- **Undocumented:** `GET https://api.anthropic.com/api/oauth/usage` returns `five_hour{utilization,resets_at}`. Claude Code calls it itself, and ccam already uses it (`docs/SPEC.md` section 7; claude-swap `oauth.py`, https://github.com/realiti4/claude-swap). claude-swap handles `resets_at: null` and `utilization: null` as normal shapes (`autoswitch.py`). **(inferred)** `resets_at` is null or past when no window is running. This is the only way to query without spending a message.
- Also undocumented: the `anthropic-ratelimit-unified-5h-reset` response header on `/v1/messages` (SigmaGrindset README).

## 2. Minimal-context headless invocation

All flags below are confirmed by `claude --help` on 2.1.278 **(local)** and by CLI.

| Flag / variable | Effect | Source |
|---|---|---|
| `-p "<prompt>"` | Non-interactive, one response, then exit | CLI |
| `--model haiku` | `haiku` is a documented alias | CLI `--model`; https://code.claude.com/docs/en/model-config |
| `--system-prompt "<text>"` | "Replaces the entire default prompt", which drops tool guidance and the rest of the default text | CLI "System prompt flags" |
| `--tools ""` | "Use `""` to disable all" built-in tools. It "doesn't affect MCP tools" | CLI |
| `--strict-mcp-config` (with no `--mcp-config`) | "Only use MCP servers from `--mcp-config`, ignoring all other MCP configurations", so no MCP servers load | CLI |
| `--disable-slash-commands` | "Disable all skills and commands for this session" | CLI |
| `--max-turns 1` | Caps agentic turns (print mode) | CLI |
| `--no-session-persistence` | "sessions are not saved to disk and cannot be resumed. Print mode only." `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` does the same in any mode | CLI, E |
| `--output-format json` | The result includes `total_cost_usd` and a per-model breakdown, which lets ccam measure the real cost | HL |
| `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` | "prevent loading any CLAUDE.md memory files into context, including user, project, and auto memory files" | E |
| `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` | Disables auto memory | E |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` | No auto-update, telemetry, error reporting, or availability checks | E |

**Rejected**
- **`--bare`.** It would be the ideal minimal mode, but "Bare mode does not read `CLAUDE_CODE_OAUTH_TOKEN`" and "never reads OAuth credentials or the system keychain" (A, HL). The local help says the same: "Anthropic auth is strictly ANTHROPIC_API_KEY or apiKeyHelper". **Risk:** HL says `--bare` "will become the default for `-p` in a future release". If that happens, `claude -p` with a subscription login may need an opt-out flag. Re-check each release.
- **`--safe-mode`.** It keeps the built-in tools and the default prompt (CLI), so it cuts less than the flags above.
- **`--setting-sources`.** Not needed once the config dir and working directory are throwaway (section 3). In the real config dir it has had side effects, for example "`--setting-sources` without `user` causing background cleanup to ... delete conversation history older than 30 days" (CL, fixed).
- **`--exclude-dynamic-system-prompt-sections`.** "ignored when `--system-prompt` ... is set" (CLI).
- **No output-token cap flag exists** (`--help`). A system prompt such as "Reply with one word." keeps the output short.

**Hidden context that remains (inferred)**
- claude.ai connectors are not fetched with an env token. The docs say the token "can't ... fetch claude.ai connectors" (A).
- Background summarisation for `--resume` is a documented idle cost (CO). `--no-session-persistence` should avoid it.
- **(inferred, unverified)** Claude Code probably still sends its own identity block with a custom `--system-prompt`. That is a few dozen tokens, not the ~4.5k of the default prompt.

**Token cost**
- **Measured by a third party with the default prompt:** "each about 4,500 tokens (almost all of it the CLI's own system prompt)" for `claude -p "hi"` on Haiku (https://github.com/viseshb/claude-codex-warmer README).
- **With the flags above: not measured here**, because the brief forbids sending a prompt. ccam should log `total_cost_usd` and usage from the JSON result on the first real run. **(inferred)** That run will come in well under 1 % of a Pro window.

## 3. Per-account auth without switching

**`CLAUDE_CODE_OAUTH_TOKEN`**
- It is an "OAuth access token for claude.ai authentication ... Takes precedence over keychain-stored credentials ... Claude Code uses the token you set for the whole session. To replace an expired token, generate a new one and restart" (E).
- It ranks 5th in the precedence list. The only sources above it are the cloud-provider switches, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY` and `apiKeyHelper` (A, "Authentication precedence"). **(inferred)** ccam must remove those from the child's environment, or they win over the token.
- **(local)** In an empty `CLAUDE_CONFIG_DIR` with a dummy token, `claude auth status --json` reported `"loggedIn": true, "authMethod": "oauth_token"`. It wrote only `.claude.json` (keys `firstStartTime`, `firstStartVersion`, `machineID`, `migrationVersion`, and a few migration flags) plus `backups/.claude.json.backup.*`. It wrote **no `.credentials.json` and no `oauthAccount`**.
- The docs describe the token as coming from `claude setup-token` (A). **(inferred)** An ordinary `/login` access token has the same `sk-ant-oat01` form and a superset of scopes (`user:inference user:profile user:sessions:claude_code`, from E `CLAUDE_CODE_OAUTH_SCOPES`), so it should work too. Verify manually once.
- **Do not combine the env token with the real config dir.** CL 2.1.225: "Fixed a transient 401 replacing a long-lived `CLAUDE_CODE_OAUTH_TOKEN` with a stored login's short-lived token". So Claude Code does reach for the stored login in some paths. **(inferred)** That could refresh, and so rotate, the live refresh token. A throwaway config dir with no `.credentials.json` makes that path impossible.
- **Expired tokens.** An expired env token is not refreshed. The docs say "generate a new one and restart" (E), and CL 2.1.117 fixed "`/login` having no effect when launched with `CLAUDE_CODE_OAUTH_TOKEN` env var and that token expires". So ccam must pass an unexpired token. For saved, non-live accounts it can reuse its existing `usage --refresh` path: refresh under the vault lock, and save the new generation before anything else uses it (SPEC section 5). SPEC section 7 says the live account is never refreshed by ccam, so if the live token is expired, skip that account and say why.
- CI tools do exactly this with a fresh home. The vdsmon workflow also writes `{"hasCompletedOnboarding":true}` to `~/.claude.json` before `claude -p` (https://github.com/vdsmon/claude-warmup/blob/main/.github/workflows/warmup.yml). Issue #8938 reports the onboarding flow appearing with only the env token (https://github.com/anthropics/claude-code/issues/8938). **(inferred)** Writing that key into the temp `.claude.json` is cheap insurance.

**Alternative: a throwaway `CLAUDE_CONFIG_DIR` holding the account's full `.credentials.json`**
- Claude Code would then refresh reactively on 401 or expiry (CL 2.1.117 "the token is now refreshed reactively on 401"; `docs/research/internals.md` section 3).
- Refresh tokens are single-use (internals.md section 3.1). A refresh would leave the only valid refresh token in the temp dir. ccam would have to read it back, check the generation, and save it before deleting the dir. A crash in between loses the account's login.
- On macOS the login would go into a Keychain item named after `sha256(CLAUDE_CONFIG_DIR)` (internals.md section 2.2), leaving an orphaned Keychain entry for every run.
- **Rejected:** the env-token path needs none of this.

## 4. Calling `/v1/messages` directly with the OAuth token

- **Legal and compliance page:** "OAuth authentication ... is designed to support ordinary use of Claude Code and other native Anthropic applications." Also: "Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, or Max plan credentials on behalf of their users. Moreover, developers may not collect, store, or intermediate Claude.ai credentials or session tokens", and "Anthropic reserves the right to take measures to enforce these restrictions and may do so without prior notice" (LC).
- **Help Center:** "Use of third-party tools that misrepresent their identity to Anthropic's servers, attempt to route third-party traffic against subscription limits, or otherwise violate applicable terms or policies is prohibited." It also says Anthropic "reserves the right to draw use of such third-party tools from usage credits rather than subscription limits" (HC-LOGIN).
- **Agent SDK docs:** "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products" (https://code.claude.com/docs/en/agent-sdk/overview).
- **Boris Cherny (Claude Code lead at Anthropic), April 2026:** "Starting tomorrow at 12pm PT, Claude subscriptions will no longer cover usage on third-party tools like OpenClaw. You can still use these tools with your Claude login via extra usage bundles ... or with a Claude API key." (https://x.com/bcherny/status/2040206440556826908)
- **Server-side enforcement.** Third-party clients get "This credential is only authorized for use with Claude Code and cannot be used for other API requests." (https://github.com/anomalyco/opencode/issues/10982, https://github.com/openclaw/openclaw/issues/559).
- tappress/claude-code-warmup did the direct call by sending `anthropic-beta: claude-code-20250219,oauth-2025-04-20` with a Haiku request (https://github.com/tappress/claude-code-warmup/blob/HEAD/api/warmup.ts). **(inferred)** The `claude-code` beta header presents the caller as Claude Code, which is the "misrepresent their identity" case above. Traffic classified as third-party may also be billed to usage credits and never touch the 5-hour window, which would defeat the purpose.
- The requirement that the system prompt start with Claude Code's identity string is widely reported but not documented anywhere I found in Anthropic sources. Treat it as unknown.
- **Recommendation: against.** Run the unmodified `claude` binary. The legal page's allowance is for "an end user ... signing in to the unmodified Claude Code binary with their own Claude subscription" (LC).

## 5. Terms

**Exact text that could apply**
- **CT section 3:** "You may not access or use, or help another person to access or use, our Services in the following ways: ... Except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it, to access the Services through automated or non-human means, whether through a bot, script, or otherwise."
- **CT section 3:** "You also must not abuse, harm, interfere with, or disrupt our Services, including, for example, ... bypassing any of our systems or protective measures."
- **CT section 2:** "You may not share your Account login information, Anthropic API key, or Account credentials with anyone else. You also may not make your Account available to anyone else."
- **AUP:** "Coordinate malicious activity across multiple accounts to avoid detection or circumvent product guardrails or generating identical or similar inputs that otherwise violate our Usage Policy"; "Circumvent a ban through the use of a different account".
- **LC:** "Advertised usage limits for Pro and Max plans assume ordinary, individual usage of Claude Code and the Agent SDK."

**What the text does and does not say**
- No clause mentions when a 5-hour window starts, or pre-starting one. Nothing I found in CT or AUP forbids one person holding several subscriptions. I searched CT for "multiple accounts" and found no match.
- **Automation is carved out when Anthropic "explicitly permit[s] it".** The docs present `CLAUDE_CODE_OAUTH_TOKEN` as the way to authenticate "CI pipelines and scripts" with a subscription (A), and HC-SDK lists `claude -p` as drawing on subscription limits. **(inferred)** A script that runs the official `claude -p` is inside that permission. A script that calls the API directly with the token is not (section 4).
- **The AUP multi-account clause is qualified by "malicious" and "product guardrails".** **(inferred)** Shifting reset times does not obviously meet that bar, but "circumvent" and "bypassing any of our systems or protective measures" are broad and Anthropic interprets them.
- Warmup adds no capacity to any account. It only moves the reset time. The community tools (section 6) describe it that way, and Anthropic has not published any statement against it that I found.
- **The gray area is LC's "ordinary, individual usage".** A one-command fan-out across several paid accounts is closer to the capacity pooling that ccam's SPEC already lists as a non-goal (SPEC section 1) than a single user's cron is. Keep it user-triggered, one tiny request per account, and never on a timer by default.
- This is a reading of public text, not legal advice.

## 6. Existing tools

| Tool | Mechanism | Notes |
|---|---|---|
| vdsmon/claude-warmup (173 stars, 2026-03) | GitHub Actions cron: `claude -p "hi" --model haiku --no-session-persistence` with `CLAUDE_CODE_OAUTH_TOKEN` from `setup-token`; writes `hasCompletedOnboarding` | Treats a rate-limit reply as success. Suggests Claude Code on the web scheduled tasks as a "native" alternative. https://github.com/vdsmon/claude-warmup |
| teerayuthj/claude-warmup, eromoe/claude-warmup (fork) | Same as vdsmon | https://github.com/teerayuthj/claude-warmup |
| viseshb/claude-codex-warmer (2026-07) | Cron every 30 min. Reads the account's usage meter, sends one Haiku "hi" only when no window is running, so windows chain back to back | Cites ~4,500 tokens per send from the default prompt. https://github.com/viseshb/claude-codex-warmer |
| SigmaGrindset/session-scheduler (2026-07) | Actions workflow with `claude -p`. Reads the undocumented `anthropic-ratelimit-unified-5h-reset` header afterwards | https://github.com/SigmaGrindset/session-scheduler |
| tappress/claude-code-warmup (2026-02) | Vercel cron, direct `POST /v1/messages` with the OAuth token and the `claude-code` beta header | Predates the April 2026 enforcement; see section 4. https://github.com/tappress/claude-code-warmup |

None of them handles several accounts from one machine, and none avoids the default system prompt.

---

## Recommended design for `ccam start`

**For each saved account (the live one included), concurrently**
1. **Resolve an unexpired access token.**
   - A saved account whose token has expired is renewed through the existing `usage --refresh` rules: vault lock, new generation saved first.
   - For the live account, read the live token. Never refresh it. If it is expired, report `skipped: live login expired, use Claude Code once`.
2. **Ask `GET /api/oauth/usage`.** If `five_hour.resets_at` is in the future, report `already running, resets HH:MM` and skip. This keeps the command idempotent and never wastes a send.
3. **Make a throwaway dir** `T` (owner-only) with `T/.claude.json` = `{"hasCompletedOnboarding":true}`. Use a separate empty dir `W` as the working directory.
4. **Run, with stdin null, a 60 s timeout, cwd `W`, and a scrubbed environment:**

```text
CLAUDE_CONFIG_DIR=<T>
CLAUDE_CODE_OAUTH_TOKEN=<account access token>
CLAUDE_CODE_DISABLE_CLAUDE_MDS=1
CLAUDE_CODE_DISABLE_AUTO_MEMORY=1
CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1
claude -p "hi" --model haiku --system-prompt "Reply with one word." --tools "" --strict-mcp-config --disable-slash-commands --max-turns 1 --no-session-persistence --output-format json
```

   - Remove from the child's environment: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL`, `ANTHROPIC_DEFAULT_HAIKU_MODEL`, `ANTHROPIC_PROFILE`, `ANTHROPIC_FEDERATION_RULE_ID`, `ANTHROPIC_ORGANIZATION_ID`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `CLAUDE_CODE_USE_FOUNDRY`, `CLAUDE_CODE_OAUTH_REFRESH_TOKEN`, `CLAUDE_CODE_SIMPLE`, `CLAUDE_SECURESTORAGE_CONFIG_DIR`.
   - Pass the token only through the environment, never argv. Pass `""` as its own argument; Rust's `Command` quotes it correctly on Windows.
5. **Read `is_error` and `total_cost_usd` from the JSON.** Delete `T` and `W`.
6. **Prove it:** re-read `/api/oauth/usage`. Report `started, resets HH:MM` only when `five_hour.resets_at` is about now + 5 h. Otherwise report `sent, but no window opened`. That is the signal that `claude -p` has moved to a separate credit pool (HC-SDK).

**Output** is one line per account: name, outcome, reset time. No email and no token. Exit 3 if any account failed.

**Manual tests to add to `docs/MANUAL-TESTS.md`**
- A `/login` access token (not a `setup-token` token) works as `CLAUDE_CODE_OAUTH_TOKEN`.
- Nothing under `~/.claude`, `~/.claude.json` or the live `.credentials.json` changes: compare hashes and mtimes before and after.
- Record `total_cost_usd` and input tokens from the first run.
- The window opens: `resets_at` is about now + 5 h.

**Risks**
1. **Headless moves to a separate credit.** If Anthropic reinstates the paused change, `claude -p` may stop opening the 5-hour window (HC-SDK). Step 6 detects it. The fallback is the user's current manual "hi" on claude.ai.
2. **`--bare` becomes the default for `-p`.** It ignores OAuth (HL), and the command would need an opt-out flag.
3. **Undocumented endpoint.** `/api/oauth/usage` is undocumented, and so are its `resets_at` semantics (null when idle is inferred). It is used only to skip and to verify, never to decide auth.
4. **The token sits in a child's environment.** It is visible to same-user processes for a few seconds. **(inferred)** This is acceptable on a single-user machine, and it is how Anthropic documents CI use (A).
5. **Refreshing saved accounts rotates refresh tokens.** Use only the existing vault-locked path. Never let Claude Code refresh in a temp dir.
6. **Terms gray area.** CT's automation clause (carved out for explicitly permitted use), CT's "bypassing ... protective measures", and LC's "ordinary, individual usage". Keep `start` manual and one message per account. Do not add a scheduler or tie it to limit-hit switching.
7. **The legal page says developers "may not collect, store, or intermediate Claude.ai credentials" (LC).** This applies to ccam's vault in general, not just `start`. **(inferred)** Its target is products serving other users; ccam stores only its own user's logins locally.
8. **Unmeasured cost.** Cost with the minimal flags is unmeasured until the first run. The default-prompt baseline is ~4.5k tokens (third-party).
9. **Managed settings still apply.** Policy settings, including `apiKeyHelper` or `forceLoginMethod`, can override the env token on managed machines (A).
