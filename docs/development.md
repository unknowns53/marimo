# Developing marimo

English | [日本語](development.ja.md)

This document describes how to build marimo, what `install` changes, where data is stored, and how marimo works. For an overview and installation, see the [README](../README.md).

- [Building from source](#building-from-source)
- [Registering hooks and statusLine](#registering-hooks-and-statusline)
- [Files and data](#files-and-data)
- [Privacy and security](#privacy-and-security)
- [Repository layout](#repository-layout)
- [Tests and checks](#tests-and-checks)
- [Testing status](#testing-status)
- [Making a release](#making-a-release)
- [Redrawing the icons](#redrawing-the-icons)
- [How it works](#how-it-works)

## Building from source

The build commands are in [1. Build](../README.md#1-build) in the README.

### Prerequisites

| Tool | Version | Used for |
| --- | --- | --- |
| Rust (rustc and cargo) | 1.90 or later (`rust-version` in `Cargo.toml`), edition 2024 | Building the hook command `marimo-hook` and the app itself |
| Node.js and npm | Node.js 22.12 or later in the 22 series, the 24 series, or 26 or later | Building and testing the app's user interface |

The Node.js versions are the range required by both the build tool Vite and the test tool Vitest. The 23 and 25 series are outside Vitest's range. The app is built with Tauri v2 (a framework for desktop apps whose user interface is built with web technologies and which call OS features from Rust).

### Build outputs

The executable from `cargo build --release -p marimo-hook` is created at `target/release/marimo-hook` (`marimo-hook.exe` on Windows). `npm run tauri -- build` first runs `npm run build`, as set by `beforeBuildCommand` in `app/src-tauri/tauri.conf.json`, and then creates the following. `target/` is at the root of the repository, not inside `app/`.

| OS | Outputs |
| --- | --- |
| macOS | `target/release/bundle/macos/marimo.app` |
| Windows | The executable `target/release/marimo.exe`, and the installers `target/release/bundle/nsis/marimo_<version>_x64-setup.exe` and `target/release/bundle/msi/marimo_<version>_x64_en-US.msi` |

Running `target\release\marimo.exe` directly works, but it is not suited to daily use: while it runs, the executable is in use, and overwriting it during a rebuild fails. The MSI installer (an installer format built into Windows) installs into Program Files for all users and requires administrator rights.

### About the ad-hoc signature

`marimo.app` is signed with an ad-hoc signature (a signature created on the spot without a certificate) rather than an Apple developer certificate, and it is not notarized by Apple. For this reason, macOS may warn that it cannot be opened the first time. In that case, Control-click `marimo.app` in Finder and choose Open, or click Open Anyway under Privacy & Security in System Settings.

## Registering hooks and statusLine

The registration steps are in [2. Register with each tool](../README.md#2-register-with-each-tool) in the README.

### What install does

`install` does the following. Steps 2 and 3 register marimo in the Claude Code settings file.

1. Copies itself to `~/.marimo/bin/marimo-hook`. settings.json refers to this path, so the hooks keep working even if you delete or rebuild `target/` in the repository.
2. Adds to `hooks` in the settings file a hook for each of the following 16 events, which calls `~/.marimo/bin/marimo-hook` with the argument `hook`. The timeout is 5 seconds.
   SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PermissionDenied, PostToolUse, PostToolUseFailure, Notification, Elicitation, ElicitationResult, Stop, StopFailure, SessionEnd, MessageDisplay, SubagentStart, SubagentStop

   On macOS, the added hook looks like this. `command` holds the absolute path of the executable; on Windows it is a `\`-separated path such as `C:\Users\user\.marimo\bin\marimo-hook.exe`.

   ```json
   {
     "type": "command",
     "command": "/Users/user/.marimo/bin/marimo-hook",
     "args": ["hook"],
     "timeout": 5
   }
   ```

   A hook with `args` is called the exec form: Claude Code starts the executable in `command` directly, without a shell, and passes `args` as its arguments as is. Because no shell is involved, a path with spaces does not need quoting, and text printed by shell startup files never mixes into the hook's output. The Claude Code version that supports the exec form is given in [Requirements](../README.md#requirements) in the README.

   If a hook that passes the string `~/.marimo/bin/marimo-hook hook` to a shell is registered, it is rewritten to the exec form, keeping its group and position.

3. Registers statusLine (see [How statusLine is wrapped](#how-statusline-is-wrapped)).
4. Registers the Codex hooks (see [Codex hooks](#codex-hooks)).
5. Places the OpenCode plugin (see [The OpenCode plugin](#the-opencode-plugin)).
6. Registers the Antigravity hooks (see [Antigravity hooks](#antigravity-hooks)).

Steps 4 to 6 run only when the tool's settings folder exists. If the folder does not exist, marimo treats the tool as not installed and says nothing about it. Hermes Agent is not handled by `install` and is registered by hand (see [Hermes hooks](#hermes-hooks)).

| Tool | Settings folder | Registered in |
| --- | --- | --- |
| Codex | The folder that the environment variable `CODEX_HOME` points to, or `~/.codex` if it is not set | `hooks.json` |
| OpenCode | `opencode` under the environment variable `XDG_CONFIG_HOME`, or `.config/opencode` in your home folder if it is not set (the same on every OS) | `plugins/marimo.js` |
| Antigravity | `~/.gemini/config` | `hooks.json` |

### How statusLine is wrapped

If no statusLine is set, only `marimo-hook statusline` is registered. If you already use a statusLine, the original command is wrapped in the form `marimo-hook statusline -- sh -c '<original command>'`. marimo passes the input it receives to the original command unchanged and returns the original command's output and exit code unchanged, so your statusLine looks the same. If the statusLine is not of the command type, it is not changed. statusLine has no exec form, so it is registered as a command string that goes through a shell.

On Windows, Claude Code runs statusLine in Git Bash if it is available, and in PowerShell otherwise. Which one runs depends on the environment, and the two have different quoting rules, so marimo writes the command without quotes, in a form both shells interpret the same way. The path of marimo-hook starts with `~/` if the executable is under your home folder, such as `~/.marimo/bin/marimo-hook.exe statusline`, and is otherwise written as an absolute path separated by `/`. If the path contains spaces or similar characters and cannot be written in either form, statusLine is not registered and an existing statusLine is not wrapped.

An existing statusLine is wrapped only if it is a command made of words without quotes or symbols, separated by single spaces, such as `bash C:/Users/user/.claude/statusline.sh`. In that case, `sh -c` is not used; the original command simply follows, as in `~/.marimo/bin/marimo-hook.exe statusline -- bash C:/Users/user/.claude/statusline.sh`. Commands that contain quotes, `$`, `|`, `;`, `\`, and similar characters, or words starting with `~`, are interpreted differently by the two shells, so they are left unwrapped, and marimo reports that Claude Code's usage limits will not arrive from statusLine. If the only obstacle is a word starting with `~`, as in `bash ~/.claude/statusline.sh`, rewrite `~` as `C:/Users/user` and run `install` again to wrap it.

When the first word of the wrapped command is a bare name such as `bash`, marimo-hook searches the environment variable `PATH` in order and starts the first match. Rust's standard search looks in places such as System32 before `PATH`, so with WSL (a way to run Linux on Windows) installed, it would find System32's `bash.exe`, which differs from what Git Bash or PowerShell would start.

### How existing settings are preserved

`install` and `uninstall` change Claude Code's settings.json and the `hooks.json` files of Codex and Antigravity under the following rules.

- Before changing a file, they create a backup in the same folder, named by adding `.marimo-backup-<date and time>` to the file name, as in `settings.json.marimo-backup-<date and time>` and `hooks.json.marimo-backup-<date and time>`. The date and time are in UTC (Coordinated Universal Time) in the form `YYYYMMDD-HHMMSS`, and if the name already exists, a number such as `-1` is added at the end. If the file did not exist, it is created without a backup.
- Existing hooks and items unrelated to marimo are kept as they are, including the order of keys. For settings.json, the file permissions are also kept, and if it is a symbolic link, the file it points to is changed.
- Running them again gives the same result. If everything is already registered, they say that nothing changed because marimo is already installed, and leave both the file and its backups untouched. Running `install` after a rebuild replaces only `~/.marimo/bin/marimo-hook` with the new one. Only hooks still in the shell form are rewritten to the exec form. The [update scripts](usage.md#updating) also run this `install` with the new `marimo-hook` after replacing the app.
- If settings.json cannot be read as JSON (a text format for data), for example, they change nothing, show the reason, and exit with a non-zero exit code.

### Subcommands and the settings file location

`marimo-hook` has nine subcommands: `hook`, `codex-hook`, `hermes-hook`, `opencode-hook`, `antigravity-hook`, `statusline`, `record`, `install`, and `uninstall`. You use only `install` and `uninstall` directly; the others are called by the tools or used for investigation. There is no help output such as `--help`; an unknown subcommand prints a one-line error and exits.

You can specify the target settings file with `--settings <path>`. If omitted, `settings.json` under the environment variable `CLAUDE_CONFIG_DIR` is used if it is set, and `~/.claude/settings.json` otherwise.

### Codex hooks

`install` registers the following 10 events in `hooks.json`, the Codex hooks settings file. The timeout is 5 seconds.

SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop, Interrupt, SessionEnd, SubagentStart, SubagentStop

Codex hooks have no exec form, and their commands always run through the user's shell. So on macOS, marimo registers a string made of the executable's path, quoted with single quotes when needed, followed by `codex-hook`, as shown below.

```json
{
  "type": "command",
  "command": "/Users/user/.marimo/bin/marimo-hook codex-hook",
  "timeout": 5
}
```

On Windows, Codex runs hooks in PowerShell or Command Prompt depending on your settings, and the two have different quoting rules. So marimo registers the `\`-separated path as is only when it has no spaces or symbols and is interpreted the same way by both without quotes. If it cannot be written that way, the Codex hooks are not registered and marimo says so.

Codex does not run hooks, other than those distributed by an administrator, until you review and trust them with `/hooks` in the Codex CLI (the command-line interface of Codex that runs in a terminal). Trust is recorded as a value computed from the hook definition. marimo does not change `config.toml`, where Codex records trust, and does not use launch options that skip trust. Running `install` again with an executable in the same location leaves `hooks.json` unchanged, so you do not need to trust the hooks again. If the executable's location changes, for example because you changed `MARIMO_HOME`, trust them again with `/hooks`.

`hooks.json` is changed under the rules in [How existing settings are preserved](#how-existing-settings-are-preserved).

### Hermes hooks

`install` and `uninstall` do not change Hermes's settings. Hermes's settings file `config.yaml` is YAML with many comments, and hooks you wrote yourself sit under the same `hooks`, so marimo has no way to change it without breaking them. Register marimo by hand as follows.

1. Add the following 12 events to `hooks` in `config.yaml` in the Hermes folder (the location the environment variable `HERMES_HOME` points to; if it is not set, `%LOCALAPPDATA%\hermes` on Windows and `~/.hermes` on other OSes). In `command`, write the absolute path of the `marimo-hook` copied by `install`, followed by `hermes-hook`. Hermes splits this string on spaces and runs it without a shell, so if the path has spaces, enclose it in double quotes.

   ```yaml
   hooks:
     on_session_start: &marimo
       - command: 'C:\Users\user\.marimo\bin\marimo-hook.exe hermes-hook'
         timeout: 5
     on_session_reset: *marimo
     on_session_finalize: *marimo
     pre_llm_call: *marimo
     post_llm_call: *marimo
     on_session_end: *marimo
     pre_tool_call: *marimo
     post_tool_call: *marimo
     pre_approval_request: *marimo
     post_approval_response: *marimo
     api_request_error: *marimo
     post_api_request: *marimo
   ```

   `&marimo` and `*marimo` are YAML's way of writing the same content once and reusing it. If `hooks` already has hooks for the same event, do not use `*marimo`; instead, add one marimo entry to that event's list. Do not add `fail_closed`. The marimo hook always exits with code 0 and prints nothing to standard output, so it never stops Hermes's operations.

2. Hermes does not run a hook it has not seen before until you approve it. Approvals are recorded per pair of event and command in `shell-hooks-allowlist.json` in the Hermes folder. Start `hermes` once in a terminal; it asks about each pair, and you approve each one. Starting with `hermes --accept-hooks` approves them all at once. The gateway cannot show prompts, so hooks not yet approved are skipped without being registered. You can check approval status with `hermes hooks list`.

3. Hermes reads hooks only at startup, so restart the gateway.

If the location of `marimo-hook` changes, rewrite `command` and approve it again. To remove marimo, delete its entries from `config.yaml` and remove the approval record with `hermes hooks revoke '<command string>'`.

### The OpenCode plugin

OpenCode v2 loads JavaScript files placed in `plugins/` in its settings folder at startup, without listing them in the settings file. So `install` does not change the OpenCode settings file (`opencode.jsonc`) and instead places one plugin at `plugins/marimo.js`. If `plugins/` does not exist, it is created.

The plugin is made from the template `crates/marimo-hook/src/opencode_plugin.js` in the repository, with the absolute path of the `marimo-hook` copied by `install` embedded as a JSON string. A JSON string can be read as a JavaScript string as is, so spaces or quotes in the path do not break it. The plugin starts `marimo-hook` without a shell.

Existing files are handled as follows.

- A file whose first line is `// marimo-opencode-plugin` is treated as one created by marimo. If a file of the same name lacks this marker, neither `install` nor `uninstall` touches it, and they say so.
- If the content is the same, they say that nothing changed because marimo is already installed, and leave the file untouched. If you edited a file created by marimo by hand, the next `install` restores it.
- If `plugins/marimo.js` is a symbolic link, the link is kept and the file it points to is changed.
- Unlike settings.json, no backup is created. The file is written to a temporary file and then renamed.

OpenCode loads plugins only at startup, so when the file is created or deleted, `install` and `uninstall` show the following message.

```text
OpenCode loads plugins when it starts. Restart any running OpenCode, including the background service (restart the service with opencode service restart).
```

### Antigravity hooks

agy's per-user hooks are written in `~/.gemini/config/hooks.json`. It has been confirmed that `/hooks` in the agy CLI lists the entries in this file, and that the agy run by the Gemini desktop app (Hub) reloads and calls the hooks in this file without restarting. The top level of this file maps hook names to their settings. `install` writes a single entry named `marimo`, without touching entries with other names.

```json
{
  "marimo": {
    "PreInvocation": [
      { "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PreInvocation", "timeout": 10 }
    ],
    "PreToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PreToolUse", "timeout": 10 }] }
    ],
    "PostToolUse": [
      { "matcher": "*", "hooks": [{ "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook PostToolUse", "timeout": 10 }] }
    ],
    "Stop": [
      { "type": "command", "command": "/Users/user/.marimo/bin/marimo-hook antigravity-hook Stop", "timeout": 10 }
    ]
  }
}
```

agy stops and waits until a hook finishes, so marimo registers only the four events it uses to decide states, with a time limit of 10 seconds. The agy run by the Gemini desktop app does not include `hookEventName` in its input, so the event name is passed as a command argument. agy's handlers have no args, and pass `command` as written to `sh -c` on Unix and to `cmd /c` on Windows. So, as with Codex, the path is single-quoted on Unix, and on Windows the hooks are registered only when the path is a single word without spaces or symbols (see [Codex hooks](#codex-hooks)).

- An entry is treated as marimo's if all of its handlers call `antigravity-hook` of a `marimo-hook` somewhere, with or without the event name argument. marimo entries pointing to a `marimo-hook` in an old location are replaced by the new `marimo` entry, whatever their name.
- If an entry named `marimo` is not marimo's, the file is not changed and the command exits with an error.
- Otherwise, the file is changed under the rules in [How existing settings are preserved](#how-existing-settings-are-preserved).

agy's logs show that it loads hooks at startup, so when the file changes, `install` shows the following message.

```text
Restart any running agy CLI and the Gemini desktop app so they load the hooks.
```

### What uninstall does

`uninstall` also accepts `--dry-run` and `--settings <path>`, and creates backups before changing files. It removes only what marimo added to each tool, including hooks registered in the shell form. Codex, OpenCode, and Antigravity are handled only when their settings folders exist.

| Tool | What is removed |
| --- | --- |
| Claude Code | The marimo hooks in settings.json. statusLine is removed if it was only marimo's, and restored to the original command if marimo had wrapped it |
| Codex | The marimo hooks in `hooks.json` |
| OpenCode | The `plugins/marimo.js` created by marimo. The `plugins/` folder is kept |
| Antigravity | The marimo entry in `hooks.json` |

Groups and events that contained only marimo hooks are removed entirely; where marimo's hooks sat alongside other hooks in the same event, the other hooks are kept. Hermes's `config.yaml` is not changed, so remove marimo from it by hand by following [Hermes hooks](#hermes-hooks).

## Files and data

All marimo data is stored in `~/.marimo`. If you set the environment variable `MARIMO_HOME`, that location is used instead.

| Path | Purpose |
| --- | --- |
| `sessions/<session_id>.json` | The state of each Claude Code session. Hooks write it, and SessionEnd deletes it. Done and error sessions are deleted only when the SessionEnd `reason` is `clear`, `resume`, `logout`, or `prompt_input_exit`, and are kept otherwise. The app deletes files with no update for 24 hours, the original files of conversations moved to the background, and the files of idle sessions whose process has exited |
| `sessions/<tool>-<ID>.json` | The state of each session of tools other than Claude Code. The name starts with `codex-`, `hermes-`, `opencode-`, or `antigravity-`. The ID is the conversation ID (`conversationId`) for Antigravity and the session_id for the others. marimo tells sessions apart by the pair of tool and ID, so sessions and read records never mix even if IDs overlap. The rules for writing and deleting are the same as for Claude Code, except that deleting a conversation in OpenCode (`session.deleted`) also deletes done and error sessions, and Antigravity files are deleted by the app because there is no hook that reports the end of a conversation (see [What the Antigravity hooks record](#what-the-antigravity-hooks-record)) |
| `rate_limits.json` | Claude Code's 5-hour and 7-day usage limits. Written from statusLine input only when a new response has arrived in that session. The app also writes the values that Claude Code left in `cachedUsageUtilization` in `~/.claude.json`, when they are newer than the saved `updated_at`. In that case, `updated_at` is the time Claude Code fetched the values (`fetchedAtMs`), not the time marimo read them. `cachedUsageUtilization` is not documented by Claude Code; marimo follows the form written by Claude Code 2.1.316 |
| `statusline_responses.json` | For each session, `cost.total_api_duration_ms` from the statusLine input (the total time spent waiting for API responses) and the time it last changed. If this value has increased since last time, a new response is considered to have arrived, and `rate_limits.json` is updated. Records of sessions unchanged for 24 hours are deleted at the next write |
| `codex_rate_limits.json` | Codex's usage limits. Written by the Codex hooks from the rollout, and, if a refresh interval is chosen, by the app from `account/rateLimits/read` of `codex app-server`. See the table below for the fields |
| `antigravity_rate_limits.json` | Antigravity's usage limits for the Gemini models. The same form as `rate_limits.json`, with the 5-hour usage in `five_hour` and the weekly usage in `seven_day`. If a refresh interval is chosen, the app converts the remaining fraction (`remaining_fraction`) of the buckets whose `id` is `gemini-5h` and `gemini-weekly` in the output of `agy -p /usage --output-format json` into usage and writes it. `updated_at` is the time `agy` ran. This output format is not documented; marimo follows the output of agy 1.5.8 |
| `display.json` | Display settings. See the table below for the fields |
| `window.json` | The window position and the corner of the window where the character sits |
| `acknowledged.json` | Records of triggers, such as done states, that have been marked as read |
| `dialogue.json` | Dialogue overrides you write yourself. marimo never creates this file |
| `dialogue.json.unused-default` | A `dialogue.json` that was identical to the default dialogue written out by an older version of marimo, moved aside at startup |
| `bin/marimo-hook` | The hook command copied by `install`. `marimo-hook.exe` on Windows. settings.json points to this path |
| `logs/agy-usage.log` | `agy`'s log when the app runs `agy -p /usage`. `agy` creates a new file in its own log folder every time it starts, so `--log-file` points it to this file instead. It is overwritten on every run |
| `logs/record-<date>.jsonl` | The file to which the investigation command `marimo-hook record <label>` appends the JSON it receives as is. Created only if you register this command yourself. The date is in UTC, and there is one file per day. It may contain prompts and tool arguments, so on every append, files from days more than 7 days ago are deleted. An undated `logs/record.jsonl` is also deleted at the same time if it has not been updated for more than 7 days |
| `.lock` | A lock file that keeps hooks from colliding when they write at the same time |

The fields of `display.json` are as follows. When a value is missing or invalid, the value in the right column is used.

| Field | Content | When missing or invalid |
| --- | --- | --- |
| `language` | The language of the interface, the default speech bubble dialogue, the activity summaries, and the output of `marimo-hook` and the update scripts: `"auto"`, `"ja"`, or `"en"`. Besides the app, the hooks, `marimo-hook`, and the update scripts also read it | `"auto"`: Japanese if the OS language is Japanese, English otherwise |
| `stage_scale` | The character's scale (a number from 0.6 to 4.0) | The former `scale` (a scale relative to a character area 180 pixels wide) is converted to the same size and used, and `scale` is removed the next time the scale is saved |
| `show_character` | Whether to show the character | |
| `panel_style` | How the panel is shown: `"detail"` or `"counts"` | |
| `row_order` | How the panel orders sessions: `"started"` (Start Time), `"status"` (Status), or `"updated"` (Last Update) | `"started"` |
| `panel_opacity` | The opacity of the panel background (a number from 0.2 to 1.0). Text and icons are not made transparent | 0.8. Numbers outside the range are moved to the nearest end |
| `bubble_waiting`, `bubble_done`, `bubble_error` | Which states show speech bubbles (booleans) | `true` |
| `bubble_seconds` | Seconds until a speech bubble closes on its own (an integer from 0 to 3600). 0 keeps it until clicked | 0. Numbers above the limit are moved to the limit |
| `usage_refresh_minutes` | The interval for refreshing usage limits (in minutes, an integer from 0 to 1440). 0 means never | 0. Numbers above the limit are moved to the limit |
| `usage_tools` | Which tools' values appear in the usage limits row: an object with booleans for `claude`, `codex`, and `antigravity`. Tools set to `false` are not refreshed | `true` for missing tools and non-boolean values |
| `character` | The name of the selected character | |
| `stage_aspect` | The aspect ratio (height divided by width) of the selected character's area. Saved whenever the user interface reports it, and at the next start the window opens at this ratio from the beginning, so the window does not move right after startup | 1.5. Kept within 0.25 to 4.0 |

If the old-format key `panel_mode` remains, it is read in their place until you switch the display and the new keys are saved (`"list"` means details without the character, and `"picture"` means counts only with the character). Whether you hid the window from the icon is not saved.

The fields of `codex_rate_limits.json` are as follows.

| Field | Content |
| --- | --- |
| `windows` | An array of usage limit windows. Each element has the window length in minutes `window_minutes`, the usage in percent `used_percentage`, and the next reset time in Unix seconds `resets_at`. Windows for which Codex sent no values are omitted |
| `plan_type` | The plan type that Codex reported |
| `observed_at` | The time of the rollout line the values were read from, or the time the app queried `codex app-server` (Unix milliseconds). Values from an earlier point in time do not overwrite the file |
| `updated_at` | The time marimo wrote this file (Unix milliseconds) |

Session state files have `provider`, which tells which tool the session belongs to (`claude`, `codex`, `hermes`, `opencode`, or `antigravity`). Files without `provider` are read as Claude Code sessions, and versions of the app older than the introduction of `provider` skip files whose `provider` they do not know. Every tool's file has `host`, the ID and start time of the process driving the turn (see [When a process exits abnormally](#when-a-process-exits-abnormally)). Claude Code files also have `transcript_path`, the location of the conversation log (see [Conversations moved to the background](#conversations-moved-to-the-background) and [Responses stopped by the user](#responses-stopped-by-the-user)). Other tools have the following values.

| Tool | Context usage `source` | Other fields |
| --- | --- | --- |
| Codex | `codex-rollout` | |
| Hermes | `hermes-api` | `turn_id`, the ID of the current turn, and `link`, the URL opened when you click the session in the panel |
| OpenCode | `opencode-step` | |
| Antigravity | `antigravity-transcript` | `transcript_path`, used to find the location of the conversation DB |

If `CODEX_HOME` is set, that location is used instead of `~/.codex`.

State files are changed by writing to a temporary file and then renaming it. This way, a reader never sees a file in the middle of being written (this is called an atomic write). While writing, a temporary file whose name starts with `.` and ends with `.tmp` exists for a moment.

`MARIMO_HOME` must have the same value for both the `marimo-hook` called by each tool and the app. For OpenCode, the value seen by the service process that runs the plugin is passed to `marimo-hook`. When the app is launched at login, `MARIMO_HOME` is not passed to it, so the app reads `~/.marimo`. This setting is mainly for trying things out during development without touching your real data.

## Privacy and security

marimo's top priority is never to interfere with the work of the tools it supports. The hooks only write to state files; whatever fails, they exit with code 0 and print nothing to standard output. Claude Code stops an operation when a hook exits with code 2, and adds the standard output of some hooks to Claude's context. Codex and Hermes hooks can also stop operations or add context to the conversation through their exit code and standard output. marimo does neither. The OpenCode plugin lets OpenCode continue without waiting for `marimo-hook` to finish, and does not read `marimo-hook`'s exit code or output. agy's documentation asks hooks to output JSON, but `antigravity-hook`, like the others, outputs nothing. It has been confirmed that agy 1.5.8 does not record a hook that outputs nothing as a failure, and that its handling of operations requiring approval is the same with or without hooks. Even when the marimo app is not running, none of the tools are affected.

### What marimo reads

- **Hook input** The session ID, working folder, event name, tool name and arguments, the last text of the response, the notification type, the error type, the path of the conversation log, and similar fields.
- **Hook environment** The environment variables `__CFBundleIdentifier` and `TERM_PROGRAM`, which tell which app the hook was started from; `CLAUDE_CODE_HOST_SESSION_ID`, the id of a conversation in the Code tab of the desktop app; and the terminal device names of ancestor processes. On macOS, when `__CFBundleIdentifier` is missing, marimo also reads, at session start and whenever a prompt is sent, the executable paths of ancestor processes and the `Info.plist` of the app containing them. On Windows, it additionally records in the state file the handle of the console window where Claude Code or Codex runs, and the process ID, start time, and executable path of the ancestor processes. All of this is used to jump to a session when you click it in the panel. For Hermes and OpenCode sessions, the hook's ancestors are not the conversation's window, so none of this is read.
- **The working folder's `.git`** marimo searches upward from the working folder for `.git` to decide the repository name. When `.git` is a file (in a git worktree, for example), it reads only its `gitdir:` line. It runs no git commands. It looks only when the working folder changes or the repository name is not yet known.
- **Conversation logs** On PostToolUse and Stop, marimo reads up to 4 MB from the end of the conversation log and extracts only the token count of the last response and any chat title within that range, to show context usage and the title.
- **Codex rollouts and session_index.jsonl** For Codex sessions, marimo reads up to 4 MB from the end of the rollout and extracts only the token count and usage limits from the last `token_count`, and reads up to 512 KB from the end of `session_index.jsonl` and extracts only that session's title. To tell whether you started the conversation yourself, it reads up to 64 KB from the start of the rollout whenever a hook arrives and extracts only `originator` from the first line. It does not read Codex's `auth.json` or `config.toml`.
- **Hermes's session list** For Hermes sessions, at the start of a turn, marimo looks up the session's row in `sessions/sessions.json` in the Hermes folder and extracts only the chat type and name and the Discord server and channel IDs. It does not read Hermes's `.env`, `auth.json`, or `state.db`.
- **What the OpenCode plugin passes** For OpenCode sessions, marimo receives the content of the events the plugin selects (the session ID, working folder, tool names and arguments, the kind and target of operations asking for permission, the fields of question forms, error types and messages, and the token count of each step). The plugin adds the last response text written in the turn and values it asked OpenCode for. The plugin asks OpenCode only for the parent, title, and working folder of sessions whose creation it did not see, and for context limits from the model list. It does not read OpenCode's files.
- **The process driving the turn** At session start and whenever a prompt is sent (at the start of a turn for Hermes and OpenCode), and on a hook for a session with no record yet, marimo records in the state file the ID and start time of the nearest non-shell ancestor of the hook. It reads the names of ancestor executables to recognize shells, but does not record them. The app only asks the OS whether the recorded process is running, and does not read its arguments.
- **Antigravity hook input and transcripts** For Antigravity sessions, marimo extracts the conversation ID, working folder, tool names and arguments, and the reason a turn ended from the hook input. For conversations in the Gemini desktop app, at the start and end of each turn it reads only the title from `annotations/<conversation ID>.pbtxt`. Whenever a hook arrives, it reads the first 2 KB of the conversation's transcript (which agy writes to `brain/<conversation ID>/.system_generated/logs/transcript_full.jsonl`) and the lines from the end back to the last model response, and extracts only whether it is a subagent conversation, the text of the last response, and the input token count of the last model call. It does not read agy's credentials or the database of the conversation list.
- **The Antigravity conversation DB** While an Antigravity session is working or waiting, the app opens the conversation DB (`conversations/<conversation ID>.db` in agy's data folder) of that conversation, and of subagent conversations running in that session, read-only every second, and extracts only the state of the last step, the tool name and arguments, the time the state changed, and whether the current turn has unfinished steps. This is to tell apart waiting for approval, turns stopped by declining approval, and turns stopped while waiting for a model response (see [What the Antigravity hooks record](#what-the-antigravity-hooks-record)).
- **statusLine input** marimo extracts the context usage, the usage limit values, the session name (`session_name`), and the total time spent waiting for API responses (`cost.total_api_duration_ms`). The input itself is passed unchanged to the original statusLine command. Besides when a response arrives, statusLine is also called when the prompt cache (a mechanism that temporarily keeps the earlier part of a conversation on the API side) expires, and that input contains the old usage limits from the session's last response. For this reason, usage limits are saved only when a new response has arrived since the previous call (see `statusline_responses.json`).
- **Claude Code's `~/.claude.json`** Every 30 seconds, the app checks the file's modification time, and if it changed, reads it and extracts only `cachedUsageUtilization`, the usage limits Claude Code fetched, and `oauthAccount.accountUuid` of the signed-in account. The account ID is only compared, to avoid using values fetched by another account, and is not recorded. marimo does not read credentials, including Claude Code's tokens.
- **The OS language** When **Language** is **Automatic (system language)**, marimo asks the OS for the first of your preferred languages on macOS, or the display language on Windows, to decide the interface language.
- **App icons** For the marks in the panel, marimo receives the icons of the Claude, Codex, OpenCode, and Gemini desktop apps from the OS once at startup. On macOS, it asks NSWorkspace for the icons of the apps with bundle ids `com.anthropic.claudefordesktop`, `com.openai.codex`, `ai.opencode.desktop`, and `com.google.GeminiMacOS` (the app that bundles Antigravity); on Windows, it asks PackageManager for the logos of the packages `Claude_pzs8sxrjxfjjc` and `OpenAI.Codex_2p2nqsd0c76g0`. It does not open files inside the apps directly. On Windows, it does not ask for the OpenCode and Antigravity icons. The icons received are kept only in memory and never written to files. For the Hermes Agent icon, and for the OpenCode icon when the OpenCode desktop app's icon cannot be obtained, marimo uses images bundled in `app/src/assets/providers/` (sources and licenses are in `LICENSE-ASSETS`).

### What marimo runs

- **`codex app-server`** Only when you choose a **Refresh interval** in the settings window, and the saved Codex values are older than the interval, marimo starts it, sends only `account/rateLimits/read` after `initialize`, and, once it receives the response, closes its standard input to make it exit. Codex handles its own authentication, and marimo does not read Codex's `auth.json`. No conversation is started, so no rollout is created.
- **`claude -p "/usage" --no-session-persistence --settings '{"disableAllHooks": true}'`** Only when you choose a **Refresh interval** in the settings window, and the saved Claude values are older than the interval, marimo starts it with `~/.marimo` as the working folder. Claude Code does not refetch the usage limits in `~/.claude.json` just because a session is running. `/usage` is handled inside Claude Code, and no request is sent to a model. `--no-session-persistence` leaves no conversation log, and `disableAllHooks` keeps hooks, including marimo's, from running. If it does not exit within 60 seconds, marimo stops it.
- **`agy -p /usage --output-format json --log-file ~/.marimo/logs/agy-usage.log`** Only when you choose a **Refresh interval** in the settings window, and the saved Antigravity values are older than the interval, marimo starts it with `~/.marimo` as the working folder. `/usage` is handled inside agy, creates no conversation, and sends no request to a model. agy handles its own authentication with Antigravity, and marimo does not read agy's credentials. If it does not exit within 60 seconds, marimo stops it.

None of these commands is started if you uncheck the tool under **Usage limits** in the settings window, or while that tool is working and new values are arriving. GUI apps do not inherit the terminal's PATH, so besides PATH, marimo looks for the commands in `~/.local/bin`, and on macOS in `/opt/homebrew/bin` and `/usr/local/bin`; for `codex`, it also looks in `Contents/Resources/codex-cli/bin`, bundled with the ChatGPT desktop app on macOS. A `codex` installed with npm runs on `node`, so marimo adds the folder of the command it found to the front of the child process's PATH.

### What marimo writes

- The app and the hooks write to files in `~/.marimo` (see [Files and data](#files-and-data)). State files contain summaries such as the commands run, file paths, and the beginning of responses, up to 500 characters. They also contain the repository name and the chat title, up to 200 characters.
- `install` and `uninstall` write to Claude Code's settings.json and its backups. If Codex and Antigravity settings folders exist, they also write to each `hooks.json` and its backups. If the OpenCode settings folder exists, they create or delete `plugins/marimo.js`.
- Turning on **Launch at login** writes a LaunchAgent plist on macOS and a registry value on Windows (see [Launching at login](usage.md#launching-at-login) for where).

### What marimo sends

marimo sends neither state files nor conversation content outside your computer. The hooks make no network connections. The app connects only to GitHub: 3 seconds after starting, once a day, and when you click **Check for Updates**, it reads the list of releases through the API, and when updating, it downloads the release files (see [Checking for updates](usage.md#checking-for-updates)). In every case, it sends only the request, which contains no marimo state. Speech bubble lines are chosen from prepared candidates, so no large language model (LLM) is called. The OpenCode plugin only asks OpenCode for information through OpenCode's plugin mechanism, and starts `marimo-hook` and passes events to its standard input.

## Repository layout

| Path | Content |
| --- | --- |
| `crates/marimo-core` | A Rust library holding the state model, aggregation of the overall state, reading and writing state files, translating each tool's input and reading its records (Claude Code conversation logs, Codex rollouts and session_index.jsonl, Hermes's session list, OpenCode events, Antigravity transcripts), checking whether the process driving a turn is alive, deciding repository names, finding a session's window and bringing it to the front on Windows, reading app logos from MSIX packages (the Windows app package format), and choosing the interface language. All code that calls Windows APIs lives here |
| `crates/marimo-hook` | The command called by each tool. For the subcommands, see [Subcommands and the settings file location](#subcommands-and-the-settings-file-location). The template of the OpenCode plugin that `install` places, `src/opencode_plugin.js`, is also here and is embedded in the executable at build time |
| `app/src-tauri` | The Tauri v2 app itself (the Rust side). It handles file watching, fetching usage limits from Claude Code, Codex, and Antigravity, window control, the menu bar and notification area icon, click-through, jumping to sessions, and reading app icons on macOS |
| `app/src` | The user interface (TypeScript). No framework is used |
| `assets/character/index.json` | The list of built-in characters (see [Making your own character](customize.md#making-your-own-character)) |
| `assets/character/koharu` | Koharu's assets: 32 PNGs, `manifest.json`, the default dialogue `dialogue.json`, and the English `dialogue.en.json` |
| `assets/character/clawd` | Clawd's assets: 22 PNGs, `manifest.json`, the default dialogue `dialogue.json`, and the English `dialogue.en.json` |
| `art/koharu` | The source images of Koharu's assets |
| `tools/character` | The script `build.py` that creates Koharu's assets and its configuration `koharu.json`, the script `clawd.py` that draws Clawd's assets, and the Python dependencies `requirements.txt` |
| `tools/tray_icon.py` | The script that draws the menu bar and notification area icons |
| `tools/app_icon.py` | The script that draws the app icon |
| `tools/release.sh` | The macOS script that makes a release (see [Making a release](#making-a-release)) |
| `tools/update.sh`, `tools/update.ps1` | The update scripts: `update.sh` for macOS and `update.ps1` for Windows (see [Updating](usage.md#updating)) |

## Tests and checks

Run the Rust tests and static checks from the root of the repository. clippy is Rust's static analysis tool, and `cargo fmt --check` checks formatting.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --check
```

Code that calls Windows APIs can be type-checked for Windows on macOS as well. Add the target beforehand with `rustup target add x86_64-pc-windows-msvc`. The app itself needs llvm-rc to compile its Windows resources, so it is not included in this check.

```bash
cargo clippy -p marimo-core -p marimo-hook --all-targets --target x86_64-pc-windows-msvc
```

Type-check, test, and build the user interface in `app/`.

```bash
npx tsc --noEmit
npm test
npm run build
```

The placement of the character, panel, and speech bubble is checked with `npm run test:visual`. Playwright (a test tool that drives browsers) opens `index.html` in headless Chromium (a browser that shows no window on screen), replaces Tauri's `invoke` and `listen` with mocks from `@tauri-apps/api/mocks`, and checks the layouts for all four corners and switching between details and counts by clicking the character. Nothing appears on screen, so it does not get in the way of your work. Screenshots of each corner are saved in `app/test-results/`. The first time, get the browser with `npx playwright install chromium`. The CI does not run this test.

Placing the window and choosing its corner again (`app/src-tauri/src/window_pos.rs`) puts window operations behind a trait called `Screen` (Rust's equivalent of an interface), so `cargo test` checks the whole flow from startup to saving with a fake window that has no screen. macOS applies window size and position changes later, and reading them back right away returns the old values, so the fake behaves the same way.

To run the app during development, use `npm run tauri -- dev` in `app/`, which starts the development server for the user interface (port 1420) and then the app. To try things without touching your real `~/.marimo`, point `MARIMO_HOME` to another folder (see [Files and data](#files-and-data)).

### CI

The GitHub Actions CI (`.github/workflows/ci.yml`) runs the checks above on both macOS and Windows for every push to main and every pull request. It builds and tests the user interface, runs clippy and `cargo test --workspace`, and runs `cargo fmt --check` on macOS only. clippy treats warnings as failures. Code for Windows is also built and tested on Windows, so mismatches that the type check above cannot catch are found here.

On a push to main, after the checks pass on both OSes, the CI makes and publishes the builds that the update scripts download. On macOS, it builds `marimo.app` and `marimo-hook` on a GitHub-provided arm64 Mac and packs them into `marimo-macos-arm64.tar.gz` with tar, which keeps symbolic links and execute permissions. On Windows, it builds the NSIS (a system for making Windows installers) installer and `marimo-hook.exe` on x64 Windows and packs them into `marimo-windows-x64.zip`. Only when both builds succeed does it publish the two files, together with `SHA256SUMS`, which lists their checksums, as a GitHub prerelease. The prerelease gets a tag named `build-<first 12 characters of the commit hash>` pointing to that commit. The update scripts look for builds by this name when `--main` or `--no-pull` is given (see [Updating](usage.md#updating)). When a failed run is rerun, the files of the same prerelease are replaced.

After publishing, the CI sorts the prereleases whose names start with `build-` by creation time, keeps the newest 3, and deletes older ones together with their tags. This step does not touch releases and tags that start with `v`. main builds are deleted oldest first, so if you go back to an old commit after main has moved on, its build may no longer exist. In that case, build locally with `--build`.

Pushes and pull requests that change only documents (`.md` files, files in `docs/`, and files starting with `LICENSE-`) do not run the CI. GitHub does not apply this filter to tag pushes, so pushes of tags starting with `v` always run it. Besides documents, the scripts in `tools/`, the source images in `art/`, and `.gitignore` are also not part of the app, so when a push to main changes only these, the CI runs the checks but does not build. The `target` job finds the last commit that changed anything else, and builds and publishes only if that commit's build is not yet among the releases. The publishing job tags it with that commit's name, and the update scripts look for the tag by the same rule. This list of excluded paths exists in three places, the `target` job and the two scripts, so change them together.

Pull requests, and manual runs (workflow_dispatch) on anything other than main and tags starting with `v`, only run the checks, without building or publishing. A manual run on main builds and publishes if there is no build, just like a push to main. A manual run on a tag starting with `v` makes a stable release, just like a tag push (see [Making a release](#making-a-release)). A force push to a commit with unchanged content does not start the CI, so make that commit's build with a manual run. When you push to main several times in a row, the build of the older commit is stopped once the build of the newer commit starts, but publishing and deleting old builds that have already started run to the end. Only the publishing job is given permission to change releases; the other jobs run with read permission. The `marimo.app` built by the CI also has an ad-hoc signature, like a local build (see [About the ad-hoc signature](#about-the-ad-hoc-signature)).

## Testing status

This section summarizes what has been tested for each tool and interface, and the constraints. The required versions are in [Requirements](../README.md#requirements) in the README.

| Tool | How it is used | Status | What was tested, and constraints |
| --- | --- | --- | --- |
| Claude Code | CLI in a macOS terminal | Tested | All features. Both the hooks and statusLine, and building locally and updating with the update script, have been tested. Jumping to a session by clicking it in the panel has been tested in Terminal.app and Ghostty, and in Ghostty it also works from Claude Code background sessions. How sessions are told apart in iTerm2 and the integrated terminal of VS Code follows each app's documentation |
| Claude Code | The Code tab of the macOS desktop app | Tested | Hooks arrive, including for waiting. statusLine is not called (see [Usage limits](usage.md#usage-limits)) |
| Claude Code | VS Code extension | Not tested | Whether statusLine works has not been checked |
| Claude Code | Windows | Partly tested | On Windows 11, the following have been tested: building, installing with the NSIS installer, building locally and updating with the update script, `install`, hooks firing, launching at login, click-through, and hiding and showing the window from the notification area icon. It has also been confirmed that `install` registers statusLine when there is none, in both Git Bash and PowerShell, and that when `install` wraps an existing statusLine, the original display is unchanged and CLI sessions show context usage as a percentage (see [How statusLine is wrapped](#how-statusline-is-wrapped)). Jumping to a session by clicking it in the panel has been tested in the Code tab of the desktop app, Windows Terminal, conhost, and the integrated terminal of VS Code (see [Jumping to a session](usage.md#jumping-to-a-session)) |
| Claude Code | Linux | Not tested | Not tested at all |
| Claude Code | Sessions running in the cloud (such as the Code tab on a smartphone) | Unsupported | They do not read the local `~/.claude/settings.json`, so hooks do not arrive |
| Claude Code | Cowork | Unsupported | There are reports that hooks in settings.json do not fire |
| Codex | macOS desktop app | Tested | Hooks trusted with `/hooks` arrive, and the panel shows working, waiting through `request_permissions`, done, the title, and the context usage. Clicking a session brings the desktop app to the front, and usage limits are recorded in `codex_rate_limits.json` (see [Codex hooks](#codex-hooks)) |
| Codex | Codex CLI in a macOS terminal | Tested | In Terminal.app, hooks arrive and working and waiting are shown, and clicking a session brings the terminal to the front |
| Codex | Codex CLI on Windows | Partly tested | Hooks arrive and change the state of panel sessions, and the app icon is shown. For the conditions on the executable path under which hooks can be registered, see [Codex hooks](#codex-hooks) |
| Hermes Agent | The Discord gateway on Windows | Partly tested | Hand-registered hooks are loaded by the gateway, and a conversation in a Discord channel moves from working to done as a session named after the channel. The review that Hermes runs in the background after a response does not move a done session back to working. Clicking the session opens that Discord channel and brings the Discord window to the front. Waiting, errors, and CLI conversations have not been tested (see [Hermes hooks](#hermes-hooks)) |
| OpenCode | v2 in a macOS terminal | Partly tested | With `opencode run --standalone`, sessions move from working to done, from waiting for permission to working, and from waiting for a question to working; subagents do not show their own sessions in the panel; and the title and context usage are shown. Under the TUI and the background service as well, sessions appear and move through working, waiting for permission, and done; clicking a session switches to the Ghostty terminal; interrupting returns it to idle; and errors are shown |
| OpenCode | v2 TUI in a Windows terminal | Not tested | Not tested at all |
| OpenCode | v1 desktop app | Unsupported | The marimo plugin uses the v2 plugin mechanism, so v1 does not load it |
| Antigravity | agy 1.5.8 CLI in a macOS terminal | Partly tested | With the same hooks placed in the workspace's `.agents/hooks.json`, and with `agy -p`, the following have been confirmed: sessions move from working to done; the working summary shows the command description (`toolSummary`); the context token count is shown; Ghostty is recorded as the origin; subagents do not show their own sessions in the panel and are counted inside the parent session; the parent session stays working until the subagent ends; agy started from inside Claude Code does not appear in the panel; and the handling of operations requiring approval (writing outside the workspace) is the same with or without hooks. When an operation requiring approval is automatically denied under `-p`, neither PostToolUse nor Stop arrives, the session stays working, and it returns to idle in the check after agy exits. With the hooks that `install` registered in `~/.gemini/config/hooks.json`, conversations in the interactive terminal screen also appear in the panel, and clicking a session switches to the Ghostty tab. Interrupts and errors have not been tested |
| Antigravity | Gemini desktop app | Partly tested | Hooks in `~/.gemini/config/hooks.json` arrive for conversations started without choosing a workspace. Sessions appear in the panel and move from working to done, the title the app gives and the context token count are shown, and clicking a session brings the Gemini app to the front |
| Antigravity | Windows | Not tested | Not tested at all |

## Making a release

A stable release is made by tagging a commit on main with a tag named `vX.Y.Z`. The first stable release is `v0.1.0`. By default, the update scripts install the latest of these stable releases (see [Updating](usage.md#updating)).

On macOS, passing a single version to `tools/release.sh` runs all the following steps.

```bash
tools/release.sh 0.1.3-beta.2
```

The script checks that you are on main with no uncommitted changes and that no tag of the same name exists, pulls the latest main, and then rewrites the versions in four files and the versions of the workspace crates in `Cargo.lock`. It commits this as `Bump the version to <version>`, pushes it to main, and waits for the CI on main to finish testing and building. Only if the CI passes does it push the tag `v<version>`, wait for the tag's CI to finish publishing the release, and show the release's address. If the CI on main fails, it stops without tagging, so fix the problem, push to main, and run it again with the same version. If the version files already have that version, it tags the current tip of main without making a commit. The script requires a signed-in GitHub CLI (`gh`).

To do it by hand:

1. Change the version in the following four files to the same value: `version` under `[workspace.package]` in `Cargo.toml`, `version` in `app/src-tauri/tauri.conf.json`, `version` in `app/package.json`, and the top-level `version` and `packages[""]` `version` in `app/package-lock.json`. Running `npm version X.Y.Z --no-git-tag-version` in `app/` changes both `package.json` and `package-lock.json`. After changing `Cargo.toml`, running `cargo build` once also updates the version in `Cargo.lock`, so commit it together.
2. Commit the changes to main and push them.
3. After the CI on main finishes, tag that commit with `vX.Y.Z` and push the tag. If you tag before the CI finishes, the release is still published, but the tag's CI redoes the checks and builds.

```bash
git tag v0.1.0
git push origin v0.1.0
```

Pushing the tag runs the CI, which checks that the version in the tag without its leading `v` matches the versions in the four files, and that the tagged commit is on main. If either check fails, it shows the reason and fails without building. In that case, delete the local tag with `git tag -d vX.Y.Z` and the GitHub tag with `git push origin :refs/tags/vX.Y.Z`, and tag the fixed commit again.

After these checks, the CI looks for the `build-` prerelease that the push to main made for that commit. If its three files are all there, it copies them after checking them against `SHA256SUMS`, without redoing the checks and builds, and publishes them as a release titled `marimo vX.Y.Z`. The tagged commit has already been checked and built by the run on main, so the same things are not built again. If the run on main has not finished yet, or the old build has been deleted, the CI builds files with the same names as the main build on macOS and Windows after the checks pass, and publishes them. If you push the tag after the CI for the version-bump commit on main has finished, the tag's run only copies the files. A tag whose version has no `-` is published as a stable release (not a prerelease) and becomes the latest release (Latest). GitHub generates the release notes from the changes since the previous release. When a failed run is rerun, the files of the same release are replaced.

Tags with a `-` in the version, such as `v0.2.0-beta.1` and `v0.2.0-rc.1`, are treated as SemVer prereleases. The version check compares the whole version, including the part after the hyphen such as `-beta.1`, with the four files, so write the file versions as `0.2.0-beta.1` as well. The CI publishes such a tag as a GitHub prerelease and does not make it the latest release (Latest). The update scripts do not choose prereleases by default, and updating from the settings window never chooses a prerelease, whatever the app's version, so publishing a prerelease does not change what users update to. Only `tools/update.sh --pre` (`tools\update.ps1 -Pre` on Windows) installs a prerelease. When the CI publishes a stable release, it deletes the prereleases starting with `v` whose version, without the part after the hyphen, is the same or older, together with their tags. For example, publishing `v0.2.0` deletes `v0.2.0-beta.1` and keeps `v0.2.1-beta.1`. Those prereleases are never chosen by `--pre` again, because a newer release exists.

## Redrawing the icons

Run the icon scripts in the Python virtual environment created by following [Rebuilding the assets](customize.md#rebuilding-the-assets). The icons are embedded in the app at build time, so rebuild the app after redrawing them.

The menu bar and notification area icons are drawn by `tools/tray_icon.py` in the shape of a marimo ball, using only Pillow. Running it writes four PNGs to `app/src-tauri/icons/`. `tray-template.png` is the normal icon on macOS: a template image that macOS draws in white or black to match the brightness of the menu bar. The Windows notification area does not do this, so `tray-color.png` is the normal icon on Windows. `tray-waiting.png` and `tray-error.png` are used on both OSes when a session is waiting or has an error (see [The menu bar and notification area icon](usage.md#the-menu-bar-and-notification-area-icon) for how they look). The menu bar scales images down to a height of 18 points, so they are all 36×36 pixels, which is actual size on a 2× display.

The app icon is drawn by `tools/app_icon.py`: a marimo ball with a face in a water tank. Its fur is drawn by layering tens of thousands of short strokes placed at random, with a fixed random seed, so the image is the same every time. Running it writes `icon.icns`, `icon.ico`, `icon.png`, `32x32.png`, `128x128.png`, and `128x128@2x.png` to `app/src-tauri/icons/`. The macOS `icon.icns` draws a rounded square in the central 824 pixels of a 1024×1024 image with a transparent surround, so that it matches the size of other apps in the Dock. The remaining files, for Windows and Linux, crop only the rounded square from the same picture and fill the whole image with it.

Run both from the root of the repository.

```bash
.venv/bin/python tools/tray_icon.py
.venv/bin/python tools/app_icon.py
```

## How it works

### Data flow

Data flows in the following order.

1. `marimo-hook`, called by each tool, reads JSON from standard input, decides the session's new state, and writes it to a state file. Input from tools other than Claude Code is translated into Claude Code input first, and the state is decided by the same rules (see [How states are decided](#how-states-are-decided)). How it is called and where it writes are as follows.

   | Caller | Subcommand | Writes to |
   | --- | --- | --- |
   | Claude Code hooks | `marimo-hook hook` | `sessions/<session_id>.json` |
   | Codex hooks | `marimo-hook codex-hook` | `sessions/codex-<session_id>.json`, and usage limits to `codex_rate_limits.json` |
   | Hermes hooks | `marimo-hook hermes-hook` | `sessions/hermes-<session_id>.json` |
   | The plugin running inside the OpenCode service | `marimo-hook opencode-hook` | `sessions/opencode-<session_id>.json` |
   | agy hooks | `marimo-hook antigravity-hook` | `sessions/antigravity-<conversationId>.json`, recording the token count read from the transcript as context usage |
   | Claude Code statusLine | `marimo-hook statusline` | Context usage and the session name to the session's file, and, if a new response has arrived, usage limits to `rate_limits.json` |

   The OpenCode plugin subscribes to events and starts `marimo-hook` for each selected event. It starts the next one only after the previous `marimo-hook` has exited, so writes never happen out of the order of the events. A hook holds up the tool, so waiting for the lock is limited to 300 milliseconds.
2. The app checks every 30 seconds the usage limits that Claude Code left in `~/.claude.json` and writes them to `rate_limits.json`. If a refresh interval is chosen, it writes the usage limits queried from `codex app-server` to `codex_rate_limits.json`, and those read with `agy -p /usage` to `antigravity_rate_limits.json` (see [What marimo runs](#what-marimo-runs)).
3. The app receives changes to `sessions/` and `rate_limits.json` through a file watcher (a mechanism that receives a notification from the OS when files in a folder change). Parallel tool calls produce overlapping writes, so it waits until there have been no changes for 120 milliseconds, or 500 milliseconds at most, and then reads a snapshot of all sessions.
4. The Rust side switches the menu bar and notification area icon to match the overall state of the snapshot, and sends the snapshot to the user interface as an event named `snapshot`. The icon is not left to the user interface because the user interface may stop while the window is hidden. On macOS 14 and later, `backgroundThrottling` in `tauri.conf.json` is set to `disabled` so that WebKit does not stop the user interface of a hidden window. Tauri does not handle this setting on macOS 13 and earlier or on Windows.
5. The user interface decides the overall state again, taking the read records into account, and redraws the expression, speech bubble, and panel. It also decides the list of sessions at the top of the icon's menu, and sends it to the Rust side only when it changes. When you choose a session in the menu, the Rust side returns the session's key to the user interface as an event named `tray-session`, and the user interface does the same as when you click the session in the panel.

### How states are decided

Claude Code sessions take the following states depending on the hooks that arrive. How other tools' events are translated is described in the "What ... records" section for each tool.

| State | Events that lead to this state |
| --- | --- |
| Waiting | A request for tool permission (PermissionRequest, including from subagents), a question (AskUserQuestion), plan approval (ExitPlanMode), a request for input (Elicitation), a notification that the session is waiting for approval |
| Error | StopFailure (for example, when a usage limit is reached) |
| Working | Sending a prompt, before and after running a tool, the start of a subagent (SubagentStart) |
| Done | Stop, the end of the last subagent (SubagentStop) |
| Idle | The start of a session, the idle notification (idle_prompt) that arrives after an interrupt or a declined approval |

Done is ranked below working so that, while you watch one session work, the character's expression does not change every time another session finishes. You can tell that a session is done from its color in the panel.

The "activity summary" item in each tool's section maps that tool's tools to the Claude Code tools whose summaries are made the same way. Tools with no counterpart show their name and arguments briefly as they are.

The rest of this section is about Claude Code's subagents. How Hermes and OpenCode subagents are handled is described in [What the Hermes hooks record](#what-the-hermes-hooks-record) and [What the OpenCode plugin records](#what-the-opencode-plugin-records).

Hooks fired inside a subagent (another Claude that Claude starts with the Agent tool) arrive with the same session_id as the parent session. marimo tells them apart by the `agent_id` field in the hook input. According to Common input fields in the hooks documentation, this field is present only when the hook fires inside a subagent. In sessions started with `--agent`, the parent conversation's hooks also include `agent_type`, so `agent_type` is not used to tell them apart.

Subagent hooks change neither the name shown in the panel nor the session's own progress. Even if a subagent isolated in a worktree runs in another folder, the name shown in the panel stays the parent session's repository or folder name. marimo records running subagents with SubagentStart and SubagentStop, and while even one is running, the parent session is shown as working, even after its response has finished. When the last subagent ends, the session becomes done, and the time taken to finish is counted up to that point. When a subagent asks for tool permission or asks a question, the session is shown as waiting, and returns to its previous display once the permission is given or denied or the question is answered.

Subagent records are removed in the following cases.

- When you stop a foreground subagent partway, SubagentStop does not arrive. A foreground subagent always ends before the parent's response ends, so when the parent conversation's Stop arrives, subagents not included in its `background_tasks` (the list of work still running in the background) are removed. The hooks documentation does not state that `id` in this list has the same value as `agent_id` in SubagentStart, so if none of the subagents in the list matches a record, nothing is removed.
- When you stop the parent's response with Esc, Stop does not arrive either. So when the idle notification (idle_prompt) arrives, all recorded subagents are removed. Claude Code sends this notification only after no subagent is running in the background. If the parent conversation is done or errored, the state is kept as it is, because it is an unread result.
- SubagentStop is not guaranteed to arrive, for example when a subagent fails, so a subagent with no events for 30 minutes is treated as ended.

### When a process exits abnormally

When a tool's process crashes or is killed, no tool sends a hook or event equivalent to SessionEnd. The idle notification (idle_prompt) that Claude Code sends after an interrupt also never arrives once the process is gone. Left alone, working and waiting sessions would stay until they are deleted after 24 hours.

So the hook records the ID and start time of the process driving the turn in `host`. It records them at session start, whenever a prompt is sent, and on a hook for a session with no record yet. A conversation that moved to another process with `--resume` is updated to the new process at that point. When the hook's parent is a shell (sh, bash, zsh, dash, ksh, fish, cmd, powershell, pwsh, and whatever the environment variables `SHELL` and `COMSPEC` point to), the ancestor beyond the shell is recorded, because the shell exits along with the hook. The process recorded for each tool is as follows.

| Tool | How the hook is started | Process recorded |
| --- | --- | --- |
| Claude Code | On macOS, `/bin/sh` is replaced by the hook through exec. On Windows, Git Bash's bash.exe remains as the parent even after exec | The claude process |
| Codex | Started with `-lc` of the shell in `SHELL` on macOS, and with `/C` of `COMSPEC` (cmd.exe if unset) on Windows (build_command in codex-rs/hooks/src/engine/command_runner.rs) | The Codex process |
| Hermes | Started without a shell | The Hermes process itself |
| OpenCode | The plugin starts it without a shell | The OpenCode service process running the plugin |
| Antigravity | Started with `sh -c` on Unix | The `agy` process in the terminal |

At startup and every 10 minutes, if the process of a working or waiting session has exited, the app treats the session as interrupted, returns it to idle, and removes the records of subagents that were running. A session shown as working only because subagents were running, while the parent conversation is done or errored, stays done or errored as an unread result, and only the subagent records are removed.

If the process of an idle session has exited, the session's file is deleted. This is because the spare process that Claude Code prepares in the background (`claude bg-spare`) sends SessionStart with its own session_id, creating an idle session, and sends no SessionEnd even when it exits unused after about 60 minutes. Idle sessions are not shown in the panel, so even if a session whose file was deleted is still running, nothing looks different, and the next hook creates the file again.

Process IDs are reused, so a process whose start time differs from the record is treated as a different process. On macOS, a zombie process not yet reaped by its parent is also treated as exited. When marimo cannot check, for example for lack of permission, the process is treated as running.

### Conversations moved to the background

When you move a conversation to the background in Claude Code, the conversation continues in a session with a new session_id, which creates its own state file. Neither SessionEnd nor Stop arrives for the original session, and the original CLI process keeps running, so the original session stays working.

So the hook records `transcript_path` from the parent conversation's hook input in the state file. At startup and every 10 minutes, the app reads the end of the recorded conversation log, and deletes the original session if there is a `continued-in` line after the last line of the parent conversation (a `user` or `assistant` line whose `isSidechain` is not true). `continued-in` is a line that Claude Code writes to the original conversation log when the conversation is moved. The original session may keep responding even after the move, in which case conversation lines follow `continued-in`, and the session is not deleted. The form of this line is not in Claude Code's documentation; it was confirmed in local conversation logs.

### Responses stopped by the user

When you stop a response with Esc in Claude Code, or decline a tool permission or a question, neither Stop nor PermissionDenied arrives. The CLI sends the idle notification (idle_prompt) about 60 seconds later, so marimo returns the session to idle then. The desktop app does not send this notification, and its process keeps running, so working and waiting sessions would stay until the next prompt or for 24 hours.

So at startup and every 10 minutes, the app reads the end of the recorded conversation log. If the last line of the parent conversation is a `user` line consisting only of the text that marks an interrupt (`[Request interrupted by user]` or `[Request interrupted by user for tool use]`), working and waiting sessions are returned to idle. A prompt sent after stopping may not yet be written to the conversation log, so if the prompt was sent after the line's `timestamp`, nothing is changed. Subagents running in the background keep running after the stop, so those that sent events after that time are kept, and only those inactive since before it are removed. The form of this line is also not in Claude Code's documentation; it was confirmed in local conversation logs.

### How titles and context usage are read

This section covers Claude Code sessions. For other tools, see the "What ... records" section for each tool.

statusLine does not run in the Code tab of the desktop app, so marimo counts context usage from the conversation log (transcript: a record in JSON Lines format that Claude Code writes for each session). In this case, the model's context limit is unknown.

The title comes from two places. In the CLI, marimo uses `session_name` from the statusLine input. This is the name given with `--name` or `/rename`, or, without one, the title Claude Code assigned automatically. In the Code tab of the desktop app, where statusLine is not called, marimo looks for a title line at the end of the conversation log, which it reads on PostToolUse and Stop. The format of this line is not documented by Claude Code. In both cases, only the name is shown until a title is assigned or the first read happens.

### What the Codex hooks record

- **State** Session start, prompt submission, before and after running a tool, permission requests (PermissionRequest), the end of a response (Stop), and the start and end of subagents are mapped to the same states as in Claude Code. Codex's question tool `request_user_input` puts the session in waiting, like Claude Code's AskUserQuestion. When Codex asks for permissions such as writing outside the sandbox with `request_permissions`, no PermissionRequest arrives, so the PreToolUse of this tool puts the session in waiting, and it returns to working when PostToolUse arrives after the permission is granted or denied. Interrupt, which arrives when you interrupt a turn, returns working and waiting sessions to idle.
- **Activity summary** marimo summarizes shell commands (Codex sends them with the tool name `Bash`), file edits (`apply_patch`, showing the names of the files in the patch), tools of MCP (Model Context Protocol, a way to connect external tools to an AI), starting subagents (`spawn_agent`), showing images (`view_image`), and questions (`request_user_input`).
- **Context usage** marimo reads the last `token_count` from the end of the record Codex writes for each conversation (the rollout, a JSON Lines file under `~/.codex/sessions/`), and computes it with the same formula as the remaining percentage shown at the bottom of Codex's screen. Codex subtracts 12000 tokens, always used by things like the system prompt, from both the context limit and the usage before computing the percentage, so marimo does the same and records 100 minus the remaining percentage as the usage. It reads only at session start, PostToolUse, and Stop.
- **Title** At session start, prompt submission, and Stop, marimo reads the session's last `thread_name` from the end of `~/.codex/session_index.jsonl`. The Codex desktop app creates a working folder `~/Documents/Codex/<date>/<a name made from the first prompt>` for each conversation started without choosing a project. This folder name is merely a rewording of the title, so the title is used instead of the folder name for the name shown in the panel and for `{folder}` in speech bubbles.
- **Conversations started by programs** A state file is created only if `originator` in the `session_meta` at the start of the rollout is the name of a client you open yourself. These names are `Codex Desktop` for the desktop app, `codex-tui` for the Codex CLI in a terminal, and `codex_vscode` for the VS Code extension. Conversations started by programs, such as `codex exec` (`codex_exec`), codex-async (`codex-async-bridge`), to which Claude Code delegated work, and the Codex plugin for Claude Code (`Claude Code`), are not listed in the panel. For a conversation delegated by Claude Code, the delegating Claude Code session already shows that it is working. No state file is created for a conversation that has no rollout, or whose first line does not fit in 64 KB so that `originator` cannot be read. Conversations that the desktop app starts in the background to make suggestions (with `/` as the cwd, writing their results to `~/.codex/ambient-suggestions`) leave no rollout and are excluded here. Even for conversations you started yourself, hooks that arrive before the rollout is created do not create a state file; the session is listed from the next hook that can read `originator`. SessionEnd arrives without `transcript_path`, so a state file already created after reading `originator` is not deleted when a hook that cannot read it arrives. Usage limits are account-wide values, so they are recorded even from the rollouts of excluded conversations. The values of `originator` are not in Codex's documentation; they were confirmed in local rollouts on macOS and Windows. Conversations started through app-server contain the `clientInfo.name` that the client announced.
- **Usage limits** The usage limits in the same `token_count` of the rollout are written to `~/.marimo/codex_rate_limits.json`. If a refresh interval is chosen, the app also queries `codex app-server` and writes to the same file, so the values do not go stale while you are not using Codex. Depending on the plan, Codex has two usage limit windows, 5 hours and 7 days, or only one of 7 days, so the window length is recorded in minutes as is. The rollouts of different sessions hold values from different points in time, so the file is updated only with values from a later point than those already recorded.

### What the Hermes hooks record

For every event, the input of Hermes hooks has `hook_event_name`, `tool_name`, `tool_input`, `session_id`, and `cwd`, and puts all other values in `extra` (`agent/shell_hooks.py` in Hermes).

- **State** Events are translated as follows. Hermes's `on_session_end` is not the end of a conversation but an event that arrives at the end of every turn.

  | Hermes event | Translated to | State |
  | --- | --- | --- |
  | `on_session_start`, `on_session_reset` | SessionStart | Idle. Done and error sessions are kept as they are |
  | `pre_llm_call` | UserPromptSubmit | Working |
  | `pre_tool_call`, `post_tool_call` | PreToolUse, PostToolUse | Working. Waiting only while the clarifying tool `clarify` is in use |
  | `pre_approval_request` | PermissionRequest | Waiting (while asking you to approve a dangerous command) |
  | `post_approval_response` | PostToolUse or PermissionDenied | Working |
  | `post_llm_call`, `on_session_end` (`completed`) | Stop | Done |
  | `on_session_end` (`interrupted`) | Interrupt | Idle |
  | `on_session_end` (`failed`), `api_request_error` (`retryable` is false) | StopFailure | Error |
  | `on_session_finalize` | SessionEnd | The session is deleted. Done and error sessions are kept |

- **Background review** After a response, Hermes reviews its memory and skills in the background under the same session_id. This review uses tools without sending `pre_llm_call`, so events whose turn ID differs from the one announced by `pre_llm_call` (`extra.turn_id`) are discarded. Otherwise, a done session would go back to working.
- **Activity summary** `terminal` is treated as Bash; `read_file`, `write_file`, and `patch` as Read, Write, and Edit; `search_files` as Grep; `web_search` and `web_extract` as WebSearch and WebFetch; `delegate_task` as Agent; and `clarify` as Codex's question tool `request_user_input`.
- **Context usage** It is computed by dividing `usage.prompt_tokens` from `post_api_request`, which arrives every time the model is called, by `context_length`. `prompt_tokens` is the number of input tokens passed to the model, including those read from the cache.
- **Title and destination** At the start of a turn, marimo looks up the row whose session_id matches in `sessions/sessions.json` in the Hermes folder. This file is a map in which the gateway writes the current session for each chat, and Hermes itself calls it a mirror of `state.db` (legacy mirror). For a channel, only the channel name from a name of the form `Server name / #channel name` becomes the title; for a DM, the title is `DM` rather than the other person's name. For a Discord conversation, marimo builds and records a URL of the form `discord://-/channels/<server>/<channel>` from the server and channel IDs (the DM channel for a DM). CLI conversations are not in this map, so they have neither a title nor a URL.
- **Working folder** The gateway runs with the Hermes folder as its working folder, so that location is not where the conversation lives. A `cwd` equal to the Hermes folder is not recorded, and the title is used for the name shown in the panel and for `{folder}` in speech bubbles. For CLI conversations, the folder where Hermes was started is recorded.
- **End of a process** Sessions of cron jobs do not send the hook that reports their end (`on_session_finalize`), and neither does the gateway or CLI when it exits abnormally. So, as with other tools, sessions whose turn-driving process has exited are returned to idle (see [When a process exits abnormally](#when-a-process-exits-abnormally)). This also removes, within about 10 minutes at most, the sessions of cron jobs that were working when the gateway restarted.
- **Origin** The ancestor processes of Hermes hooks are not the conversation's window, so no clues about the origin are recorded. When you click a session in the panel, marimo opens the recorded URL after checking that it has the form of a Discord channel URL that marimo builds. Discord on Windows only passes the link to its running instance and switches screens without coming to the front, so marimo brings the Discord window to the front. Sessions without a URL do nothing.
- **What is not recorded** Usage limits are not recorded, because the Hermes hook input has no usage limit values. Subagents (child agents run by `delegate_task`) send hooks with their own session_id, so they appear in the panel as sessions separate from the parent. The gateway's `/stop` only sends events without a session_id, so a stopped session may stay working until the next turn puts it back in the right state. When Hermes changes the session_id while compressing a conversation, a session with the new ID appears and the old session is deleted after 24 hours.

### What the OpenCode plugin records

The OpenCode v2 plugin (placed by `install` at `plugins/marimo.js` in the OpenCode settings folder) selects the following events and passes them to `marimo-hook opencode-hook`. For sessions with a parent (subagents), only the events for the start and end of a turn, permissions, and questions are passed on to the parent session; the plugin discards the rest.

| OpenCode event | Translated to | State |
| --- | --- | --- |
| `session.created` | SessionStart | Idle. The title and working folder (`location.directory`) are recorded |
| `session.renamed` | None | Only the title is changed |
| `session.execution.started` | UserPromptSubmit | Working |
| `session.tool.input.started`, `session.tool.called` | PreToolUse | Working. Tool names and arguments are mapped to Claude Code's |
| `session.tool.success`, `session.tool.failed` | PostToolUse, PostToolUseFailure | Working |
| `permission.asked` | PermissionRequest | Waiting |
| `permission.replied` | PostToolUse, or PermissionDenied (`reply` is `reject`) | Working |
| `form.created` | PreToolUse (AskUserQuestion) | Waiting (question) |
| `form.replied`, `form.cancelled` | PostToolUse | Working |
| `session.execution.succeeded` | Stop | Done. The last response text written in the turn is used for the summary |
| `session.execution.failed` | StopFailure | Error. `session.step.failed` and `session.retry.scheduled` are not used, because they are retried |
| `session.execution.interrupted` (`reason` other than `superseded`) | Interrupt | Idle |
| `session.deleted` | SessionEnd | The session is deleted, including done and error sessions |
| `session.step.ended` | None | Only the context usage is changed. The token count is the sum of the step's input, output, reasoning, and cache tokens, and the limit is used only when it could be obtained from the model list |

- **Scope** The plugin subscribes to events through OpenCode v2's plugin mechanism (`ctx.event.subscribe` on the `ctx` received in `setup`). The plugin is loaded not in the terminal screen (TUI) but in the service that runs conversations. This covers the background service that the terminal screen connects to, a dedicated server started with `--standalone`, and an `opencode serve` server. The v1 desktop app is not supported.
- **Conversations delegated by Claude Code** If the environment variable `CLAUDECODE` of the process that loaded the plugin is `1`, and the process is not the background service (`opencode serve` with `--service` in its arguments), the plugin does not subscribe to events and no state file is created. Claude Code sets `CLAUDECODE=1` in the environment of processes it starts from tools, and the dedicated server started by `opencode run --standalone` inherits it. The delegating Claude Code session already shows that it is working, so the same work is not listed as a separate session. If the background service is first started from inside Claude Code, it keeps `CLAUDECODE=1` while also running conversations you open from the terminal screen, so it is not used for this distinction. OpenCode's conversation records have no value, like Codex's `originator`, that tells which client started a conversation, so an `opencode run` conversation sent to the service without `--standalone` cannot be told apart and appears in the panel.
- **Subagents** For a session with a parent, the plugin records the parent from the creation event and keeps the session from creating its own state file. For a session whose creation event it did not see, it asks OpenCode for the parent before deciding. `opencode-hook` also ignores `session.created` with a parent. The parent session is shown as working, using the tool that runs subagents (`subagent`). Only `session.execution.started` at the start of a subagent's turn, `permission.asked`, `permission.replied`, `form.created`, `form.replied`, `form.cancelled`, and `session.execution.succeeded`, `session.execution.failed`, and `session.execution.interrupted` at the end of its turn are passed on, with the ID of the topmost session without a parent, found by following parents, added as `parent`; other events are not passed on. `opencode-hook` treats these the same as events of a Claude Code subagent running inside that session. The start of a turn counts as a running subagent, as SubagentStart. While waiting for a permission or question to be answered, the session is shown as waiting; once answered, it is shown as working with a subagent running; and when the subagent's turn ends (except for an interrupt whose `reason` is `superseded`), it returns to the parent conversation's state. The same applies when a subagent running in the background asks for an answer after the parent's response has ended.
- **Sessions seen partway** The first time the plugin sees a session whose creation event it did not see, such as one that was already running before the plugin loaded, it also asks OpenCode for the session's title and working folder and adds them to the event it passes on.
- **Activity summary** `shell` is treated as Bash; `read`, `write`, and `edit` as Read, Write, and Edit; `patch` as Codex's `apply_patch`, which has the same format; `glob` and `grep` as Glob and Grep; `webfetch` and `websearch` as WebFetch and WebSearch; and `subagent` as Agent. The v1 names `bash` and `task` are handled the same way. Tool execution events do not carry the tool name, so the plugin adds the name it recorded from the input start event (`session.tool.input.started`). Permission requests arrive with the kind of operation (`action`) and target (`resources`) rather than a tool name, so they are mapped to tools by the same correspondence. For question forms, each field's description is shown as the question, together with the option labels.
- **Context usage** The token count at the end of a step is the sum of the step's input, output, reasoning, and cache reads and writes, counted the same way as the context usage shown by OpenCode's terminal screen. `session.usage.updated`, which reports a session's cumulative total, is not used. The plugin obtains the limits from OpenCode's model list when it loads, and fetches them again only when you switch to a model not in the list. If the limit still cannot be obtained, none is passed, and the panel shows a token count.
- **Order of events** The plugin starts the next `marimo-hook` only after the previous one has exited. If separate processes wrote at the same time, writes could happen out of order, and a session could go back to working after being done. Only the queue inside the plugin waits; OpenCode's processing never does. Events with the same ID are not passed twice. When OpenCode drops the subscription, the plugin subscribes again after 1 second. When the plugin is unloaded (including when OpenCode exits, as with `opencode run --standalone`), it passes on all remaining events immediately.
- **End of a process** The hook's parent is the OpenCode service process, so when the service crashes or is killed, working and waiting sessions are returned to idle within about 10 minutes at most, as with other tools (see [When a process exits abnormally](#when-a-process-exits-abnormally)).
- **Origin** The service that runs conversations is a separate process from the terminal, so no clues about the origin are recorded. When you click a session in the panel, marimo looks in Ghostty on macOS for a terminal titled `OC | <conversation title>`. This is the title that OpenCode's TUI sets on the terminal; when the title is longer than 40 characters (the JavaScript string length, counted in UTF-16 units), it is cut to 37 characters with an ellipsis added. If no terminal matches the title, marimo chooses the terminal with the same working folder if there is exactly one. If none is found, it does nothing, not even bringing Ghostty to the front. Terminal.app and iTerm2 can only look up tabs by tty (the terminal's device name), so they are not handled, and nothing is done on Windows.
- **What is not recorded** Usage limits are not recorded, because the plugin does not extract them from OpenCode events.

### What the Antigravity hooks record

agy hooks (registered by `install` in `~/.gemini/config/hooks.json`) call `marimo-hook antigravity-hook <event name>`. The form of the hook input follows `docs/hooks.md` in the agy-customizations skill bundled with agy. Unlike the 1.5.8 CLI, the input of the agy run by the Gemini desktop app has no `hookEventName` or `cwd`, so the event name is taken from the command argument and the working folder from the first entry of `workspacePaths`. Conversations started without choosing a workspace have no working folder, so, as with Hermes gateway conversations, the title is used as the name shown in the panel.

| agy event | Translated to | State |
| --- | --- | --- |
| `PreInvocation` (`invocationNum` is 0) | UserPromptSubmit | Working. Arrives before the first model call of a turn |
| `PreInvocation` (for a working session, `invocationNum` is 1 or more, or 0 when resuming after a subagent's reply) | PreInvocation | Stays working. Only the last hook and its time are changed |
| `PreToolUse` | PreToolUse | Working. Tool names and arguments are mapped to Claude Code's |
| `PostToolUse` | PostToolUse | Working |
| `Stop` (`terminationReason` is a normal end such as `NO_TOOL_CALL`, and `fullyIdle` is true) | Stop | Done. The text of the last response in the transcript is used for the summary |
| `Stop` (a normal end, with `fullyIdle` false) | PostToolUse | Stays working. A parent conversation waiting for a subagent's reply sends Stop once and resumes when the reply arrives |
| `Stop` (`terminationReason` is `USER_CANCELED`) | Interrupt | Idle |
| `Stop` (`terminationReason` is `ERROR`, `MAX_INVOCATIONS`, `MAX_FORCED_INVOCATIONS`, or `MAX_TOKEN_BUDGET_EXCEEDED`) | StopFailure | Error. The lowercase `terminationReason` is used as the dialogue category, and the `error` text for the summary |

- **Start of a turn** agy sends no `SessionStart` from the second turn on, and restarts counting `invocationNum` from 0 at the first `PreInvocation` of each turn. When a parent conversation resumes after a subagent's reply, it also restarts from 0, but the session is still working then, so the start time of the turn is not changed. `SessionStart` and `PostInvocation` are not registered.
- **Waiting** agy has no hook that is called while it asks for approval. When you decline an approval, agy stops the turn without sending PostToolUse or Stop. So, for Antigravity sessions that are working or waiting, the app reads the `steps` table of the conversation DB (`conversations/<conversation ID>.db` in agy's data folder, an SQLite file) read-only every second. It reads, among the steps after the last model response (`step_type` 15), the one waiting for approval if there is one, and otherwise the newest one. When a single response calls several tools, steps waiting their turn can line up behind the one waiting for approval. The location of the DB is derived from the transcript location that the hook recorded in the state file (`<data folder>/brain/<conversation ID>/.system_generated/logs/transcript_full.jsonl`).

  | Last step | Session state | Translated to | Change in session state |
  | --- | --- | --- | --- |
  | WAITING (9) | Working | PermissionRequest | Waiting. The summary is made from the step's tool name and arguments |
  | ERROR (7), CANCELED (6), or INTERRUPTED (12) for 2 seconds or more | Waiting | Interrupt | Idle |
  | RUNNING (2) for 1 second or more | A session put in waiting by this table | PostToolUse | Working |
  | WAITING in a subagent's conversation | The subagent is not waiting | PermissionRequest (subagent) | The parent session's subagent is put in waiting |
  | DONE (3), or an end such as ERROR, in a subagent's conversation | The subagent is waiting | PostToolUse (subagent) | The subagent's waiting is cleared |

  Even after approval, PostToolUse does not arrive until the tool finishes, and for commands not until 5 seconds later, when they switch to running asynchronously. So when the step returns to RUNNING and 1 second passes, marimo treats the approval as given and returns the session to working. When you decline or close with Esc, the step also passes through RUNNING, but becomes ERROR or CANCELED within a few milliseconds. A session put in waiting by a question tool may stay RUNNING for a few seconds before becoming WAITING, so this rule does not apply to it. When an approved tool fails, the step also becomes ERROR, but then PostToolUse arrives immediately. So marimo treats an approval as declined only when the session is still waiting 2 seconds after the step becomes ERROR.

  The step state values were read from the protobuf definitions embedded in agy's executable. The tool name and arguments (field 4) and the time of the state transition (field 26) are read from the protobuf in the `metadata` column. None of this is documented; it is the form confirmed by causing approvals, declines, and Esc in the agy 1.5.9 CLI. agy creates the DB tables when it creates a conversation, and if another agy process has the same DB open at that time, it gives up saving to SQLite with "database is locked" and switches to a format that writes the conversation as a whole to `<conversation ID>.pb`. Conversations in this format do not write running steps to the DB, so waiting cannot be shown and they are shown as working. The app opens the DB only when the `-wal` file exists, which shows that agy has opened it in WAL mode, so as not to trigger this switch. The DB of the Gemini desktop app has been confirmed to have the same table structure, but the behavior when the desktop app asks for approval has not been tested. The tool that asks you a question (`ask_question`) puts the session in waiting (question) as soon as it is called, like Claude Code's AskUserQuestion. This tool name was picked up from a string inside agy's executable, and it has not been confirmed to arrive.
- **Turns stopped while waiting for a response** If you stop agy while it is waiting for a model response, agy sends neither Stop nor PostInvocation, and leaves no stopped step in the conversation DB. This is the same before the first response of a turn and before a response after a tool call, as confirmed by pressing Esc in the agy 1.5.9 CLI. No steps are added while waiting for a response either, so a stopped conversation and a conversation where the model is thinking can only be told apart by how long no hook has arrived. So, in the same once-a-second check used for waiting, sessions that meet all of the following are translated to Interrupt and returned to idle.
  - The session is working and has no running subagents.
  - The last hook is UserPromptSubmit or PreInvocation. A stopped turn stops with one of these as its last hook. After PreToolUse a tool is running, and after a Stop with `fullyIdle` false (translated to PostToolUse) the session is waiting for background work, so these do not count.
  - The current turn (from the step of the last user input onward) has no PENDING (1), RUNNING (2), GENERATING (8), or WAITING (9) step. While a response is being generated, the response step is GENERATING. When agy has not opened the DB and it cannot be read, this condition is not checked.
  - 10 minutes have passed since the last hook.

  In the agy 1.5.9 conversation DB, the longest wait for a response without new steps was 46 seconds, and the longest time from starting to think to finishing the response step was 89 seconds. Returning a session to idle by mistake makes a running conversation disappear from the panel, so the margin is large, matching the time it takes to return a working session whose process has exited (about 10 minutes at most). Even if a session is returned to idle by mistake, the next PreToolUse or PostToolUse puts it back in working, and Stop in done. The agy of the Gemini desktop app does not keep the conversation DB open after a stop. It has been confirmed that a conversation stopped in the desktop app before its first response returns to idle after 10 minutes without the DB being read, but not whether the DB stays open in the middle of a turn.
- **Conversations delegated by Claude Code** If the hook's environment has `CLAUDECODE`, no state file is created, and any existing state file is deleted. agy passes its own environment on to its hooks, so the hooks of agy started from inside Claude Code (such as agy-async, to which Claude Code delegated work) see `CLAUDECODE=1`, which Claude Code sets in the processes it starts from tools. The delegating Claude Code session already shows that it is working, so the same work is not listed as a separate session.
- **Subagents** agy subagents send hooks with a conversation ID different from the parent's, and their input has no field pointing to the parent. The first line of a subagent's transcript is a `SYSTEM_MESSAGE` line carrying the parent's instructions, and its body contains `sender=<parent conversation ID>`. So only when this pattern appears in the first 2 KB of the transcript is the conversation treated as a subagent's, and its events are recorded as a subagent running inside the parent session. A subagent's transcript is not created until its first tool call, so events of a conversation whose transcript cannot be read and which has no state file yet are discarded. It has been confirmed that the transcript of a conversation you start is created before the first hook.
- **Activity summary** `run_command` is treated as Bash, and the command description that agy adds (`toolSummary`) is used for the summary. `read_file` and `view_file` are treated as Read; `write_file`, `write_to_file`, and `create_file` as Write; `edit_file`, `replace_file_content`, and `multi_replace_file_content` as Edit; `grep_search` as Grep; `search_web` and `read_url_content` as WebSearch and WebFetch; and `invoke_subagent` as Agent.
- **Context usage** The sum of `input_tokens` and `cache_read_tokens` in the transcript's last model response line is shown as the token count. agy writes the model's context limit neither to its hooks nor to the transcript, so no percentage is shown.
- **Title** The Gemini desktop app writes the title it gave a conversation to `annotations/<conversation ID>.pbtxt` in agy's data folder (two levels above `artifactDirectoryPath`) as `title:"..."` in protobuf text format. marimo reads this file at the start and end of each turn and uses it as the title. This format is not documented; it was confirmed in local files. The agy CLI does not create this file, so for CLI conversations the panel shows only the working folder's name.
- **End of a conversation** agy has no hook that reports the end of a conversation. marimo records the nearest non-shell ancestor of the hook (the `agy` process for the terminal CLI), and, as with other tools, the app deletes idle sessions whose process has exited and returns sessions left working to idle within about 10 minutes at most (see [When a process exits abnormally](#when-a-process-exits-abnormally)). Done and error sessions stay until marked as read and are deleted after 24 hours.
- **Origin** As with Claude Code, marimo records the terminal from the hook's environment variables and ancestor processes, and switches to that terminal when you click the session in the panel. agy sets the Ghostty terminal title to `agy` and has no conversation title, so in Ghostty marimo looks for the terminal whose tty matches the recorded one. For conversations in the Gemini desktop app, the Gemini app (`com.google.GeminiMacOS`) is recorded as the origin, so clicking the session brings the Gemini app to the front.
- **What is not recorded** Usage limits are not in the hook input, so the app reads them separately with `agy -p /usage` (see [What marimo runs](#what-marimo-runs)).

### How the launching app is identified

On macOS, the hook gets the app it was started from with the environment variable `__CFBundleIdentifier`. When this variable is missing, as with sessions running from the Codex desktop app, the hook walks the ancestor processes from the nearest one at session start and whenever a prompt is sent. If it finds an executable inside an app, it reads the bundle id from the `Info.plist` of the outermost app containing it and records it. Apps that cannot come to the front (those with `LSBackgroundOnly` in their `Info.plist`), such as claude.app, which contains the Claude Code CLI, are skipped, and the search continues with further ancestors. For Codex sessions where the app is still unknown, if `originator` in the `session_meta` at the start of the rollout is `Codex Desktop`, the conversation is recorded as started from the Codex desktop app (`com.openai.codex`). This value is not in Codex's documentation; it was confirmed in local rollouts.

The environment of the Claude Code CLI running from the Code tab of the desktop app contains `CLAUDE_CODE_HOST_SESSION_ID`. Its value is the id of the conversation inside the app, starting with `local_`, and it has been confirmed to be the same as `sessionId` in the conversation records that the app keeps in `~/Library/Application Support/Claude/claude-code-sessions`. The hook records it as `desktop_session`, and passes it to `claude://code/continue?session=` when you click the session in the panel. This variable is also not in Claude Code's documentation, so when the value is missing or has a different form (anything other than `local_` followed by up to 64 letters, digits, and hyphens), marimo only brings the app to the front.

### Click-through

When the window lets clicks pass through, mouse move events also stop reaching it. So the user interface sends the Rust side the areas that receive clicks (the rectangles of the panel and speech bubble, and a coarse grid 40 cells wide made from the alpha of the character image; the number of rows depends on the aspect ratio of the assets, 40×60 for Koharu), and the Rust side reads the cursor position periodically and lets the window receive clicks only when the cursor is over those areas. The cursor is read every 40 milliseconds when it is over the window and every 150 milliseconds when it is outside. While the window is hidden from the icon, the cursor is not read, and every 500 milliseconds the Rust side only checks whether the window has been shown again. Whether the mouse is over the character is also reported to the user interface along with the same check.
