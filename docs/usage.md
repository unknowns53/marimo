# Using marimo

English | [日本語](usage.ja.md)

This document describes everyday use of marimo after installation. For installation, see the [README](../README.md#installation).

- [Starting and quitting](#starting-and-quitting)
- [Launching at login](#launching-at-login)
- [The menu bar and notification area icon](#the-menu-bar-and-notification-area-icon)
- [The right-click menu](#the-right-click-menu)
- [Settings](#settings)
- [Switching the display](#switching-the-display)
- [Reading the panel](#reading-the-panel)
- [Jumping to a session](#jumping-to-a-session)
- [Marking Done and Error as read](#marking-done-and-error-as-read)
- [Speech bubbles](#speech-bubbles)
- [Character expressions](#character-expressions)
- [Interacting with the character](#interacting-with-the-character)
- [Usage limits](#usage-limits)
- [Updating](#updating)
- [Troubleshooting](#troubleshooting)
- [Uninstalling](#uninstalling)

## Starting and quitting

When marimo starts, the character appears in the lower right of the screen, and a marimo-shaped icon appears in the menu bar on macOS or in the notification area on Windows (see [The menu bar and notification area icon](#the-menu-bar-and-notification-area-icon)). marimo does not appear in the Dock or the app switcher (Cmd+Tab) on macOS, or in the taskbar on Windows. Its window always stays in front of other windows. If you start marimo again while it is running, the second instance exits without doing anything, and the first one shows its window again if you had hidden it from the icon.

To quit, choose **Quit** from the right-click menu or the icon's menu.

## Launching at login

On macOS, open `marimo.app` from `/Applications`; on Windows, open `%LOCALAPPDATA%\marimo\marimo.exe` installed by the installer. Then check **Launch at login** under **Startup** in the [settings](#settings) window. It is off by default.

When it is on, marimo creates a LaunchAgent (a settings file that macOS uses to start a program at login) at `~/Library/LaunchAgents/com.marimo.desktop.plist` on macOS, and registers itself under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` in the registry on Windows. Both point to the path of the executable that was running when you turned it on.

If you start marimo while launching at login is on, it registers the path of the running executable again. So if you moved `marimo.app` elsewhere, or turned the setting on while running `target\release\marimo.exe`, starting the installed marimo once makes it the one that starts at your next login. When started from `target` in the source tree, marimo does not register again, so that a trial run does not change the registration.

## The menu bar and notification area icon

marimo shows a marimo-shaped icon in the menu bar on macOS and in the notification area on Windows. Left-click the icon to hide the character and panel window, and click it again to show it. marimo keeps tracking states while hidden, and shows the current state when the window comes back. On macOS, showing the window does not take keyboard focus away from the app you are using. The hidden state is not saved, so marimo always starts with its window shown.

Right-click the icon to open the following menu. When sessions need your attention, they are listed at the top of the menu, followed by a separator and the items in the table.

| Item | What it does |
| --- | --- |
| Waiting, Error, and Done sessions | Lists waiting sessions, then unread error sessions, then unread done sessions, with the most recently updated first within each state. Each item shows the state, the same name as in the panel, and the conversation title if known (shortened if long). Choosing one jumps to the session and marks it as read, just like clicking it in the panel (see [Jumping to a session](#jumping-to-a-session)). If there are no such sessions, neither the list nor the separator is shown |
| N More | The list shows up to 10 sessions; this item gives the number left out when there are more. It cannot be chosen |
| Hide marimo, Show marimo | Hides or shows the window. Only the one that applies to the current state is shown |
| Show Character | Turns the character and speech bubbles on or off (see [Switching the display](#switching-the-display)) |
| Show Details, Show Counts Only | Switches how the panel is shown (see [Switching the display](#switching-the-display)) |
| Sort Order | Chooses how the panel orders sessions from a submenu of **Start Time**, **Status**, and **Last Update** (see [Sort order](#sort-order)) |
| Mark All as Read | Marks every unread done and error session as read. It cannot be chosen when there are none (see [Marking Done and Error as read](#marking-done-and-error-as-read)) |
| Update Available: X | Appears only when a newer stable version exists. Choosing it opens the settings window at **Updates** (see [Checking for updates](#checking-for-updates)) |
| Settings… | Opens the settings window (see [Settings](#settings)) |
| Quit | Quits marimo |

The icon is colored only when some session needs your attention. When there are both waiting and error sessions, the icon shows waiting, following the same priority as the character's expression. An error you have marked as read no longer affects the icon, just as in the panel.

| Icon | State |
| --- | --- |
| A green marimo with an orange "!" | Some session is waiting |
| A green marimo with a red "!" | No session is waiting, and some session has an error |
| All black on macOS; a green marimo on Windows | Neither |

While the window is hidden, the icon and the session list in its menu keep following the states, so you can notice changes and jump to a session without showing the window. On Windows, the icon may end up in the hidden part of the notification area (the area shown by clicking `^` on the taskbar).

## The right-click menu

Right-click the character or the panel to open the icon's menu without the session list, **N More**, **Hide marimo**, and **Show marimo**. Each item works the same as in the table in [The menu bar and notification area icon](#the-menu-bar-and-notification-area-icon). Other settings are in the [settings](#settings) window.

## Settings

Choose **Settings…** from the right-click menu or the icon's menu to open the "marimo Settings" window. If it is already open, it comes to the front. This is an ordinary window with a frame, and you close it with its close button. Changes made in the window apply to the character and panel immediately, and are saved for the next start (see [Files and data](development.md#files-and-data) for where). If you change the display from a menu or a panel button while the window is open, the window follows.

The left side of the window lists six sections: **Display**, **Bubbles**, **Character**, **Usage limits**, **Startup**, and **Updates**. The settings of the selected section appear on the right. **Display** is selected when the window opens.

| Section | Setting | What it does |
| --- | --- | --- |
| Display | Language | Chooses the interface language, **日本語** or **English**. Until you choose one, marimo follows the OS language (see [Language](#language)) |
| Display | Show character | Turns the character and speech bubbles on or off (see [Switching the display](#switching-the-display)) |
| Display | Panel | Chooses how the panel is shown: **Details** or **Counts** (see [Switching the display](#switching-the-display)) |
| Display | Sort order | Chooses how the panel orders sessions: **Start time**, **Status**, or **Last update** (see [Sort order](#sort-order)) |
| Display | Background opacity | Changes how solid the panel background is with a slider (see [Background opacity](#background-opacity)) |
| Bubbles | Show for (Waiting, Done, Error), Display time | Chooses which states show speech bubbles and how long until they close on their own (see [Speech bubbles](#speech-bubbles)) |
| Character | Character | Chooses the character. The change takes effect immediately. If switching fails, the previous character stays |
| Character | Size | Changes the size of the character with a slider (see [Size](#size)) |
| Usage limits | Show for (Claude Code, Codex, Antigravity) | Chooses which tools' values appear in the usage limits row. All are shown by default. Tools you uncheck are not asked to refresh their usage limits (see [Usage limits](#usage-limits)) |
| Usage limits | Refresh interval | Chooses how often the checked tools are asked to refresh their usage limits: **Never**, **Every 5 min**, **Every 10 min**, **Every 15 min**, **Every 30 min**, or **Every 60 min**. The default is **Never** (see [Usage limits](#usage-limits)) |
| Startup | Launch at login | Turns launching at login on or off. It is off by default (see [Launching at login](#launching-at-login)) |
| Updates | Current version, latest release, last checked | Shows the version of the running marimo and the latest stable release found at the last check, and updates marimo if a newer version exists (see [Checking for updates](#checking-for-updates)) |

### Language

Until you choose a language, marimo uses Japanese if your OS language is Japanese, and English otherwise, and **Language** shows the language in use. The OS language is the first of your preferred languages under Language & Region in System Settings on macOS, and the display language on Windows. Choosing **日本語** or **English** uses that language regardless of the OS language. The choice takes effect immediately and is saved as `language` in `display.json` (see [Files and data](development.md#files-and-data)). To follow the OS language again, quit marimo, delete `language` from `display.json`, and start marimo.

The chosen language is used for the following.

- The text of the character, panel, speech bubbles, right-click menu, icon's menu, and settings window
- The default speech bubble dialogue (see [Changing the dialogue](customize.md#changing-the-dialogue))
- The activity summary on the second line of each panel row. A summary is written in the language in effect when the hook writes it to the state file, so summaries written before you change the language stay in the previous language until the session's next hook arrives.
- The output of `marimo-hook install` and `uninstall`, and of the update scripts

If you change the OS language before choosing a language, marimo's interface switches at its next start. Hooks, `marimo-hook`, and the update scripts read the OS language every time they run.

### Checking for updates

marimo checks GitHub for a newer stable version 3 seconds after starting and once a day after that. When a newer version exists, an item such as **Update Available: 0.1.7** appears above **Settings…** in the icon's menu, and the same text appears under **Updates** in the list on the left of the settings window. If the check fails, nothing is added to the icon's menu, and the reason is shown under **Updates** in the settings window.

**Updates** in the settings window shows the current version, the latest release, and when marimo last checked. Click **Check for Updates** to check again right away. Only when a newer version exists does a button such as **Update to 0.1.7** appear; clicking it opens a confirmation dialog. Click **Update** in the dialog to start the update, and its progress appears below the button. When the update finishes, marimo restarts with the new version. If an even newer stable release has been published since the dialog opened, marimo does not update and asks you to check again. If the update fails, the reason appears below the button. How this update works is described in [Updating from the settings window](#updating-from-the-settings-window).

### Background opacity

The **Background opacity** slider changes how solid the panel background is, from 20% to 100% in steps of 5%. The default is 80%. The panel background changes as you move the slider, and the current value, such as "80%", appears to the right of the slider. The value is saved when you release the mouse button.

Only the backgrounds of the panel and the row of buttons along its top edge become transparent; text, icons, and state color dots do not. The details that appear over the counts row when you point at it become more transparent along with the panel, but they stay more solid than the panel so that they do not blend with the counts row and button text underneath. When the panel is at 80% or more they are opaque, and at their most transparent they stay at 85%. Speech bubbles do not change.

### Size

The **Size** slider changes the scale of the character from 0.6× to 4.0× in steps of 0.1. The default is 1.0×, and the current scale, such as "1.0×", appears to the right of the slider. You can also change it over the same range in steps of 0.1 by turning the mouse wheel over the character while holding Cmd (Ctrl on systems other than macOS). The size of the character area at a scale of 1.0 is given in [Making your own character](customize.md#making-your-own-character).

When you change the size, the corner of the window where the character sits stays in place, and any part that would go off screen is moved back inside. The text size of the panel and speech bubbles does not change.

## Switching the display

**Show Character** and the panel's **Show Details** and **Show Counts Only** can be chosen independently. By default, the character is shown with the detailed panel. The panel is shown in every combination.

These and **Sort Order** can be changed from the right-click menu, the icon's menu, or **Display** in the settings window, and the same setting is saved wherever you change it. Every menu shows a check mark next to the current choice, and the settings window also shows the current state. Choices made in the icon's menu are applied and saved by the app immediately, so they are not lost even if you choose right after startup or several times in quick succession. You can also switch between details and counts by clicking the character, and change the sort order with the button at the top left of the panel (see [Sort order](#sort-order)).

The detailed panel shows parts of file names and commands, so switch to **Show Counts Only** when you show your screen to others. The counts row shows neither file names nor commands, and the details appear on top of it only while you point at it. To hide the window entirely, click the menu bar icon.

Turning off **Show Character** hides the character and speech bubbles. The window size and panel position do not change. Right-click an empty part of the panel to open the menu, and drag it to move the window. While the character is hidden, no speech bubbles are shown; when you turn **Show Character** back on, a bubble appears if the state it is about still continues. Turning the wheel over the character is not available, so change the size with **Size** in the settings window.

## Reading the panel

The **Details** panel lists every session that is not idle, one per row. By default, sessions are ordered by start time, newest at the top (see [Sort order](#sort-order)). Each session takes two lines.

- **First line** A state color dot, the name, the chat title, and the context usage. The name is the name of the git repository containing the working folder, or the working folder's name outside a repository. Inside a git worktree, the name of the original repository is shown rather than the worktree folder's name. The title follows the name in small, faint text, and is shortened first when space runs out. Sessions with no known title show only the name.
- **Context usage** When the limit is known, a small bar and a percentage are shown, and the bar changes color at 80% or more. When the limit is unknown, the token count is shown as a number in a box (for example, `45.2k`) so that it is distinguishable from the bar. marimo does not guess the limit from the model name. Sessions in the Code tab of the desktop app show a token count because the limit is unknown, but once statusLine reports the limit for the same conversation opened in the CLI, a percentage is shown from then on.
- **Second line** A one-line summary of the current activity. For shell commands, it is the description the agent gave the command, or the command itself if there is no description; for reading and writing files, the path relative to the working folder; while responding, the beginning of the text. While subagents run, their number appears in small text before the summary, such as "Sub 3". A subagent whose SubagentStop never arrived stays counted until 30 minutes pass or the session returns to idle.
- **Time since the last update** To the right of the second line, below the context usage, the time since a hook (or, for OpenCode, a notification from the plugin) last arrived for the session is shown, such as "now", "5m ago", or "2h ago". The panel redraws every 30 seconds, so nothing finer than a minute is shown. Point at it to see the time the last hook arrived. statusLine input does not update this time.
- **Tool icon** To the left of the second line, below the state dot, an app icon or a text mark shows which tool the session belongs to. The icons of Claude Code, Codex, and Antigravity are not bundled; marimo reads them from each tool's desktop app installed locally (see [What marimo reads](development.md#what-marimo-reads)). The icons of Hermes Agent and OpenCode are bundled from the files each project distributes under the MIT License (see `LICENSE-ASSETS` for sources).

  | Tool | Icon | Mark when the icon cannot be read |
  | --- | --- | --- |
  | Claude Code | The Claude desktop app's icon | `CC` |
  | Codex | The Codex desktop app's icon. On macOS, the Codex app is installed as ChatGPT.app, so the ChatGPT icon may appear | `CX` |
  | Hermes Agent | The bundled Hermes Agent icon | None (the icon is always shown) |
  | OpenCode | On macOS, the OpenCode desktop app's icon. Without it, and on Windows, the bundled OpenCode icon | None (the icon is always shown) |
  | Antigravity | On macOS, the icon of the Gemini desktop app, which bundles Antigravity | `AG` |

- **Tooltip** Point at a session to see the chat title, the full path of the working folder, the full summary, and the command or arguments themselves.

How the title and context usage are obtained differs by tool, and is described in [How titles and context usage are read](development.md#how-titles-and-context-usage-are-read) and in the "What ... records" section for each tool. Titles in the Code tab of the desktop app are read from an undocumented format, so they may stop appearing after a Claude Code update.

There is no limit to the number of sessions listed. As sessions are added, the panel grows upward and the character stays in place. When the character is in the upper half of the screen, the panel aligns its top with the character's and grows downward (see [Layout by screen position](#layout-by-screen-position)). When the panel grows close to the edge of the window, it stops growing and becomes scrollable with the mouse wheel. Growing downward, it stops just short of the bottom of the window; growing upward, it stops below the top of the window by the height of the speech bubble so that it does not overlap the bubble while the character is shown. A thin scrollbar appears at the right edge of the panel. The scroll position is kept when the panel redraws, and moves to the bottom when sessions decrease and the position can no longer be reached. When there are no sessions to list, a faint line reading "No active sessions" is shown.

Below the sessions and above the usage limits row, a faint line such as "Read today: 3" shows how many sessions became done or errored today and have been marked as read. Click it to list those sessions faintly, newest state change first, and click it again to close the list. Clicking a session in the list jumps to it like any other session. The "✓" button for marking as read is not shown there. Whether the list is open is not saved, and it starts closed. When no done or error session has been marked as read today, this line is not shown (see [Marking Done and Error as read](#marking-done-and-error-as-read)).

<p align="center">
  <img src="images/screenshot-counts.png" width="516" alt="The counts-only display. The panel is collapsed to a single line, 'Waiting 1 · Working 2 · Done 1', and the usage limits row">
</p>

The **Counts** panel shows the number of sessions in each state on a single counts row, together with the color dots (for example, "Waiting 1 · Working 3 · Done 2"). States with no sessions are left out. When there is nothing to count, it shows "No active sessions". Clicking the counts row jumps to the waiting, error, or done session with the highest priority.

Point at the counts row to see the detailed panel on top of it. These details have no limit on the number of sessions either, and scroll when they do not fit in the window. While the mouse stays over the details, they do not close, even while you scroll. The details also include the "Read today" line below the sessions.

### Sort order

At the top left of the panel is a button with the name of the current sort order; each click switches between **Start Time**, **Status**, and **Last Update** in that order. Point at it to see a tooltip that explains the current order and what the next click switches to. The button is shown even when there are no sessions. You can also choose from **Sort Order** in the right-click menu, the icon's menu, or the settings window. The chosen order applies both to the detailed panel and to the details shown over the counts row.

| Sort order | What it does |
| --- | --- |
| Start Time | Shows sessions with the latest start time at the top. This is the default |
| Status | Orders sessions from highest to lowest state priority (see [States and their priority](../README.md#states-and-their-priority)), with the most recently updated first within the same state. Done and error sessions that you marked as read and that stay faintly in place are ordered by the same rule as unread ones, so they do not move when you mark them as read |
| Last Update | Shows the most recently updated sessions at the top |

With **Start Time**, sessions do not change places when their state changes, and new sessions are added at the very top. While the panel keeps its bottom fixed and grows upward, existing sessions also keep their positions on screen. In layouts that keep the top fixed and grow downward, adding a new session moves the existing ones down by one row. A session with no recorded start time is placed at the bottom until its next event arrives. With **Status** and **Last Update**, sessions change places whenever their state or update time changes. In every order, sessions whose times are equal and whose order cannot be decided do not swap places on each redraw.

## Jumping to a session

Click a session in the panel to switch to the window or tab where it is running. For Claude Code, Codex, and Antigravity sessions, marimo switches to the origin that the hook recorded. Hermes Agent and OpenCode sessions are described after the tables. On macOS, it works as follows.

| Where the session runs | How marimo switches to it |
| --- | --- |
| Terminal.app, iTerm2 | Finds the tab with the matching TTY (the terminal's device name) with AppleScript and brings it to the front |
| Ghostty | Finds the tab with the TTY the hook recorded with AppleScript and brings it to the front. If none matches the TTY, or an older Ghostty does not report the TTY, it looks for a tab whose terminal title matches the conversation title; failing that, if exactly one tab has the same working folder, it chooses that one. Claude Code background sessions started from Ghostty also fall into this case. Antigravity agy conversations were also tested here |
| The integrated terminal of VS Code | Brings the window with the working folder open to the front. For editors derived from VS Code, opens the recorded app |
| The Code tab of the desktop app | Opens the app with the URL `claude://code/continue?session=<conversation id>` and goes to that conversation. The conversation id is `CLAUDE_CODE_HOST_SESSION_ID`, recorded by the hook. If the conversation cannot be opened, for example because it is archived, the first screen of the Code tab appears |
| Codex in the ChatGPT desktop app | Opens the app with the URL `codex://threads/<conversation id>` and goes to that conversation |
| The Gemini desktop app | Brings the Gemini app to the front |
| Anything else | Brings the recorded app to the front. Does nothing if the app is unknown |

The two URLs that open conversations in the desktop apps are not published specifications but routes found in the apps' code. If an app update breaks them, the app is only brought to the front. In the Claude app, marimo goes to the conversation only when you are signed in and your organization's settings have not disabled opening from URLs (deep links).

When choosing a tab in Terminal.app, iTerm2, or Ghostty, macOS asks the first time for permission to let marimo use automation. If you do not allow it, marimo stops after bringing the app to the front.

On Windows, it works as follows. Sessions in the Code tab of the desktop app go to the conversation by URL, as on macOS. If the hook's environment had no `CLAUDE_CODE_HOST_SESSION_ID`, the desktop app's window is brought to the front by the "Anything else" method in the table.

| Where the session runs | How marimo switches to it |
| --- | --- |
| Windows Terminal | Brings the window that has the session's tab to the front. Windows Terminal has no public way for another program to select a tab, so the tab itself is not selected |
| The classic console (conhost) of Command Prompt or PowerShell | Brings that console window to the front |
| The integrated terminal of VS Code and its derivatives (Cursor, VSCodium, Windsurf, and so on) | Passes the working folder to the editor and brings the window with that folder open to the front |
| Anything else | Walks up the ancestor processes of Claude Code or Codex from the nearest one, and brings the window of the first app found to the front. It does not go above File Explorer |

At session start and whenever a prompt is sent, the hook records the console window and the ancestor processes in which Claude Code or Codex runs. If the recorded window or process has since closed or been replaced by something else, that window is not used. Windows restricts other apps' windows from coming to the front, so the taskbar button may only flash instead. marimo also cannot switch to an app that runs as administrator (see [Clicking a session on Windows does not switch to it](#clicking-a-session-on-windows-does-not-switch-to-it)).

Hermes Agent sessions open the chat where the conversation takes place, on any OS. For Discord conversations, the channel or DM opens in the Discord app, and on Windows the Discord window is brought to the front. For sessions with no known destination, such as CLI conversations, clicking only marks them as read (see [What the Hermes hooks record](development.md#what-the-hermes-hooks-record)).

For OpenCode sessions, marimo looks for the OpenCode screen running in Ghostty on macOS and brings it to the front. OpenCode sets the terminal title to `OC | <title>`, so marimo first looks for a terminal with that title, and failing that, chooses the terminal with the same working folder if there is exactly one. If neither is found, marimo does not bring Ghostty to the front either, and only marks the session as read. The OpenCode service that runs conversations is a separate process from the terminal and cannot record which terminal started a conversation, so marimo does not switch to other terminals (see [What the OpenCode plugin records](development.md#what-the-opencode-plugin-records)). When choosing a Ghostty terminal, macOS also asks the first time for permission to let marimo use automation.

## Marking Done and Error as read

A session stays done until the next prompt is sent. When a session ends (SessionEnd), marimo removes it from the list, but it keeps done and error sessions unless you ended the conversation yourself in its own screen, as with `/exit` or `/clear`. The Claude desktop app may stop idle sessions in the background or reopen conversations when memory runs low, and marimo keeps these sessions so that unread results do not disappear without notice. When you delete a conversation in OpenCode, it cannot be reopened, so marimo removes done and error sessions as well.

Done and error sessions are marked as read, and collapsed in the panel, by any of the following. Read done and error sessions count as idle for the character's expression and speech bubbles, and are not counted in the counts-only display.

- Click the session in the panel to jump to it.
- Click its speech bubble. This jumps to the session, just like clicking it in the panel.
- Choose **Mark All as Read** from the right-click menu or the icon's menu. This marks all unread done and error sessions as read at once.
- Point at an unread done or error session and click the "✓" button that appears at its right edge, two lines tall. While the button is shown, the session's context usage and time since the last update are hidden.
- Click a session or speech bubble while holding Option (Alt on systems other than macOS). This marks it as read without jumping. Doing this on the counts row in the counts-only display marks the session that a click would have jumped to as read, without jumping.

The last three do not bring any app to the front.

A waiting session does not disappear when clicked; it stays until it is approved or its state changes. Clicking it with Option held only closes its speech bubble.

A session marked as read while the mouse is over the panel stays faintly in place until the mouse leaves the panel. If it collapsed immediately, the sessions above or below would shift, and when you click several "✓" buttons in a row, the next session you meant to click would move out from under the mouse. Sessions marked as read while the mouse is not over the panel collapse immediately.

Of the collapsed done and error sessions, those that reached their state today are shown when you click the "Read today" line at the bottom of the panel (see [Reading the panel](#reading-the-panel)). Earlier ones are not shown.

The read records are saved in `~/.marimo/acknowledged.json`, so the same done or error is not announced again after restarting the app. Sessions marked as read before a restart start collapsed after it. A record lasts only while its trigger (which session, since when, and in which state) continues. When the same session becomes done or errors again, it appears as a new, unread done or error.

## Speech bubbles

By default, a speech bubble stays on screen while the state it announces remains the overall state: until approval for waiting, until the next prompt for done, and until the state changes for errors. The line does not change while the same trigger lasts.

Click a speech bubble to jump to its session, just like clicking the session in the panel. The bubble closes and does not reappear while the same session stays in the same state. Click it while holding Option (Alt on systems other than macOS) to close it without jumping. When several sessions qualify, the bubble is about the session chosen as the overall state, and switches to the next session once that one is resolved.

The speech bubble appears just above the character area. For short characters such as Clawd, a bubble with a long line could overlap the panel, so only then is it lifted above the top of the panel.

**Bubbles** in the settings window offers the following two settings.

| Setting | What it does |
| --- | --- |
| Show for (Waiting, Done, Error) | States you uncheck show no speech bubble. Panel sessions and the character's expression do not change. Bubbles of an unchecked kind are not marked as read, so if you check it again later, they appear while the state continues. All are shown by default |
| Display time | Chooses how long until a bubble closes on its own: **Until dismissed**, **10 sec**, **30 sec**, **1 min**, or **5 min**. The default is **Until dismissed**, which keeps the bubble until you click it. If you set another number of seconds by editing `display.json` by hand, that value also appears in the list. A bubble closed by the timer is not treated as read, unlike one you click, and done sessions do not turn faint. It does not reappear while the same trigger lasts. The timer does not count while the character is hidden, and starts over when the character is shown again |

How to change the lines is described in [Changing the dialogue](customize.md#changing-the-dialogue).

## Character expressions

The character's expression follows the overall state (see [States and their priority](../README.md#states-and-their-priority)). While idle and working, it makes small movements such as blinking and breathing. Expressions change with a crossfade (fading out the previous picture while fading in the next), except for blinks.

While working, Koharu's face changes with the tool in use: a seriously focused face, a happy smile when the code just written works, a face following the text with glasses on, a curious look while investigating, a sideways glance to check the steps, a face listening carefully, and a wink that says "leave it to me". While thinking or writing a response without a tool, Koharu looks thoughtful, and with any other tool, reads a document held in hand. Which tool maps to which face is listed in `rules.working_tools` in [Making your own character](customize.md#making-your-own-character). While idle, about once every 20 to 60 seconds, Koharu makes a gesture for about 5 to 8 seconds (looking away, tucking hair behind an ear, yawning, taking a break with a mug, resting chin on hand, stretching, or humming) or for about 8 to 12 seconds (reading a paperback or dozing off).

The faces for done, waiting, and error can vary within a state, using the same categories as the speech bubble lines. Koharu closes both eyes and stretches when a task that took 15 minutes or more finishes (`done.long`), and takes a break with a mug when work stops at a usage limit (`error.rate_limit`). The list of categories is in [Changing the dialogue](customize.md#changing-the-dialogue), and how to assign expressions is in `rules.variants`.

Clawd changes expressions the same way as Koharu, and its working face changes with the tool in use. When waiting, it raises one hand with a "!"; when done, it raises both hands with sparkles; while idle, it looks left and right, hops, sleeps, or stretches. The pixel art is drawn so that it stays sharp at any size, and it does not do the breathing motion.

## Interacting with the character

- **Click** Switches the panel between the detailed and counts-only displays. As when you choose from a menu, the chosen display is saved (see [Switching the display](#switching-the-display)).
- **Point** Pointing at the character while working makes it look at you. When the mouse leaves, the original expression returns after a short pause.
- **Drag** Press and move a little to move the whole window. The position is saved in `~/.marimo/window.json`, and the window appears in the same place next time. If the saved position ends up off screen, for example because the monitor setup changed, the window returns to the lower right of the screen.

### Layout by screen position

Depending on whether the center of the character is in the left or right half of the screen, and in the upper or lower half, marimo chooses one of the four corners of the window for the character and changes how the character and panel are arranged.

| Character position | Arrangement of the character and panel |
| --- | --- |
| Lower right | The panel is to the left of the character, aligned at the bottom and growing upward. This is the default |
| Lower left | The panel is to the right of the character, aligned at the bottom and growing upward |
| Upper right | The panel is to the left of the character, aligned at the top and growing downward |
| Upper left | The panel is to the right of the character, aligned at the top and growing downward |

The panel always appears on the side of the character facing the inside of the screen, so it never extends off screen even when you move the window to an edge. The speech bubble appears above the character's head, extending toward the panel's side.

The arrangement switches about 0.4 seconds after you stop dragging and the window comes to rest. The character stays in the same place on screen, and only the panel moves to the other side. The chosen corner is saved in `~/.marimo/window.json` together with the window position.

Clicks on transparent parts of the window pass through to the window underneath. Only the opaque parts of the character, the panel, and the speech bubble receive clicks. When **Show Character** is off, only the panel does.

## Usage limits

The panel shows the usage limits of Claude Code, Codex, and Antigravity. Choose which tools to show with the **Show for** checkboxes under **Usage limits** in the settings window. Tools you uncheck show no values and are not refreshed.

Where the values come from and when they update differs by tool, as follows.

| Tool | Source | When it updates |
| --- | --- | --- |
| Claude Code | statusLine input, the result of `/usage` | When a new response arrives in a CLI session, when you run `/usage`, and when refreshed |
| Codex | Codex's records (rollout) | When a Codex hook reads the record, and when refreshed |
| Antigravity | The result of `agy -p /usage` | Only when refreshed |

When you choose a **Refresh interval** in the settings window, marimo asks each tool whose saved values are older than the interval to fetch them again. Refreshing sends no request to a model and creates no conversation. It is not done while the tool is working and new values are arriving. The default is **Never**. The commands marimo runs are described in [What marimo runs](development.md#what-marimo-runs).

statusLine is not called in the Code tab of the desktop app, so if you use only the Code tab, choose a **Refresh interval**. Antigravity's values are updated only by refreshing, so with **Never** they do not appear.

Antigravity's usage limits are split by model group, and marimo shows the 5-hour and weekly values of the Gemini model group (Gemini Flash, Gemini Pro).

### The usage limits row

At the bottom of the panel, the 5-hour and 7-day usage, such as `5h 23% · 7d 84%`, and the last update time (for example, `Updated 14:05`) are shown. There is one row for the whole account, and it is shown even when every session is idle. When there are no values to show, the row is not shown.

When there are Codex or Antigravity values, the values are listed in the order Claude Code, Codex, Antigravity, each preceded by the app's icon (or the text `CC`, `CX`, or `AG` if the icon cannot be read). For example, `5h 12% · 7d 30%` follows the Claude icon, `7d 13%` the Codex icon, and `5h 10% · 7d 3%` the Antigravity icon. When they do not fit in the width of the panel, they wrap between tools, and the update time stays at the right edge. When only Claude Code's values are shown, no icon is added.

Codex window names are decided from the window length that Codex reports: `7d` if it divides evenly into days, `5h` if it divides evenly into hours, and otherwise the number of minutes (for example, `90m`). A window whose length was not reported is shown as a percentage without a name.

The time at the right edge is the newest update time among the values shown. Point at a value to see its own update time in a tooltip. The update time is when the value arrived from statusLine or was obtained with `/usage` for Claude Code, when the value was written to the record or queried by marimo for Codex, and when `agy -p /usage` ran for Antigravity.

Values 30 minutes or more past their last update are likely to have drifted from the actual values, so they are shown faintly. Staleness is judged for each value separately. If you set the **Refresh interval** longer than 30 minutes, Antigravity's values are shown faintly during the latter part of each interval. Values for windows whose reset time has passed are not shown.

## Updating

The update scripts `tools/update.sh` and `tools/update.ps1` from [4. Update](../README.md#4-update) in the README download the build that the GitHub Actions CI made and published as a GitHub release, and replace the app. Nothing is built locally, so Rust and Node.js are not needed.

On Windows, run `pwsh -ExecutionPolicy Bypass -File .\tools\update.ps1` in PowerShell 7. `-ExecutionPolicy Bypass` relaxes the execution policy for this command only, so that the script runs even where running scripts is prohibited by default. Without PowerShell 7, you can replace `pwsh` with `powershell`, the Windows PowerShell 5.1 that comes with Windows.

By default, the scripts install the latest stable release. To install something else, add the following options.

| What to install | macOS | Windows |
| --- | --- | --- |
| A prerelease | `--pre` | `-Pre` |
| The CI build of the latest commit on main | `--main` | `-Main` |
| The same, waiting up to 30 minutes for the CI build to finish | `--main --wait` | `-Main -Wait` |
| The CI build of the commit currently checked out | `--no-pull` | `-NoPull` |
| A local build | `--build` | `-Build` |

The macOS options are used below. On Windows, read them as the corresponding options in the table (`-NoPull -Build` for `--no-pull --build`, `-Wait` for `--wait`). `--pre` cannot be combined with `--main`, `--no-pull`, or `--build`, and `--main` cannot be combined with `--no-pull`. `--wait` can only be used together with `--main` or `--no-pull`. The CI builds only for arm64 Macs (Apple silicon Macs) and x64 Windows. On an Intel Mac, build locally with `--build`.

The script updates in the following steps.

1. It decides which release to install. By default, it looks up the latest stable release, named like `v0.1.0`, with the GitHub API and shows its name. In this case, it does not pull the repository or check the branch or local changes. If there is no stable release at all, it suggests `--main` and exits without changing anything. With `--pre`, it uses the most recently published release whose name starts with `v`, including prereleases. With `--main`, it pulls the latest main with `git pull --ff-only` and uses the release named `build-<12 characters>`, made from the first 12 characters of that commit's hash.
2. From the chosen release, it downloads `marimo-macos-arm64.tar.gz` on macOS or `marimo-windows-x64.zip` on Windows, together with `SHA256SUMS`, which lists the checksums, into a temporary folder. If the checksum does not match, it stops without changing anything. If the repository's `origin` points to a GitHub repository, the script looks for releases there, and otherwise in marimo's original repository, so it also works from a fork.
3. It quits the running marimo. On macOS, if marimo has not quit after about 10 seconds, the script stops without replacing the app. On Windows, it continues even if marimo has not quit and leaves closing the app to the installer. If the installer cannot close it either, the app is not replaced and the failure is shown.
4. It replaces the app. On macOS, it copies the new `marimo.app` into `/Applications` under another name and then swaps it with the old `/Applications/marimo.app`. On Windows, it runs the NSIS installer (a system for making Windows installers) without showing its screens (`/S`) to reinstall `%LOCALAPPDATA%\marimo\marimo.exe`.
5. It runs `install` with the downloaded `marimo-hook`, replacing `~/.marimo/bin/marimo-hook` (`marimo-hook.exe` on Windows) with the new one. `install` copies itself to this location, so the hooks keep working even though it runs from a temporary folder.
6. It restarts marimo and deletes the temporary folder.

With `--main`, before pulling, the script checks that the current branch is `main` and that the files tracked by Git have no uncommitted changes. If either check fails, it shows the reason and exits without doing anything. Untracked files are fine. With `--no-pull`, it skips the checks and the pull, and downloads the build of the commit currently checked out. The CI builds only commits pushed to main, so this works only for commits that went through main. Commits that change only files not included in the app, such as documents, are not built, so when using a `build-` release, the script downloads the build of the last commit that changed anything else (see [Tests and checks](development.md#tests-and-checks)).

### When an update fails

Downloads can fail for a few dozen seconds right after a release is published, so the script retries every 10 seconds for up to 1 minute. When using a `build-` release, if the build is still missing after retrying for 1 minute, the script says that the CI has not finished building that commit or that the build failed, shows the address of the list of GitHub Actions runs, and exits with a non-zero exit code without touching the app. With `--wait`, it checks every 30 seconds for up to 30 minutes and continues once the build is ready. If it is still not ready after 30 minutes, it shows the same message and exits.

On Windows, if the connection drops before a response arrives, the script retries up to 3 times, 5 seconds apart. If downloading fails, it does not quit marimo or replace the app. If `install` fails, the script still replaces and restarts the app, then exits with a non-zero exit code. Check the reason shown, then run the script again.

### Updating from the settings window

**Updates** in the settings window can also update to the latest stable release, just like running the script without arguments (see [Checking for updates](#checking-for-updates)). Prereleases such as `0.1.2-beta.1` are never chosen as the update target, whatever the version of the installed app. To install a prerelease, run the script with `--pre`. The release is downloaded from the GitHub repository the app was built from.

When you click the button, marimo downloads the target release, verifies the checksum, extracts it, and then quits. Replacing the app, running `marimo-hook install`, and restarting are done by a helper script that marimo writes out, after waiting for marimo to quit. On macOS, the running `marimo.app` is replaced, wherever it is, not only `/Applications/marimo.app`. If downloading, verification, or extraction fails, marimo shows the reason without touching the app. The helper script's progress and failures after marimo quits are not shown on screen, so they are appended to `~/.marimo/update.log` (`update.log` under `MARIMO_HOME` if it is set). If replacing fails or the helper script stops partway, the original app is restarted. For a development executable started from `target` in the source tree, you can check for updates, but the update button does nothing and shows a failure.

### Building and updating locally

With `--build`, the script builds locally instead of downloading a build. Use it when you have no network connection, or to try a branch other than main. Without `--no-pull`, it checks the branch and local changes and pulls the latest main before building, as with `--main`. To try a branch other than main, add `--no-pull --build` to build what is currently checked out as is. Building requires the tools in [Prerequisites](development.md#prerequisites).

In this case, `marimo-hook` and the app are built in place of step 2. The user interface dependencies are reinstalled with `npm ci`, which does not modify `package-lock.json`. On macOS, only `marimo.app` is built; on Windows, only the NSIS installer. If the environment variable `CARGO_TARGET_DIR` is set, the build outputs are looked up under it. If the build fails, the script does not quit marimo or replace the app. If `install` fails, run `install` again with the `marimo-hook` you built.

On Windows, you can also update without the script by running the locally rebuilt NSIS installer again. In that case, also run `install` again with the `marimo-hook` you built, to replace the hook command.

### After updating

`install` does not change existing registrations, so you do not need to trust the hooks again with `/hooks` in Codex (see [Codex hooks](development.md#codex-hooks)). Hooks written in Hermes's `config.yaml` also need no changes unless the location of `marimo-hook` changes.

`install` rewrites the OpenCode plugin only when its content changes in a new version, and then shows a message asking you to restart OpenCode. The message appears in the terminal when you use the update script, and is left in `update.log` when you update from the settings window. If you see it, restart OpenCode, including its background service (see [The OpenCode plugin](development.md#the-opencode-plugin)).

## Troubleshooting

### Usage limits are missing or not updating

First, check in the table in [Usage limits](#usage-limits) when the tool's values are updated. When the **Refresh interval** is **Never**, neither the values nor the time change while you are not working locally, and after 30 minutes they are shown faintly.

If they do not update even though a **Refresh interval** is chosen, either the `claude` or `codex` command cannot be found, or refreshing fails. Where marimo looks for the commands is described in [What marimo runs](development.md#what-marimo-runs). The reason for a failure is printed to standard error when you start the app from a terminal.

If Claude Code's values do not change even though you work in the CLI, run `install` with `--dry-run` and check what it says about statusLine. If the existing statusLine could not be wrapped, or registration failed because of the path of marimo-hook, the reason is shown. Even then, choosing a **Refresh interval** makes the values appear (see [How statusLine is wrapped](development.md#how-statusline-is-wrapped)).

### A session's state does not appear

Idle sessions are not shown in the panel. If a session should be working, waiting, done, or errored but does not appear, check the following in order.

1. Check that the hooks and plugins are registered. Running `install` with `--dry-run` shows the list of registered hooks without changing any file. If Codex, OpenCode, or Antigravity settings folders exist, it also shows their registration status. `install` does not register Hermes Agent hooks, so check them with `hermes hooks list`.

   ```bash
   ~/.marimo/bin/marimo-hook install --dry-run
   ```

2. For Claude Code sessions, check the version with `claude --version`. marimo registers its hooks in the exec form, so older Claude Code versions that do not understand `args` do not call them correctly (see [Requirements](../README.md#requirements) for the required version).
3. If you use `CLAUDE_CONFIG_DIR` with Claude Code, check that `install` changed the settings.json under it. If you use a different settings file, register again by specifying it with `--settings` (see [Subcommands and the settings file location](development.md#subcommands-and-the-settings-file-location)).
4. Check that files are being created in `~/.marimo/sessions/`. If files exist but nothing appears, restart the app.
5. Tools that were already running before registration may not have loaded the hooks or plugins. Restart those tools. OpenCode also needs its background service restarted. In Codex, also check that you trusted the marimo hooks with `/hooks`.
6. Codex, OpenCode, and agy conversations that Claude Code delegated work to are not shown in the panel, so that the same work is not listed twice. How they are recognized is described in the "What ... records" section for each tool.
7. Check that the tool and interface you use are not listed as unsupported in [Testing status](development.md#testing-status).

### A finished session stays on the panel

Sessions whose tool did not report an end leave the panel, or return to idle, after the following times.

| Situation | Until it leaves the panel |
| --- | --- |
| The tool's process or the OpenCode service crashed or was killed, or the Hermes gateway was restarted | Working and waiting sessions return to idle after about 10 minutes. Done and error sessions are unread results and are kept (see [When a process exits abnormally](development.md#when-a-process-exits-abnormally)) |
| You stopped a response in Claude Code, or declined a tool permission or a question | In the CLI, it returns to idle with the idle notification that arrives after about 60 seconds; in the desktop app, after about 10 minutes (see [Responses stopped by the user](development.md#responses-stopped-by-the-user)) |
| You stopped Antigravity while it was waiting for a model response | It returns to idle after about 10 minutes (see [What the Antigravity hooks record](development.md#what-the-antigravity-hooks-record)) |
| You moved a Claude Code conversation to the background | The conversation continues in another session, and the original session disappears after about 10 minutes (see [Conversations moved to the background](development.md#conversations-moved-to-the-background)) |
| The SubagentStart and SubagentStop hooks are not registered in Claude Code | The session stays working for about 30 minutes after the subagent ends. Running `install` again adds only the missing hooks |
| Anything else, including Antigravity done and error sessions | Session files with no update for 24 hours are deleted, so the session disappears within about a day at most. Done and error sessions collapse once marked as read. Temporary files left behind by interrupted writes are also deleted after 24 hours |

A session shown as working because a subagent was running returns to done or error if the parent conversation is done or errored. Sessions that disappeared come back to the panel when work resumes, because the next hook creates the file again. To remove one right away, quit marimo and delete the corresponding file in `~/.marimo/sessions/`.

### Clicking a session on Windows does not switch to it

When the target app runs as administrator, marimo, which runs with normal privileges, may be unable to pass it what to open or to bring it to the front. In one case observed with Discord, opening a `discord://` URL started a new Discord that could not pass the URL to the main Discord running as administrator and exited; the channel did not change, and the taskbar button only flashed. The same can happen with apps such as VS Code that pass a folder to their running instance.

The setting that runs an app as administrator is "Run this program as an administrator" on the Compatibility tab of the executable's properties. You can find the executables with this setting in PowerShell as follows. Lines whose value includes `RUNASADMIN` start that program as administrator.

```powershell
Get-ItemProperty 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers' | Format-List
```

Discord starts through `%LOCALAPPDATA%\Discord\Update.exe`, so the setting may be on Update.exe rather than Discord.exe. After removing the setting, quit the app completely, for example from its notification area icon, and start it again. An app that shows a UAC prompt every time it starts very likely has this setting.

### The window is off screen

Quit marimo, delete `~/.marimo/window.json`, and start marimo again. The window appears in its default position (the lower right of the main monitor).

### Resetting all state

Quit marimo, then delete any of the following files as needed. All of them can be deleted safely and are recreated at the next start or by the next hook.

| What to delete | What is reset |
| --- | --- |
| The files in `~/.marimo/sessions/` | The session list. It is recreated by later hooks |
| `~/.marimo/acknowledged.json` | The read records |
| `~/.marimo/window.json` | The window position and the corner of the window where the character sits |
| `~/.marimo/display.json` | The display, speech bubble, usage limits, character, and language settings (see [Files and data](development.md#files-and-data) for the fields) |
| `~/.marimo/rate_limits.json` | Claude Code's usage limit values |
| `~/.marimo/codex_rate_limits.json` | Codex's usage limit values |
| `~/.marimo/antigravity_rate_limits.json` | Antigravity's usage limit values |
| `~/.marimo/update.log` | The log of updates run from the settings window |

Do not delete `~/.marimo/bin/`, because the hooks point to it (see [What install does](development.md#what-install-does)). If you delete it, running `install` again restores it.

## Uninstalling

1. If you turned on **Launch at login** in the settings window, uncheck it first. This removes the LaunchAgent plist on macOS and the registry entry on Windows.
2. Quit marimo with **Quit** in the right-click menu or the icon's menu.
3. Unregister the hooks and statusLine, and remove the OpenCode plugin and the Antigravity hooks.

   ```bash
   ~/.marimo/bin/marimo-hook uninstall
   ```

   What is removed is described in [What uninstall does](development.md#what-uninstall-does). Afterwards, restart any tools that are running. `uninstall` does not remove the Hermes Agent hooks; remove them from `config.yaml` by hand by following [Hermes hooks](development.md#hermes-hooks).

4. On macOS, delete `/Applications/marimo.app`. On Windows, uninstall marimo from Apps in Settings.
5. `uninstall` leaves the executable and data behind, so delete the `~/.marimo` folder by hand if you no longer need it. At the end, `uninstall` shows the paths that are safe to delete. Also delete the settings.json backups (`settings.json.marimo-backup-*`) by hand when you no longer need them.
