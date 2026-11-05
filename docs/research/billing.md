# Plan and billing information per account

Researched 2026-10-30 against Claude Code 2.1.314 (local install, Windows 11). Question: with the same OAuth access token ccam already holds for each saved login, which plan and billing facts can ccam show (plan, Max 5x vs 20x, renewal date, billing period, subscription start, cancellation, trial, extra usage, payment status)?

Conventions as in `internals.md`: **(local)** means checked on this machine; **(binary)** means read from the Claude Code 2.1.314 executable with fixed-string searches; **(live)** means one read-only GET with the live login's token, printing key paths and value types only, never values; **(inferred)** is my reading, not stated by a source. None of these endpoints are public API.

Sources used below:
- COSTS = https://code.claude.com/docs/en/costs.md ("Check your usage-credits spend", "Add usage credits to your subscription")
- UC = https://support.claude.com/en/articles/12429409 (usage credits for paid plans)
- BILL = https://support.claude.com/en/articles/8325618 (paid plan billing FAQs)
- CL = https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md (at b3f291d131, top entry 2.1.314)
- CB = https://github.com/steipete/CodexBar/blob/main/docs/claude.md (at 5f93dc3068) and `Sources/CodexBarCore/Providers/Claude/ClaudeWeb/ClaudeSubscriptionMetadata.swift`
- CS = https://github.com/realiti4/claude-swap/blob/HEAD/src/claude_swap/oauth.py (at 72ccd58612)

---

## 1. Where the data lives

### 1.1 Already on disk for every saved account (no network)
- **(local)** `claudeAiOauth` in `.credentials.json` has `subscriptionType`, `subscriptionTypeSource` and `rateLimitTier` beside the tokens. ccam keeps the whole object (`OAuthLogin.raw`, `src/login.rs`), so every saved login already carries both.
- **(local)** `oauthAccount` in `~/.claude.json` has these keys: `accountCreatedAt`, `accountUuid`, `billingType`, `ccOnboardingFlags`, `claudeCodeTrialDurationDays`, `claudeCodeTrialEndsAt`, `displayName`, `emailAddress`, `fullName`, `hasExtraUsageEnabled`, `organizationName`, `organizationRateLimitTier`, `organizationRole`, `organizationType`, `organizationUuid`, `profileFetchedAt`, `seatTier`, `subscriptionCreatedAt`, `userRateLimitTier`, `workspaceRole`. ccam's vault stores each account's `oauthAccount` (`Account.oauth_account`, `src/manager.rs`), so these are a snapshot from the time of the last save or sync. **(inferred)** They go stale when the plan changes until Claude Code refetches the profile and ccam syncs again.

### 1.2 `GET https://api.anthropic.com/api/oauth/profile` (ccam already calls it)
**(live)** Response shape with this machine's Max login, types only:

| Path | Type |
|---|---|
| `account.uuid`, `account.full_name`, `account.display_name`, `account.email`, `account.created_at` | string |
| `account.has_claude_max`, `account.has_claude_pro` | bool |
| `organization.uuid`, `organization.name` | string |
| `organization.organization_type` | string |
| `organization.billing_type` | string |
| `organization.rate_limit_tier` | string |
| `organization.seat_tier` | null (string on Team/Enterprise, inferred) |
| `organization.has_extra_usage_enabled` | bool |
| `organization.subscription_status` | string |
| `organization.subscription_created_at` | string |
| `organization.claude_code_trial_ends_at`, `organization.claude_code_trial_duration_days` | null |
| `organization.payment_auth_hosted_invoice_url` | null |
| `organization.cc_onboarding_flags` | object |
| `organization.claude_ai_completion_feedback_enabled`, `organization.claude_ai_safety_feedback_enabled` | bool |
| `application.uuid`, `application.name`, `application.slug` | string |
| `enabled_plugins` | array |

No renewal, next-charge, period-end, cancel or expiry field is present.

How Claude Code maps it **(binary)**:
- `subscriptionType` = `organization.organization_type` through a fixed map: `claude_max` -> `max`, `claude_pro` -> `pro`, `claude_enterprise` -> `enterprise`, `claude_team` -> `team`, anything else -> null.
- `rateLimitTier` = `organization.rate_limit_tier`. Claude Code compares it against `default_claude_max_5x` and `default_claude_max_20x` (a Team plan with `default_claude_max_5x` gets its own check, which reads as a premium seat (inferred), and the Max 20x tier changes the upgrade hint to `/usage-credits`). This is the only place Max 5x and Max 20x differ.
- Also copied into `oauthAccount`: `seatTier`, `hasExtraUsageEnabled`, `billingType`, `accountCreatedAt` (from `account.created_at`), `subscriptionCreatedAt` (from `organization.subscription_created_at`), `claudeCodeTrialEndsAt`, `claudeCodeTrialDurationDays`, `planDisplayName`.
- `planDisplayName` comes from `organization.plan_display_name`, validated against `^[\p{L}\p{N}](?:[\p{L}\p{N} .+-]{0,38}[\p{L}\p{N}+])?$`; when set, the plan name reads `Claude <planDisplayName>`. Otherwise it is "Claude Max", "Claude Pro", "Claude Team", "Claude Enterprise" or "Claude API". **(live)** The field was absent for this account.
- `billing_type` values Claude Code knows: `stripe_subscription`, `stripe_subscription_contracted`, `stripe_subscription_enterprise_self_serve`, `aws_marketplace`, `c4e_consumption_trial`, `apple_subscription`, `google_play_subscription` (the set that enables `/usage-credits`), plus `usage_based`. For `apple_subscription` and `google_play_subscription`, `/upgrade` says the plan is billed through the App Store or Google Play.
- `seatTier` value `enterprise_usage_based` marks usage-billed Enterprise.
- Claude Code does **not** read `subscription_status` or `payment_auth_hosted_invoice_url` (zero occurrences in the binary). Their values are undocumented. **(inferred)** `subscription_status` is a Stripe-style state (`active`, `past_due`, ...); issue https://github.com/anthropics/claude-code/issues/64480 shows `past_due` is a real account state that blocks Claude Code, and `payment_auth_hosted_invoice_url` looks like the Stripe hosted-invoice link set when a payment needs authentication.
- `/status` shows only "Login method: `<plan name>` account", Organization and Email from this data (binary, function building the `Login method` row).

### 1.3 `GET https://api.anthropic.com/api/oauth/usage` (ccam already calls it)
**(live)** Billing-relevant parts of the response, types only:
- `extra_usage`: `is_enabled` bool, `monthly_limit` number, `used_credits` number, `utilization` number, `currency` string, `decimal_places` number, `disabled_reason` string, `user_disabled` bool, `spend_limit_reached` bool, `credits_ever_enabled` bool, `daily` null, `weekly` null. No `resets_at`.
- `spend`: `used` and `limit` (`amount_minor`, `currency`, `exponent`), `percent`, `severity`, `enabled`, `disabled_reason`, `cap.money`, `cap.credits.amount_minor`, `cap.credits.exponent`, `balance` (null here), `auto_reload` (null here), `disclaimer` string, `can_purchase_credits` bool, `can_toggle` bool.
- `limits[]`: `kind`, `group`, `percent`, `severity`, `resets_at`, `scope`, `is_active`.
- Also `five_hour`, `seven_day` (each now with `limit_dollars`, `used_dollars`, `remaining_dollars`, `locked_reason`), many code-named nullable windows, `member_dashboard_available`, `seven_day_breakdown`.

What Claude Code does with it **(binary)**:
- Its schema for `extra_usage` is `is_enabled`, `monthly_limit` (nullable), `used_credits` (nullable), `utilization` (nullable), `currency`, `disabled_reason`, described as "Extra-usage (overage) spend for the billing period, when the plan has it. Amounts are in minor units of `currency` (cents for USD); is_enabled is false while extra usage cannot cover sends."
- `monthly_limit: null` is shown as "Unlimited" in the spend-limit dialog. COSTS confirms the `/usage` row: Pro and Max see "your spend for the current month, measured against your monthly spend limit when you have set one", or `Unlimited` without a limit.
- `disabled_reason` values handled include `out_of_credits`, `org_level_disabled_until`, `org_spend_cap_reached`, `overage_not_provisioned`.
- Claude Code never reads the `spend` block in 2.1.314 (`can_purchase_credits`, `credits_ever_enabled` have zero occurrences). Some of its reads add `skip_spend=1` (`/api/oauth/usage?cedar_ember=1&skip_spend=1`). **(inferred)** `spend` is a newer server-side block, probably for claude.ai, and costs the server extra work.
- A `cedar_ember` block (fetched with `?cedar_ember=1`, null in the plain read) carries `event_props` with `tier` (`claude_pro`, `claude_max_5x`, `claude_max_20x`, `claude_team`), `billing_path` (`stripe`, `apple`, `google_play`, `other`), `billing_period` (`monthly`, `annual`) and `extra_usage_state`. Claude Code uses it only for telemetry around limit resets. **(inferred)** It is gated per account, so it is no basis for a feature.

### 1.4 Other OAuth billing endpoints Claude Code calls (not called in this research)
**(binary)** Claude Code calls these on `api.anthropic.com` with auth mode `teleport-org`: the login's OAuth bearer token plus an `x-organization-uuid` header, with `:orgUUID` replaced by the org's uuid. So they accept the same token, at least for the scopes Claude Code holds (`user:profile`, `user:inference`, and others; local).
- `GET /api/oauth/organizations/:orgUUID/prepaid/credits`: prepaid usage-credit balance (`amount`, `currency`, `auto_reload_settings`, `expiry_policy_months`). Used by `/usage-credits` and `/ultrareview`.
- `GET /api/oauth/organizations/:orgUUID/payment_method`: the saved payment method, used before buying credits.
- `GET /api/oauth/organizations/:orgUUID/prepaid/bundles`: credit bundles for sale (`bundle_paid_this_month_minor_units`, `bundle_monthly_cap_minor_units`, `purchases_reset_at`).
- `PUT .../overage_spend_limit`, `POST .../setup_overage_billing`, `.../contracts/auto_reload_settings`, `POST .../contracts/prepaid/credits` (purchase), `.../billing/tax_rate`, `POST .../claude_code/pro_trial`, `.../overage_credit_grant`. These write or charge money. ccam must never call them.

None of these return a subscription renewal or period date.

## 2. Renewal date, billing period, cancellation: not reachable with the OAuth token

- Claude Code 2.1.314 contains no `next_billing`, `renews_at`, `current_period_end`, `cancel_at` or `subscription_details` string (binary, zero occurrences each). It never shows a renewal date. `/usage-credits` sends Pro and Max users to claude.ai **Settings > Usage** for balance and spend (COSTS).
- Feature requests for exactly this ("Expose subscription billing-cycle / renewal date in statusline JSON (or via CLI)"): https://github.com/anthropics/claude-code/issues/77441 (closed as inactive) and https://github.com/anthropics/claude-code/issues/95712 (open, opened 2026-09-20, no reply). They say the statusline has `rate_limits.five_hour` and `seven_day` "but no equivalent field for the monthly subscription billing cycle".
- Anthropic's help center places billing under claude.ai **Settings > Billing**, where you "confirm your recent payment status". It says "there isn't a direct way to change your subscription billing date", only cancelling and resubscribing (BILL). Mobile-app subscriptions are billed by the Apple App Store or Google Play (BILL).
- The renewal and expiry dates live on a claude.ai web endpoint authenticated by the browser `sessionKey` cookie: `GET https://claude.ai/api/organizations/{org_id}/subscription_details`, with fields `status` (`active`, `trialing`, `canceled`), `next_charge_at`, `next_charge_date`, `plan_ending_at`, `plan_ending_before` (CB, `ClaudeSubscriptionMetadata.swift`). CodexBar states: "OAuth credentials alone cannot fetch these dates." The same doc lists the cookie-only claude.ai routes it uses for spend and credits (`/api/organizations/{orgId}/overage_spend_limit`, `/prepaid/credits`).
- I did not call `subscription_details` with the OAuth token, as the research rules allowed only the profile and usage GETs. The evidence that it cannot be reached is CodexBar's statement plus Claude Code never calling it. Treat the question as settled unless someone tests it. **(inferred)** The OAuth billing routes in 1.4 sit under `/api/oauth/organizations/...` on `api.anthropic.com`, a separate route family from claude.ai's cookie routes. No OAuth twin of `subscription_details` appears in the binary.
- `subscription_created_at` is the only date ccam can get. **(inferred)** For monthly Stripe billing, the renewal day-of-month usually matches it. It breaks for annual billing, for App Store and Google Play billing, after a cancel and resubscribe (BILL), and possibly after a plan change. So it is no reliable renewal date.

## 3. What other tools do

- **CodexBar** (macOS menu bar app): plan from `subscriptionType`, falling back to `rate_limit_tier`, and "When a Max `rate_limit_tier` carries a usage multiplier (`default_claude_max_5x` / `default_claude_max_20x`), it is surfaced in the label as 'Max 5x' / 'Max 20x'". Renewal and expiry come only from the cookie route above (CB).
- **claude-swap**: reads `extra_usage` (`is_enabled`, `used_credits`, `monthly_limit`, `utilization`, `currency`) from the usage endpoint, divides by 100 and shows it as a spend line. It also looks for `extra_usage.resets_at` (CS, around line 486). **(live)** That key was absent here.
- **Claude-Code-Usage-Monitor**: the plan is a user flag, `--plan pro|max5|max20|team|custom`, with no detection (https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor README, options table). It reads `/api/oauth/usage` only for the five-hour and seven-day windows (`src/claude_monitor/output/api_usage.py`).
- **ccusage**: works from local JSONL transcripts ("5-Hour Blocks Report"); no plan or billing API (https://github.com/ryoppippi/ccusage README).

## 4. Relevant Claude Code changelog entries (CL)
- 2.1.69: "Fixed `.credentials.json` losing `subscriptionType` (showing 'Claude API' instead of 'Claude Pro'/'Claude Max') when the profile endpoint transiently fails during token refresh". So `subscriptionType` on disk can be missing after a transient profile failure.
- 2.1.30: "Fixed rate limit message showing incorrect '/upgrade' suggestion for Max 20x users when extra-usage is unavailable".
- 2.1.144: "Renamed 'extra usage' to 'usage credits' across CLI copy; `/extra-usage` is now `/usage-credits` (old name still works)".
- 2.1.248: "Added `/usage-credits` for Enterprise organizations billed through AWS Marketplace, self-serve Enterprise, and Enterprise trials".
- 2.1.284: "Fixed usage-limit warnings suggesting `/upgrade` to users already on the highest Max plan".
- 2.1.295: "Improved headless `rate_limit_event` usage-limit warnings to say whether the account has extra usage turned on".
- 2.1.311: "Fixed Claude Pro and Max subscribers being told to ask their admin when their subscription can't be used; the message now points to billing settings".

## 5. Summary for ccam

| Fact | Obtainable | Source | Reliability |
|---|---|---|---|
| Plan family (Pro, Max, Team, Enterprise) | yes | `subscriptionType` in the saved login; live `organization.organization_type` | good; can be missing (CL 2.1.69) |
| Max 5x vs Max 20x | yes | `rateLimitTier` in the saved login; live `organization.rate_limit_tier` (`default_claude_max_5x`, `default_claude_max_20x`) | good, same rule Claude Code and CodexBar use |
| Server plan label | sometimes | `organization.plan_display_name` | absent on this account |
| Team/Enterprise seat tier | yes | `organization.seat_tier` | null on personal plans |
| Billed through Stripe, App Store, Google Play, AWS | yes | `organization.billing_type` | values undocumented; list in 1.2 |
| Subscription start | yes | `organization.subscription_created_at` | meaning undocumented |
| Account creation | yes | `account.created_at` | good |
| Claude Code trial end | yes | `organization.claude_code_trial_ends_at` | null unless on a trial |
| Subscription status (active, past due) | probably | `organization.subscription_status` | values unverified, Claude Code ignores it |
| Payment needs action | probably | `organization.payment_auth_hosted_invoice_url` non-null | inferred meaning; a URL, never print it |
| Extra usage on, spent, limit, currency | yes | usage `extra_usage` | good; Claude Code relies on it |
| Prepaid credit balance | yes, extra call | `GET /api/oauth/organizations/:orgUUID/prepaid/credits` | Claude Code relies on it |
| Renewal / next billing date | **no** | claude.ai cookie route only | |
| Billing period start/end, monthly vs annual | **no** | (gated `cedar_ember` telemetry only) | |
| Cancel / expiry date | **no** | claude.ai cookie route only | |

### Recommendation for a `ccam usage` plan and billing addition
1. **Plan label, offline:** derive from the saved login's `subscriptionType` and `rateLimitTier`: `max` + `default_claude_max_20x` -> "Max 20x", `max` + `default_claude_max_5x` -> "Max 5x", else the capitalized type. This needs no new call and works for stale saved accounts. It replaces today's bare `pro`/`max` in the card border.
2. **Extra usage line, from the response ccam already fetches:** when `extra_usage.is_enabled` is true, show `used_credits` / `monthly_limit` in `currency` (minor units; divide by `10^decimal_places`, and by 100 when it is absent (inferred from Claude Code's "cents for USD" description)), or "Unlimited" when `monthly_limit` is null; show `disabled_reason` when it is off but was configured. Add it to `parse_windows`' sibling, not a new endpoint.
3. **Optional `--billing` detail, from the profile ccam already calls for identity:** subscription start (`subscription_created_at`), billing channel (`billing_type`, mapped to "Stripe", "App Store", "Google Play", "AWS Marketplace"), trial end when set, and a warning when `subscription_status` is not `active` or `payment_auth_hosted_invoice_url` is non-null. Before shipping the status warning, read the real `subscription_status` value on one account and record it in `internals.md`, since it is unverified here.
4. **Do not show a renewal date.** It cannot be read with the OAuth token, and estimating it from `subscription_created_at` is wrong for annual, app-store, and resubscribed plans. Point users to claude.ai Settings > Billing.
5. Keep to GETs on `profile` and `usage`. Never call the purchase, spend-limit, auto-reload or trial routes in 1.4. A prepaid-balance GET is possible, but it adds one request per account per run and only matters for accounts that buy credits.
6. Per AGENTS.md: mark the new fields *undocumented* in SPEC, add them to `internals.md` with version 2.1.314, and keep the invoice URL and emails out of output.

---

## Follow-up: --info (2026-10-30)

Question: what can a `ccam usage --info` flag show per account with the OAuth access token: exact plan, account and org name, subscription start and periods, billing channel, subscription status, trial, seat tier, extra usage, and guest passes. Same conventions and Claude Code version (2.1.314) as above. The live login on this machine is now a **Pro** account (`subscriptionType` `pro`, `rateLimitTier` `default_claude_ai`), not the Max login of the first pass. Three live GETs were made with its token (headers as `src/api.rs`: `Authorization: Bearer`, `anthropic-beta: oauth-2025-04-20`): one to the referral eligibility route, one to `profile`, one to `usage`. No POST, no invite created, no refresh.

Extra sources:
- GP1 = https://support.claude.com/en/articles/13456702-claude-code-guest-passes (linked from the 2.1.314 binary)
- GP2 = https://support.claude.com/en/articles/12875061-claude-code-guest-passes (older article, also linked from the binary)
- CMD = https://code.claude.com/docs/en/commands.md (`/passes` row)

### F.1 Guest passes and referrals

**Feature (docs).** GP1: "Guest passes are currently available to eligible Pro and Max subscribers only." Each pass gives the recipient "full access to Claude for seven days at no cost", after which it converts to paid Pro unless cancelled. "You have a limited number of guest passes that may expire or be withdrawn." The sender "sometimes" receives "a specified amount of overage credits" once the referee converts to paid; the credits expire one year after issue and are void on a downgrade to Free. Passes are found via `/passes` in Claude Code, the Claude Code tab of claude.ai settings, the Cowork tab in Claude Desktop, or Claude Design settings. GP2 is the earlier version of the same page and says Max only. CMD: "`/passes` | Share a free week of Claude Code with friends. Only visible if your account is eligible". CL 2.1.236: "Fixed spinner tips never appearing ... when the cached guest-pass reward in `~/.claude.json` was malformed".

**How Claude Code does it (binary).**
- `/passes` is a `local-jsx` command. Description "Share a free week of Claude Code with friends", plus " and earn usage credits" when a reward is cached. It is hidden unless the cached eligibility says `eligible` (`isHidden: !eligible || !hasCache`).
- Gate before any request: the login has an `organizationUuid`, is a claude.ai subscriber login, holds the `user:profile` scope, and `subscriptionType` is `max` or `pro`.
- `GET /api/oauth/organizations/:orgUUID/referral/eligibility?campaign=claude_code_guest_pass`, auth mode `teleport-org` (Bearer token, `anthropic-version: 2023-06-01`, `x-organization-uuid`), timeout 5 s. Read-only.
- `GET /api/oauth/organizations/:orgUUID/referral/redemptions?campaign=<campaign>`, same auth, timeout 10 s. Read-only. Claude Code reads only `redemptions` (array) and `limit` (default 3) and builds one "pass N available / used" row per slot: a slot is available when `redemptions[N]` is absent.
- Neither route is a write; Claude Code never creates an invite. Sharing is copying `referral_code_details.referral_link` to the clipboard ("Referral link copied to clipboard!").
- The eligibility response is cached in `~/.claude.json` under `passesEligibilityCache[<organizationUuid>]` with a `timestamp`; it is treated as stale after 24 h (`86400000` ms) and refetched in the background. Related keys: `hasVisitedPasses`, `passesUpsellSeenCount` (upsell shown at most 3 times), `passesLastSeenRemaining`. Zod schema for the reward: `{amount_minor_units: number, currency: /^[A-Za-z]{3}$/}`.

**Live, eligibility route (status 200), key paths and types:** `eligible` bool, `remaining_passes` int, `limit` int, `degraded` bool, `referrer_reward.amount_minor_units` int, `referrer_reward.currency` string, `referral_code_details.code` string, `referral_code_details.campaign` string, `referral_code_details.referral_link` string, `share_link` string, `terms_url` string. Values safe to record: `eligible` true, `limit` 3, `remaining_passes` 0, `degraded` false, reward 1000 minor units USD, campaign `claude_code_guest_pass` with a variant suffix. So a Pro account is eligible, as GP1 says.

**Local (no network).** `~/.claude.json` already holds 16 `passesEligibilityCache` entries on this machine, one per org Claude Code has been live on. Entries with `eligible: true` have the same shape as the live response plus `timestamp`; ineligible ones have `eligible: false` and every other field null. **(inferred)** Because ccam swaps the live login inside the same `~/.claude.json`, this cache covers every saved account that has run Claude Code here, so ccam can show passes offline, keyed by the saved account's `organizationUuid`, with the cache age from `timestamp`.

Never print `code`, `referral_link`, `share_link` or `terms_url`: the code and links are a bearer credential for the referral.

### F.2 Profile, second read

Live values for the Pro login (enum-like fields and dates only):

| Field | Value |
|---|---|
| `organization.organization_type` | `claude_pro` |
| `organization.billing_type` | `stripe_subscription` |
| `organization.rate_limit_tier` | `default_claude_ai` |
| `organization.subscription_status` | `active` |
| `organization.subscription_created_at` | ISO 8601 UTC with microseconds (`YYYY-MM-DDThh:mm:ss.ffffffZ`) |
| `organization.claude_code_trial_ends_at`, `claude_code_trial_duration_days` | null |
| `organization.seat_tier` | null |
| `organization.has_extra_usage_enabled` | true |
| `organization.plan_display_name` | absent |
| `organization.payment_auth_hosted_invoice_url` | null |
| `organization.cc_onboarding_flags` | empty object |
| `organization.claude_ai_completion_feedback_enabled`, `claude_ai_safety_feedback_enabled` | bool |
| `account.created_at` | same format; 11 s before `subscription_created_at` on this account |
| `account.has_claude_max` / `account.has_claude_pro` | false / true |
| `enabled_plugins` | empty array |

Types only: `account.display_name`, `account.full_name`, `organization.name`, `application.name`, `application.slug` are strings. The key set is identical to the first pass; no field was added.

**`subscription_status` (binary).** Zero occurrences of `subscription_status`, `subscriptionStatus`, `"past_due"`, `trialing` and `"unpaid"`. Claude Code 2.1.314 never reads or compares the field, so there is no list of values to take from it. `active` is now **(live)** confirmed; any other value is still **(inferred)** Stripe-style. A safe display is the raw string, with a warning on anything other than `active`.

**Plan detection (binary).** Besides `organization_type`, Claude Code reads `account.has_claude_max` / `has_claude_pro` in the "switch to subscription" notice for API-key users ("Use your existing Claude Max/Pro plan with Claude Code"). For an `--info` label, `organization_type` plus `rate_limit_tier` stays the right rule; `default_claude_ai` is the plain Pro tier.

**Billing period, renewal, cancel date: confirmed not available.** Zero occurrences of `current_period`, `renews_at`, `next_charge`, `plan_ending` and `cancel_at` in the binary, and no such key in the live profile. `billing_period` occurs only inside the `cedar_ember.event_props` telemetry enum (`monthly`, `annual`), described in 1.3, which was null in this read. Section 2 stands.

### F.3 Usage, second read

Key paths and types beyond what 1.3 lists:
- `iguana_necktie`: `utilization` float, `resets_at` string, `limit_dollars` int, `used_dollars` float, `remaining_dollars` float, `locked_reason` null. **Not in Claude Code 2.1.314** (zero occurrences). It has the same shape as `wattle_ember`, which 2.1.314 renders as a dollar credit bar titled by its `label` (fallback "Credit"), with optional `periods[]` (`starts_at`, `ends_at`). **(inferred)** A promotional or granted dollar credit window; its meaning is unknown, so show it only as a generic "credit: used / limit, resets" line or not at all.
- `seven_day_breakdown`: `as_of` string, `window_started_at` string, `rows[]` with `key` string, `display_name` string, `percent` int (4 rows here). Not read by Claude Code. **(inferred)** How the seven-day window splits across models or surfaces; `window_started_at` is the start of the current weekly window, not a billing period.
- New null keys since the first pass: `amber_cistern`, `amber_cistern_offer`, `amber_gauge`, `pace`, `weekly_scoped_shares` and others. Code names with no known meaning.
- `limits[]` items: Claude Code's schema adds an optional `scope` with `model.display_name` and `surface.display_name` (null here).
- `cinder_cove` (null here) is rendered by Claude Code as "Claude Code and Cowork credit".
- Nothing in `usage` gives a subscription period, renewal, status or plan name.

### F.4 What `--info` can show

| Item | Endpoint | Field path | Shape | Extra request | Status |
|---|---|---|---|---|---|
| Plan family | profile (or saved login) | `organization.organization_type` / `subscriptionType` | `claude_pro`, `claude_max`, `claude_team`, `claude_enterprise` | no | confirmed |
| Max 5x / 20x | profile (or saved login) | `organization.rate_limit_tier` / `rateLimitTier` | `default_claude_ai`, `default_claude_max_5x`, `default_claude_max_20x` | no | confirmed |
| Server plan label | profile | `organization.plan_display_name` | string, often absent | no | confirmed absent here |
| Account name | profile (or saved `oauthAccount`) | `account.display_name`, `account.full_name` | string | no | confirmed type |
| Org name | profile (or saved `oauthAccount`) | `organization.name` / `organizationName` | string | no | confirmed type |
| Account created | profile | `account.created_at` | ISO 8601 UTC | no | confirmed |
| Subscription start | profile | `organization.subscription_created_at` | ISO 8601 UTC | no | confirmed; meaning after a plan change undocumented |
| Billing channel | profile | `organization.billing_type` | `stripe_subscription`, `apple_subscription`, `google_play_subscription`, `aws_marketplace`, ... | no | confirmed |
| Subscription status | profile | `organization.subscription_status` | `active`; other values unseen | no | `active` confirmed, others inferred |
| Payment needs action | profile | `organization.payment_auth_hosted_invoice_url` non-null | null or URL; show only a flag | no | inferred meaning |
| Claude Code trial | profile | `organization.claude_code_trial_ends_at`, `claude_code_trial_duration_days` | ISO date / int, null off-trial | no | confirmed null |
| Seat tier | profile | `organization.seat_tier` | null on personal plans | no | confirmed null; Team values inferred |
| Extra usage on | profile + usage | `organization.has_extra_usage_enabled`; `extra_usage.is_enabled` | bool | no | confirmed |
| Extra usage spend and cap | usage | `extra_usage.used_credits`, `monthly_limit`, `currency`, `decimal_places`, `disabled_reason` | minor units | no | confirmed |
| Credit windows | usage | `wattle_ember`, `iguana_necktie`, `cinder_cove` | dollars used/limit, `resets_at` | no | `wattle_ember`/`cinder_cove` read by Claude Code; `iguana_necktie` unknown |
| Prepaid credit balance | `GET /api/oauth/organizations/:orgUUID/prepaid/credits` | `amount`, `currency` | minor units | yes | binary only, not called |
| Guest passes eligible, left, total | referral eligibility | `eligible`, `remaining_passes`, `limit` | bool, int, int | yes, or read `passesEligibilityCache` in `~/.claude.json` | confirmed live (true, 0, 3) |
| Referral reward | referral eligibility | `referrer_reward.amount_minor_units`, `currency` | int, ISO currency | same as above | confirmed live (1000, USD) |
| Pass slots used | referral redemptions | `redemptions` length vs `limit` | array, int | yes | binary only, not called; `remaining_passes` already gives the count |

**Cannot be shown:**
- Renewal or next charge date, billing period start and end, monthly vs annual, cancel or plan end date: only on the claude.ai cookie route `subscription_details` (section 2); not in profile, usage, or any OAuth route in the binary.
- Referral code and links: available, but they are a credential and must never be printed.
- Who redeemed a pass: Claude Code reads only the count of `redemptions`; the element shape is unverified and would identify other people.
- Meaning of the code-named usage windows and of non-`active` statuses: undocumented and unread by Claude Code.

**(inferred) Recommendation.** `--info` needs no new request for everything except passes and prepaid balance: the profile ccam already fetches plus the usage response it already parses cover the rest. For passes, read `passesEligibilityCache[organizationUuid]` from `~/.claude.json` first (no request, and it covers non-live saved accounts), show its age, and fall back to one eligibility GET only for the live account when the entry is missing. The route needs `x-organization-uuid` as well as the bearer token. Record the route and cache key in `internals.md` as *undocumented* when implementing.
