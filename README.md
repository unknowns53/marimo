# marimo

English | [日本語](README.ja.md)

marimo is a desktop widget that shows the state of your AI coding agent sessions on screen. It currently supports five tools: Claude Code, Codex, Hermes Agent, OpenCode, and Antigravity.

A character standing in the corner of the screen uses facial expressions and speech bubbles to tell you whether each session is working, waiting for your approval, or done. The panel beside it shows the context usage of each session and the usage limits of your Claude Code, Codex, and Antigravity accounts. You can follow progress while doing other work, without switching to the agent's window.

<p align="center">
  <img src="docs/images/screenshot-detail.png" width="509" alt="The marimo window. A speech bubble above the character announces a session waiting for approval, and the panel on the left lists four sessions and the usage limits">
</p>

In this screenshot, the gaussian-scan session is waiting for permission to run a command, so the character waves and points it out in a speech bubble. Each session row is tinted by its state: one waiting (orange), two working (blue), and one done (green). Working sessions take a single line. The bottom row shows the usage limits of the whole account as bars for the 5-hour and 7-day windows. The screenshots show the Japanese interface; marimo also has an English interface (see [Requirements](#requirements)).

> [!NOTE]
> marimo is an unofficial tool made by volunteers. It is not affiliated with, or endorsed by, Anthropic or any other developer of the tools marimo supports. Claude and Claude Code are trademarks of Anthropic. Other product names, such as Codex, ChatGPT, Hermes Agent, OpenCode, Antigravity, and Gemini, may be trademarks or registered trademarks of their respective companies.

## Contents

- [What marimo shows](#what-marimo-shows)
- [Supported tools](#supported-tools)
- [Requirements](#requirements)
- [Installation](#installation)
- [Reporting bugs and contributing](#reporting-bugs-and-contributing)
- [License](#license)

For day-to-day use, customization, and development details, see the following documents.

| Document | What it covers |
| --- | --- |
| [Using marimo](docs/usage.md) | Starting and quitting, settings, reading the panel, jumping to a session, character expressions, usage limits, updating, troubleshooting, uninstalling |
| [Customizing marimo](docs/customize.md) | Changing the dialogue, making your own character, rebuilding the assets |
| [Developing marimo](docs/development.md) | Building from source, what `install` changes, files and data, privacy, tests, testing status, how it works |

## What marimo shows

### The character, the speech bubble, and the panel

The marimo window has three parts: the character, the speech bubble, and the panel.

<p align="center">
  <img src="docs/images/expressions.jpg" width="640" alt="The default character's expressions: idle, nine working expressions, waiting, done, error, and nine idle gestures">
</p>

- The **character** shows the overall state at a glance. Its expression changes with the situation (see [Character expressions](docs/usage.md#character-expressions)).
- The **speech bubble** appears above the character when a session is waiting, done, or has an error, and says in a short sentence which repository it concerns and what happened.
- The **panel** lists every session that is not idle, one per row. Each row shows the state color, the tool's icon, the repository name and chat title, the context usage, a short summary of the current activity, and the time since the last tool call (see [Reading the panel](docs/usage.md#reading-the-panel)).

Two characters come with marimo: Koharu and Clawd, the Claude Code mascot (see [Settings](docs/usage.md#settings)).

### States and their priority

marimo sorts sessions into the following five states.

| State | Meaning |
| --- | --- |
| Waiting | The agent is waiting for you to act |
| Error | The response failed partway |
| Working | The agent is thinking, using a tool, or running a subagent |
| Done | The response has finished and no subagent is running |
| Idle | Nothing is running |

Which events lead to which state is described in [How states are decided](docs/development.md#how-states-are-decided).

Done and Error stay visible while unread and collapse automatically once you mark them as read (see [Marking Done and Error as read](docs/usage.md#marking-done-and-error-as-read)).

When several sessions are active, the character shows the state with the highest priority. From highest to lowest, the priority is Waiting, Error, Working, Done, and Idle.

## Supported tools

Whatever the tool, `marimo-hook` picks up what is happening.

| Tool | How marimo receives the state |
| --- | --- |
| Claude Code | Hooks and statusLine |
| Codex | Hooks |
| Hermes Agent | Shell hooks |
| OpenCode | Plugin |
| Antigravity | Hooks |

How to register them is described in [Installation](#installation).

A hook is an external command that a tool calls when a particular event happens. statusLine is an external command that the Claude Code CLI (the command-line interface that runs in a terminal) calls to draw the line at the bottom of its screen; marimo reads context usage and usage limits from it.

What the panel can show, and where clicking a session takes you, differ by tool.

| Item | Claude Code | Codex | Hermes Agent | OpenCode | Antigravity |
| --- | --- | --- | --- | --- | --- |
| Title | Session title | Session title | Discord channel name or `DM` | Session title | Session title (not in the CLI) |
| Context usage | A percentage in the CLI; a token count in the Code tab of the desktop app | Percentage | Percentage (token count if the limit is unknown) | Percentage (token count if the limit is unknown) | Token count |
| Waiting | Shown | Shown | Shown | Shown | Shown |
| Usage limits | 5 hours / 7 days | The windows Codex reports | Not shown | Not shown | 5 hours / 7 days |
| Clicking a session opens | The terminal, or the conversation in the desktop app | The terminal, or the conversation in the desktop app | The Discord channel | The terminal | The terminal, or the conversation in the desktop app |
| Subagents | Shown as part of the main session | Shown as part of the main session | Shown as separate sessions | Shown as part of the main session | Shown as part of the main session |

Each session's icon tells you which agent tool is running it (see [Reading the panel](docs/usage.md#reading-the-panel)). What happens when you click a session is described in [Jumping to a session](docs/usage.md#jumping-to-a-session), and the usage limits display in [Usage limits](docs/usage.md#usage-limits).

## Requirements

marimo is tested on macOS, and most features are also tested on Windows 11. It has not been tested on Linux.

The interface and output are available in English and Japanese and follow your OS language. You can also switch the language in the settings window (see [Language](docs/usage.md#language)).

Claude Code 2.1.139 or later is required, because marimo registers its hooks in the exec form that this version introduced. For OpenCode, only v2 sessions run from a terminal are supported.

[Testing status](docs/development.md#testing-status) describes in detail how well each tool is supported.

## Installation

Steps 1 to 3 below are for the first installation. Later version updates use the script in step 4. Run the commands from the root of the repository.

### 1. Build

For the first installation, clone the repository and build it yourself. The required tools and their versions are listed in [Prerequisites](docs/development.md#prerequisites).

First, build `marimo-hook`, the command that each tool calls.

```bash
cargo build --release -p marimo-hook
```

Next, install the dependencies of the user interface and bundle the app with Tauri. `npm run tauri -- build` builds both the user interface and the Rust side.

```bash
cd app
npm install
npm run tauri -- build
cd ..
```

### 2. Register with each tool

`marimo-hook install` registers the Claude Code hooks and statusLine. It also registers the Codex hooks if a Codex folder exists, the OpenCode plugin if an OpenCode settings folder exists, and the Antigravity hooks if an agy settings folder (`~/.gemini/config`) exists.

Run it first with `--dry-run`. This shows the resulting settings.json and the changes without modifying any file.

```bash
./target/release/marimo-hook install --dry-run
```

If the changes look right, run it without `--dry-run`. `install` backs up settings.json before changing it (see [Registering hooks and statusLine](docs/development.md#registering-hooks-and-statusline)).

```bash
./target/release/marimo-hook install
```

After installing, restart any tools that are running. In Codex, also trust the marimo hooks with `/hooks` in the Codex CLI. `install` does not register Hermes Agent; register it by hand by following [Hermes hooks](docs/development.md#hermes-hooks).

### 3. Launch the app

On macOS, copy `marimo.app` to `/Applications` and open it. If macOS warns that the app cannot be opened the first time, follow [About the ad-hoc signature](docs/development.md#about-the-ad-hoc-signature).

```bash
cp -R target/release/bundle/macos/marimo.app /Applications/
open /Applications/marimo.app
```

On Windows, run `marimo_<version>_x64-setup.exe`, which is created in `target\release\bundle\nsis\`. The app is installed to `%LOCALAPPDATA%\marimo\marimo.exe`, and a shortcut is added to the Start menu.

When marimo starts, the character appears in the lower right of the screen, and an icon appears in the menu bar on macOS or in the notification area on Windows. To start marimo when you log in, open **Settings…** from the right-click menu and check **Launch at login** (see [Using marimo](docs/usage.md#starting-and-quitting)).

### 4. Update

Use the update script in the repository instead of overwriting the app with `cp`. The script downloads the latest stable release built by the GitHub Actions CI (a service that runs tests and builds automatically on every push) and replaces the app. You can also update from **Updates** in the settings window.

```bash
tools/update.sh
```

On Windows, run the following command in PowerShell 7.

```powershell
pwsh -ExecutionPolicy Bypass -File .\tools\update.ps1
```

Options for installing a prerelease or the latest build of main, and what happens when an update fails, are described in [Updating](docs/usage.md#updating). To remove marimo, see [Uninstalling](docs/usage.md#uninstalling).

## Reporting bugs and contributing

Please report bugs and suggest improvements in GitHub Issues. It helps if you include the following.

- Steps to reproduce
- Your OS (macOS or Windows) and its version
- Which tool (Claude Code, Codex, Hermes Agent, OpenCode, or Antigravity) it happened in, and its version
- Whether you use the terminal or the desktop app

If you attach state files or conversation logs, make sure they contain no personal information. Report security issues, such as those involving changes to settings.json, the placement of the OpenCode plugin, or reading conversation logs, privately through GitHub Security Advisories rather than in a public issue.

If you send a pull request, make sure all the commands in [Tests and checks](docs/development.md#tests-and-checks) pass, and add a test that checks the behavior when your change alters behavior.

## License

The code is available under your choice of the [MIT License](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE).

The character images and other pictures, and the dialogue, are published under the [Creative Commons Attribution 4.0 International License (CC BY 4.0)](https://creativecommons.org/licenses/by/4.0/). This covers `assets/` (except `assets/character/clawd/`), `art/`, `docs/images/`, `app/src-tauri/icons/`, and `app/src-tauri/shipped-dialogue/`. See [LICENSE-ASSETS](LICENSE-ASSETS) for details.

Koharu's images are based on images created with an OpenAI image generation model; `tools/character/build.py` composites the expression variants and removes the background.

The Clawd pixel art in `assets/character/clawd/` is unofficial fan art of Anthropic's Claude Code mascot, drawn by `tools/character/clawd.py` following the shape of the logo shown when Claude Code starts. This folder is not covered by CC BY 4.0. marimo grants no rights to Anthropic's designs or trademarks, and this art does not imply endorsement by or affiliation with Anthropic.

The menu bar and notification area icons, drawn by `tools/tray_icon.py`, and the app icon, drawn by `tools/app_icon.py`, are marimo's own pictures of a marimo ball.
