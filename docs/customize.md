# Customizing marimo

English | [日本語](customize.ja.md)

This document describes how to change the speech bubble dialogue, the character, and the assets. For an overview and installation, see the [README](../README.md).

- [Changing the dialogue](#changing-the-dialogue)
- [Making your own character](#making-your-own-character)
- [Rebuilding the assets](#rebuilding-the-assets)

## Changing the dialogue

The speech bubbles use the default dialogue of the selected character. The default dialogue exists for each language, and the English interface uses the English lines (see [Language](usage.md#language)). To use your own lines, write **only the categories you want to change** in `~/.marimo/dialogue.json`. For each category, marimo uses the lines in this file if there are any, and the default lines otherwise.

For example, to change only the lines for when a long task finishes and for when a session is waiting for approval, write the following.

```json
{
  "done.long": [
    "{folder} took {duration}, but it's finished. Time for a cup of tea?"
  ],
  "waiting": [
    "{folder} could use your help.",
    "{folder} is waiting for your reply."
  ]
}
```

The value of each category is an array of one or more strings. If there are several candidates, one of them is chosen. The file is read again whenever a new speech bubble is shown, so changes take effect without restarting the app. If the file cannot be read, or a category has the wrong format, it is ignored and marimo keeps running with the default lines.

Categories are separated by dots, and if a more specific category has no lines, its parent category is used. For example, if there is no `waiting.permission`, `waiting` is used.

| Category | When it is used |
| --- | --- |
| `waiting.permission` | Waiting for permission to run a command or operate on a file |
| `waiting.question` | Claude has asked a question and is waiting for an answer |
| `waiting.plan` | Waiting for approval of a plan |
| `waiting` | Any other kind of waiting |
| `done.short` | Finished in under 2 minutes |
| `done` | Finished in 2 to 15 minutes, or the time taken is unknown |
| `done.long` | Finished after 15 minutes or more |
| `error.rate_limit` | Stopped because a usage limit was reached |
| `error` | Any other error |

The time to finish is measured from sending the prompt to the end of the response. The following two placeholders can be used in the lines.

| Placeholder | Replaced with |
| --- | --- |
| `{folder}` | The repository name, or the name of the working folder (the last part of its path) outside a repository |
| `{duration}` | The time taken to finish, such as "25 min" or "1 hr 10 min", or "under a minute" for less than a minute. In the Japanese interface it takes the Japanese form, such as "25 分". It is empty when the time is unknown |

Each character has its own default dialogue. Koharu's is in `assets/character/koharu/dialogue.json` and Clawd's in `assets/character/clawd/dialogue.json`, where you can see all the lines and use them as a model for your own file. The English lines are in `dialogue.en.json` in the same folder. `~/.marimo/dialogue.json` is not split by character or by language, so the same overrides apply whichever character and language you select.

At startup, if `~/.marimo/dialogue.json` is exactly the same as the default dialogue that an older version of marimo wrote out in full, marimo treats it as unedited and renames it to `dialogue.json.unused-default`. While such a file remains, fixes to the default dialogue would never reach you.

## Making your own character

The character assets live in `assets/character/`, one folder per character, and are built into the app. The built-in characters are Koharu (Akizuki Koharu), `koharu`, and Clawd, `clawd`. You can add a character without changing any code; after the following steps, rebuild the app.

1. Under `assets/character/`, create a folder named after the character (an ID written in lowercase letters), and put the images, `manifest.json`, and the default dialogue `dialogue.json` in it. You can also add English dialogue as `dialogue.en.json`. Without it, the English interface also uses `dialogue.json`.
2. Add the name to the array in `assets/character/index.json`. The order of the array is the order in the **Character** list of the settings window, and the first character is the default.

At startup, if the saved name is not in this list, or that character's `manifest.json` cannot be read, the first (default) character is loaded. If `index.json` itself cannot be read, the default `koharu` is loaded and the settings window lists only `koharu`. If switching characters from the settings window fails, the previous character stays. What happens when an expression image cannot be read is described at the end of this section.

All assets must be transparent PNGs (an image format that supports a transparent background) of the same canvas size. The character area is fixed at a width of 108 pixels at a scale of 1.0, and its height follows from the aspect ratio of the canvas. Koharu's canvas is 800×1200 pixels, so the area is 108×162; Clawd's is 216×180 pixels, so the area is 108×90. The window height also follows the area. The speech bubble appears just above the top of the area, so keep the margin above the head only as large as any decoration drawn there. The area that receives clicks is built automatically from the alpha (opacity) of each image, so you do not need to prepare it separately.

`manifest.json` says which image to use in which situation. All expression names are kept here, so that you can add expressions or change characters without changing code. The fields are explained below, using Koharu's `manifest.json` as the example.

| Field | Meaning |
| --- | --- |
| `name` | The character's name (ID). Make it the same as the folder name |
| `display_name` | The name shown in the **Character** list of the settings window. If written like Koharu's, `{"ja": "小春", "en": "Koharu"}`, the name for the interface language is used, falling back to the other one. If omitted, `name` is used |
| `pixelated` | If `true`, images are drawn without smoothing the pixels when scaled, and the breathing motion is turned off. Use it for pixel art |
| `mode` | How the character is drawn. Only `"standing"` (a standing picture) works. `"sprite"` exists only as a type definition and is not implemented, so specifying it hides the character |
| `canvas` | The `width` and `height` of the asset canvas, used for the aspect ratio of the character area |
| `expressions` | For each expression name, the image file `image` and the blink variant `blink`. Only expressions with a `blink` blink |
| `rules.status` | The base expression for each of the five states (`idle`, `working`, `waiting`, `done`, `error`) |
| `rules.variants` | For each dialogue category name (from the table in [Changing the dialogue](#changing-the-dialogue)), the expression to use in that category. For example, `"done.long": "idle_stretch"` shows `idle_stretch` when a task finishes after 15 minutes or more. This takes priority over the state's base expression; if there is no entry for the specific category, the base expression is used (unlike dialogue, parent categories are not used). An expression whose image cannot be read is also replaced by the base expression. By default, Koharu uses this for `done.long` and `error.rate_limit` |
| `rules.working_tools` | While working, switches to the expression `expression` when any tool in the list of tool names `tools` is in use. By default: `working_focus` for Bash and PowerShell; `working_write` for Edit, Write, NotebookEdit, and apply_patch (Codex's file edit); `working_read` for Read, Grep, and Glob; `working_curious` for WebSearch and WebFetch; `working_plan` for TodoWrite, Codex's update_plan, ToolSearch, and Skill; `working_listen` for MCP tools; and `working_delegate` for Agent, Task, and spawn_agent (Codex), which start subagents. A `*` at the end of a tool name matches every tool whose name starts with the part before it. MCP tools arrive with names of the form `mcp__<server>__<tool>`, so `mcp__*` covers them all. An exact match takes priority. `max_ms` sets the longest time (in milliseconds) the expression is shown continuously, after which the base working expression returns. While a subagent runs, the same tool stays in use for minutes and a long wink looks odd, so `working_delegate` is set to 30000 by default |
| `rules.working_no_tool` | The expression while working without a tool (thinking or writing a response). The default is `working_think`. If omitted, the working expression from `rules.status` stays |
| `rules.min_switch_ms` | The shortest interval (in milliseconds) between changes of the working expression. Tools change every few seconds, so this prevents flicker. The default is 4000 |
| `rules.idle_gestures` | Gestures while idle. `interval_ms` is the minimum and maximum interval between gestures, and `gestures` lists each gesture's expression and the minimum and maximum time `duration_ms` it is shown. The same gesture is never chosen twice in a row |
| `rules.hover` | The expression shown for each state when the mouse is over the character. By default, `idle` is used only while working |
| `rules.hover_release_ms` | How long after the mouse leaves before the original expression returns. The default is 600 |

If an expression image cannot be read or is missing from the manifest, the base expression of that state is used, and failing that, `idle`. At minimum, a single `idle` image is enough.

## Rebuilding the assets

Rebuilding the assets uses a Python virtual environment (an independent Python environment that installs dependencies just for the project). Create the virtual environment at the root of the repository and install the dependencies. The versions are pinned in `tools/character/requirements.txt`.

```bash
python3 -m venv .venv
.venv/bin/pip install -r tools/character/requirements.txt
```

This virtual environment is also used to redraw the icons (see [Redrawing the icons](development.md#redrawing-the-icons)).

### Koharu's assets

`tools/character/build.py` is a script that produces the set of transparent PNGs used by the app from a generated base image and expression variants. It cuts out only the face from each expression variant and composites it onto the base image, so for face-only variants everything below the neck is pixel-for-pixel identical to the base image, and the body does not flicker when the expression changes. Transparency is created with the isnet-anime model of rembg (a Python library that removes image backgrounds), and the hair edges are solved again with the closed-form matting of pymatting (a library that estimates transparency at the boundary between foreground and background), so that no background color is left on the edges.

Run it with a configuration file, and it writes the PNGs to the configuration's `out_dir` (`assets/character/koharu` for Koharu).

```bash
.venv/bin/python tools/character/build.py tools/character/koharu.json
```

Pass a destination to `--preview` to also write an overview image that places the results on both a dark and a light background. Use it to check by eye that no gray from the background remains on the hair edges.

```bash
.venv/bin/python tools/character/build.py tools/character/koharu.json --preview preview.png
```

The configuration file (`tools/character/koharu.json` for Koharu) has the following fields.

| Field | Meaning |
| --- | --- |
| `base` | The base standing image |
| `out_dir` | The output folder |
| `canvas` | The width and height of the output images |
| `rembg_model` | The rembg model name. If omitted, `isnet-anime` is used |
| `face_ellipse` | The ellipse that marks the face area to cut out of variants: center `cx`, `cy` and radii `rx`, `ry` |
| `states` | How to make each output image |

Each entry in `states` has exactly one of the following forms.

| Form | How the image is made |
| --- | --- |
| `{"source": "base"}` | Uses the base image as is |
| `{"face": "<image>"}` | Composites only the face area of the variant onto the base image |
| `{"face": "<image>", "onto": "<state>"}` | Composites the face area onto the result of another state instead of the base image. Used for things like the blink variants of working expressions |
| `{"whole": "<image>"}` | Uses the image as is without compositing. Used for the waiting pose with an arm drawn in, and for gesture variants that redraw the whole figure |
| `{"whole": "<image>", "keep": [[x0, y0, x1, y1]]}` | Like `whole`, but inside the rectangles given in the original image's coordinates, foreground and background are separated by the difference from the background color. rembg removes small symbols floating away from the figure as background, so use this to keep things like the musical notes of the humming gesture |

The source images that were used go in `art/<character name>/`. `build.py` writes only the PNGs and does not change `manifest.json`, so when you add expressions, edit `manifest.json` by hand as well.

### Clawd's assets

Clawd's assets are drawn as pixel art by `tools/character/clawd.py`. The shapes and expressions are kept in the script as strings with one character per pixel, and only Pillow is used. One pixel of the logo is a quarter of a terminal character cell and is taller than it is wide, so each logo pixel is drawn as 2 squares across and 4 down, keeping the same aspect ratio as in a terminal. Running it writes PNGs to `assets/character/clawd/`, and passing a destination to `--preview` also writes an overview image on dark and light backgrounds.

```bash
.venv/bin/python tools/character/clawd.py --preview preview.png
```

`clawd.py` also writes only the PNGs, so when you add expressions, edit `assets/character/clawd/manifest.json` by hand as well.
