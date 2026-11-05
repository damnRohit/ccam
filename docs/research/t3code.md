# Lanes in T3 Code: research

Observed 2026-10-30 on Windows: T3 Code (Beta), server 0.1.5, Claude Code 2.1.314. Source: the shipped server bundle `%LOCALAPPDATA%\Programs\t3code\resources\server.asar` (`apps/server/dist/bin.mjs`, `packages/contracts/src/providerInstance.ts`), `~/.t3/userdata/settings.json`, `~/.t3/caches/claudeAgent.json`, and the `t3_thread_configure` MCP tool. Everything here is *undocumented* T3 internals, read from the bundle.

## How T3 runs Claude

- T3 drives Claude Code through the Claude Agent SDK (`pathToClaudeCodeExecutable`, `query(...)`). It does not start an interactive `claude`, so `ccam run` cannot sit between T3 and Claude.
- Providers are **instances**, stored in `settings.json` as `providerInstances: { <instanceId>: ProviderInstanceConfig }`. Each instance has:
  - `driver` (`claudeAgent` for Claude), `displayName`, `accentColor`, `enabled`;
  - `environment`: a list of `{name, value, sensitive}` variables;
  - `config`, decoded by the driver.
- One driver can have any number of instances (the id is the map key). The settings UI has an "Add instance" flow.
- The Claude driver config (`ClaudeSettings`) holds:
  - `binaryPath`;
  - `homePath`, titled "CLAUDE_CONFIG_DIR path" ("Custom Claude home and config directory"), which expands `~/` and is passed to Claude as `CLAUDE_CONFIG_DIR`;
  - `launchArgs`, `autoCompactWindow` and `idleCompaction`.
- A thread's model selection carries an `instanceId`. The default thread here is `instanceId: "claudeAgent"`, runtime mode `full-access`, which is `bypassPermissions`.
- Auth status per instance is read from that instance's home (`claudeAgent.json` `auth`). The login failure message tells the user to run `claude auth login` with `CLAUDE_CONFIG_DIR` set to the instance's dir.

## What that means for lanes

- **A lane maps to one T3 Claude instance** with `homePath` set to `~/.claude/ccam/lanes/<id>`. Everything ccam relies on is per config dir: the credentials file, `.claude.json`, the refresh locks and the presence record key. So:
  - `ccam move <lane> <account>` rewrites that instance's login, and every T3 thread on the instance follows on its next request (`docs/research/sessions.md` 5.1);
  - `ccam to` keeps moving only the default instance (`~/.claude`).
- **Hooks keep working.** The session query sets no `settingSources`, and the SDK adds `--setting-sources` only when it is set, so Claude loads the lane's own `settings.json` with ccam's `--config-dir <lane>` hooks. `customHome` changes `settingSources` only in T3's bypass-availability and prompt-cache probes, which read `<home>/settings.json` directly.
- **`ccam sessions` sees T3 threads.** SDK sessions register in the config dir's `sessions/` like any Claude session. A T3 thread showed up as a `busy` interactive session with a pid under `main`.
- **History is shared** through the lane's `projects` junction, as for `ccam run`.

## Gaps

1. **Memory redirect is missing.** `ccam run` passes a per-project `--settings` file pointing `autoMemoryDirectory` at the real `~/.claude/projects/<slug>/memory`. T3 has no per-project launch hook:
   - `launchArgs` is per instance;
   - T3 strips `--settings` from `launchArgs` and reads that file itself (`readClaudeLaunchArgSettings`) into `sdkSettings`.
   So T3 lane threads write memory through the junction and hit anthropics/claude-code#98044. That was reproduced in `acceptEdits` (`sessions.md` 2.2). It is untested in `bypassPermissions`. Candidate fix: write `autoMemoryDirectory` plus the `Read(//...memory/**)` rule into the project's `.claude/settings.local.json`. The memory docs say project and local scopes are honoured (under the workspace trust rule). For the main login it names the default path, so it is harmless there.
2. ~~**Creating a lane needs `ccam run`**, which also launches Claude.~~ **Closed 2026-10-30** by `ccam lane <account> [--lane <id>]`: it prepares the lane and checks the account out without launching Claude, and writes the lane's T3 instance (`docs/SPEC.md` 9.8). It does not check the Claude Code version, since T3 runs its own binary (gap 4).
3. ~~**Lane settings go stale.** The lane's `settings.json`, `CLAUDE.md` and `keybindings.json` are copies refreshed on each `ccam run`, and T3 never calls ccam.~~ **Closed 2026-10-30.** The lane's `SessionStart` hook now refreshes those copies from the main config home (`lane::refresh_copies`, called from `hooks::handle`), so a T3 thread picks up edits when its session starts. `ccam run` and `ccam lane` refresh them too. Files are written only when they changed. The hook runs in T3 sessions because the SDK sets no `settingSources` (see "Hooks keep working" and section 2 below). Not yet confirmed end to end in T3: step 8 of `docs/MANUAL-TESTS.md` 10.
4. **No version gate.** T3 uses its own Claude binary, 2.1.314 here, which is above the lane floor of 2.1.296. ccam does not check it.
5. **Moving an existing thread to another instance**, through `t3_thread_configure` `modelSelection.instanceId`, is untested. The shared `projects` folder holds the transcript, but T3 may start a fresh native session instead of resuming.

## Web and source verification (2026-10-30)

Sources, besides the earlier sections:
- **B@n**: byte offset `n` in `%LOCALAPPDATA%\Programs\t3code\resources\server.asar` (server 0.1.5), found by fixed-string search. Module names come from the bundle's `// src/...` comments.
- **T3 docs**: `docs/user/*.md` in github.com/pingdotgg/t3code (MIT) at fa4e467 (HEAD, 2026-10-30), for example `providers-claude.md` (last changed bd3287b).
- **T3#n**: https://github.com/pingdotgg/t3code/issues/n
- **CC#n**: https://github.com/anthropics/claude-code/issues/n
- **CL**: https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md
- **SDK-CL**: https://github.com/anthropics/claude-agent-sdk-typescript/blob/main/CHANGELOG.md
- Doc pages are under https://code.claude.com/docs/en/ and were read as raw `.md`.

### 1. T3 provider instances, switching and parallel threads

- **Several Claude accounts are a documented setup.** Each account gets its own config dir: log in with `CLAUDE_CONFIG_DIR=~/.claude_personal claude auth login`, then add an instance whose "CLAUDE_CONFIG_DIR path" is that dir (T3 docs `providers-claude.md` "Separate accounts or configurations"). The docs say: "Separate account directories stay isolated, including their local conversation state. Claude does not have Codex's shared-home and shadow-home arrangement." T3 PR #4835 (closed) moved the setup from overriding `HOME` to setting `CLAUDE_CONFIG_DIR`.
- **Account pools are out of scope.** FAQ "Can I add a bunch of accounts for one provider?": "I don't think T3 Code is the right place to manage a pile of accounts. If we add an auto-switcher, we are risking TOS bans". For a larger pool the FAQ points to CLIProxyAPI (T3 docs `faq.md`). T3 PR #27356 (open) adds a per-instance "Use proxy" switch for CLIProxyAPI.
- **Per-instance environment** variables are documented, and a value can be marked sensitive (T3 docs `install.md` "Providers"). T3 expands `~` in `CLAUDE_CONFIG_DIR` and `CODEX_HOME` values (B@9122057). `accentColor` is undocumented in the user docs. The bundle stamps it on each instance snapshot (`withInstanceIdentity`, B@9902246), and the web picker reads it (B@123797473).
- **Claude allows several instances.** `ClaudeDriver.metadata.supportsMultipleInstances: true` (B@10426372).
- **Switching a thread's instance mid-conversation** (`src/orchestration-v2/ProviderSessionTransitionPolicy.ts`, B@13982078):
  - Each Claude instance has a continuation key `claude:home:<resolved CLAUDE_CONFIG_DIR>` (`makeClaudeContinuationGroupKey`, B@9486020). `resolveClaudeHomePath` uses `path.resolve`, not `realpath` (B@9484937).
  - Same key, meaning the same config dir: `restart_and_resume`. T3 restarts the session and resumes the *same* native Claude session id (`canResumeAcrossInstances`, B@14029917).
  - Different key: `create_with_handoff`. T3 starts a fresh native Claude session and injects a T3-built history selection (`src/orchestration-v2/ContextHandoffDelivery.ts`).
  - The handoff is capped at 16,000 tokens by default, settable with `T3CODE_CONTEXT_HANDOFF_TOKEN_CAP` (clamped 1,024 to 64,000) (B@7735755; T3 docs `portable-handoffs.md`). It "does not copy the outgoing provider's reasoning, tool-call state, or attachments", and "is a budgeted selection, not an agent-written summary".
  - Switching back to the first instance resumes that instance's old native session, cold cache included. T3#23086 (open) reports 543k tokens resumed with zero cache reads.
  - T3#18762 (open): an instance switch made on one device is reverted when another device sends.
  - **For lanes:** every lane has its own `homePath`, so every lane is its own continuation group. Moving a thread between lanes, or between a lane and main, always takes the handoff path. It never resumes the shared transcript, even though `projects/` is junctioned. **(inferred** from `path.resolve` plus the key format; untested.) This settles Gap 5: no native resume across lanes.
- **Parallel work** (T3 docs `thread-sidebar.md` "Start in the background"):
  - `Ctrl+Enter` starts a thread and opens another draft. With **New worktree**, each submission gets its own worktree.
  - Shift-clicking several models starts "a separate thread and worktree" per model.
  - `delegate_task` takes `isolation:"worktree"` for a separate checkout (system prompt text at B@9144931).
  - New worktrees go under `~/.t3/worktrees` by default (T3 docs `project-settings.md` "Worktree location").
  - No per-instance cap on concurrent threads appears in the bundle. The `*CONCURRENCY*` and `MAX_*` constants found cover diff, git, stat and browser work only. **(inferred** from absence.)

### 2. Agent SDK `settingSources` default

- T3 bundles TypeScript Agent SDK **0.3.276**, which sets `process.env.CLAUDE_AGENT_SDK_VERSION = "0.3.276"` and `CLAUDE_CODE_ENTRYPOINT = "sdk-ts"` (B@18537262, B@18540670). SDK-CL 0.3.276: "Updated to parity with Claude Code v2.1.276". T3 still runs its configured binary, 2.1.314, through `pathToClaudeCodeExecutable`.
- The SDK passes `settingSources` straight through and adds the flag only when the option is set: ``if (O !== void 0) q.push(`--setting-sources=${O.join(",")}`)`` (B@17920578; `settingSources: c` at B@18543275). The SDK's own settings resolver falls back to `["user","project","local"]` when the option is omitted (`ist`, B@18532268).
- The docs confirm it: "When `settingSources` is omitted or `undefined`, `query()` loads the same filesystem settings as the Claude Code CLI: user, project, and local" (`agent-sdk/typescript.md`). "Omitting `settingSources` is equivalent to `["user", "project", "local"]`", and filesystem hooks load "when `settingSources` includes the relevant source" (`agent-sdk/claude-code-features.md`).
- SDK 0.1.0 briefly loaded no filesystem settings by default (SDK-CL 0.1.0, "No filesystem settings by default"). The migration guide says "This default was briefly changed in v0.1.0 ... and then reverted" (`agent-sdk/migration-guide.md`).
- So T3 lane threads load the lane's user `settings.json`, with ccam's hooks, plus the project and local settings. This confirms "Hooks keep working" above.
- Auto memory loads regardless of `settingSources` (`claude-code-features.md`, "What settingSources does not control").

### 3. `autoMemoryDirectory` from `.claude/settings.local.json`, and CC#98044

- **The scope is documented.** The key "is read from any settings scope: user, project, local, policy, or `--settings`". From project or local settings it is honoured "under the same workspace trust rule as hooks" (`memory.md` "Storage location"; `settings-reference.md` `autoMemoryDirectory`, Scope "Any file").
- **The trust rule:**
  - An untracked `.claude/settings.local.json` is the user's own file. It becomes repository-supplied only "when the file is tracked in git, or `.claude` is a symlink" (`permissions.md` "When your local settings file needs trust").
  - "a `-p` or SDK session ... counts as accepted" for that trust check (same section), so T3 sessions qualify.
  - With `permissions.blockReadsOutsideWorkingDirectories` on, a repository-supplied memory dir is neither loaded nor written (`memory.md`).
- **Windows and worktrees caveat.** In a git repo the local file normally sits at the repository root, and in a worktree "it uses the file at the main checkout's root". That root placement does not apply "on Windows", where the file stays beside `.claude/settings.json` in the starting directory (`settings.md` "Where Claude Code keeps the local file in a git repository"). So on Windows a T3 thread in a T3-made worktree reads the worktree's own `.claude/settings.local.json`. ccam would have to write the file into each worktree as well as the main checkout. **(inferred**, untested.)
- **CC#98044** (open; labels `bug`, `has repro`, `memory`, `area:permissions`):
  - Cause, as traced from the 2.1.283 bundle: since 2.1.280 a write is judged by both the requested path and where it lands. The auto-memory carve-out is a `startsWith(<CLAUDE_CONFIG_DIR>/projects/<slug>/memory/)` test, which the landing path fails. The `.claude` segment of the landing then trips the sensitive-file check, which runs before allow rules. Neither allow rules, `PreToolUse` allow nor auto mode can approve the write.
  - Comment by uwuclxdy (2026-10-16, 2.1.300): the same failure hits `tool-results/` reads, `plans/` writes, `agent-memory/`, `tasks/`, `teams/` and workflow scripts. "`--settings` with `autoMemoryDirectory` set to the resolved dir clears the memory case".
  - Comment by JahyunBaek (2026-10-30): reproduced on Windows 11, 2.1.312, with an NTFS junction (Orca, below).
  - The docs state that the check covers junctions: "This applies to symbolic links on macOS, Linux, and Windows, and to directory junctions on Windows" (`permissions.md` "Symlinks").
- **Does the setting avoid the bug?** A `--settings` value does: shown in the issue and in our own test (`sessions.md` 2.2). The same key from `settings.local.json` should behave the same, because the docs treat all scopes alike once trusted. **(inferred**, needs a manual test.) The `Read(//...memory/**)` rule for `Glob` is an allow rule in local settings, which applies to an untracked file without trust (`permissions.md`).
- **bypassPermissions.**
  - The docs say the mode "disables permission prompts and safety checks so tool calls execute immediately, including writes to protected paths". Their protected-path table lists `bypassPermissions` as "Allowed" (`permission-modes.md` "Skip all checks with bypassPermissions mode", "Protected paths").
  - The prompt in CC#98044 comes from those checks, so T3 `full-access` threads should not hit it. **(inferred** from the docs; CC#98044 and its comments test only auto, acceptEdits and default modes; not tested here.)
  - T3 can downgrade bypass when the settings disable it (`announcedBypassDowngrades`, `readBypassSettings`, B@10195164).
- **No fix in 2.1.297 to 2.1.314.** CL has no 2.1.297 or 2.1.298 entry. 2.1.299 to 2.1.314 contain no auto-memory, carve-out or junction write-check fix. The symlink entries in that range concern the macOS sandbox `denyWrite`, profile-store write protection (2.1.309) and sandbox `allowRead`/`allowWrite`.

### 4. Prior art for several subscription accounts in GUIs

| Tool | Mechanism | Source |
|---|---|---|
| T3 Code | One provider instance per config dir. Pools are left to CLIProxyAPI. | T3 docs `providers-claude.md`, `faq.md` |
| Theo's CLIProxyAPI hub | All accounts sign in on one always-on home machine running CLIProxyAPI, which "keeps the auth fresh". Clients set `ANTHROPIC_BASE_URL` to it over Tailscale. The guide claims "Claude Code auth breaks every 2 to 3 days". | gist t3dotgg/59a521d2ee8d70aaf90e280c175072af (rev 14ad274) |
| Orca (stablyai/orca, MIT) | `CLAUDE_CONFIG_DIR=<userData>\claude-profiles\<id>\home` per account. It links `projects`, `session-env`, `file-history`, `shell-snapshots`, `todos`, `paste-cache`, `tasks`, `plans` and `transcripts` into the default home: junction on Windows, dir symlink elsewhere. `sessions` is deliberately left unlinked ("its live-process registry belongs to each folder's daemon"). Settings files are copied ("rename-replace saves break links"). It hits CC#98044 on Windows (stablyai/orca#36177, open). | `src/main/claude-accounts/claude-profile-history.ts` `CLAUDE_PROFILE_HISTORY_DIRS`, `claude-profile-sharing.ts`, at 24795f5 |
| opcode (winfunc/opcode) | A profile PR that switched `CLAUDE_CONFIG_DIR` per session was closed unmerged (#361). The request for config-dir support is still open (#176). | GitHub |
| Claude Squad (smtg-ai/claude-squad) | "Support multiple accounts" #195 was closed as completed (2025-08-12); its mechanism was not examined. Open PR #290 adds per-workspace env and `AgentHome` profiles. | GitHub |
| Crystal (stravu/crystal, now Nimbalyst) | No multi-account feature found. Only #12 (open) asks for a per-project executable path. | GitHub |
| Conductor | Closed source. No public doc on multiple accounts was found. | web search, 2026-10-30 |

Orca is the closest to ccam lanes: the same junction layout, and it has the same memory bug.

### 5. Which instance a delegated or created child gets

Both `delegate_task` and `create_threads` call `resolveTargetRechecking` → `resolveTarget` (`src/mcp/OrchestratorMcpService.ts`, B@16297137):
- **No target**: the child gets the parent's instance (`instanceId ??= input.parent.thread.modelSelection.instanceId`), with the parent's model and options.
- **`driverKind: "claudeAgent"` only**: the candidates are all healthy, orchestration-capable instances of that driver. The parent's instance goes first *if it is healthy*. T3 then picks the first candidate enabled for the project. If the parent's instance is unhealthy or turned off for the project, **another Claude instance can be chosen, which means another account**. The model is then that instance's first advertised model.
- **`providerInstanceId`**: exactly that instance. It fails if the driver does not match, if the instance is unregistered or constrained, or, for a new delegation, if the instance is turned off for the project (`assertTargetEnabledForProject`).
- `schedule_task` uses the parent's `modelSelection`, or the project default when there is no parent.
- **Locking a thread to an instance:** no per-thread lock exists. The nearest lever is per-project enablement. With a project selected, an instance switched off in **Settings → Providers** cannot start any new turn in that project, "including queued messages and follow-ups from delegated tasks" (T3 docs `project-settings.md`). Turning off every other Claude instance for the project therefore keeps `driverKind`-only children on the parent's instance. **(inferred** from `resolveTarget`.)
- Claude's native `Agent` subagents run inside the parent's Claude process, so they always share its config dir **(inferred)**.

### 6. Creating provider instances programmatically

- **No CLI or MCP route.** `t3 --help` (`~/.t3/bin/t3.cmd`) lists `help start serve app pair auth environment browser project thread service update uninstall theme trace doctor triage connect`, with no provider subcommand. `t3 thread start` takes `--provider <instance id>` with `--model` and `--worktree`. The only settings MCP tool, `t3_environment_preferences_update`, covers thread env mode, worktree origin, update checks, background activity and writing style, not instances (`src/mcp/toolkits/environment/tools.ts`).
- **WebSocket RPC (undocumented).**
  - Method `server.updateSettings` takes `{ patch, providerInstanceMutation? }`. `ProviderInstanceMutation` is `create | upsert | remove` with `instanceId` and `instance: ProviderInstanceConfig` (B@3601712, B@2438318).
  - It requires the auth scope `providers:manage` (B@2494849, B@15656644).
  - `create` fails if the id already exists; `upsert` replaces the whole instance entry (`applyProviderInstanceMutation`, B@6554173).
- **Editing `~/.t3/userdata/settings.json` directly does work live** (`src/serverSettings.ts`, B@6552385):
  - T3 `fs.watch`es the file and its symlink target, debounced 100 ms, then re-reads it and emits a change.
  - `layerSettingsWatcher` reconciles the provider-instance registry on every change (`mutator.reconcile(deriveProviderInstanceConfigMap(current))`, B@13612432). A new instance appears without a restart.
  - Every T3-side update re-reads the file from disk under a write semaphore before applying its patch, then writes atomically (`updateAndPersistSettings`). A stale cache therefore does not overwrite an external edit.
  - A write landing between T3's re-read and its rename can still be lost **(inferred)**.
  - A file that fails to parse makes every T3 settings update fail with `read-file` until it is fixed, so ccam must write it atomically.
  - Sensitive environment values go through T3's secret store (`persistSettingsSecrets`), so ccam writes only non-secret fields: `driver`, `displayName`, `enabled`, `config.homePath`. It leaves `accentColor`, `environment` and other keys of a `ccam-lane-*` entry as they are, and writes none of them itself (`src/t3.rs`, 2026-10-30).
