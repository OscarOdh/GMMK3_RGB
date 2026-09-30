# OctoCopy

**Eight clipboards instead of one. Summon it from anywhere with `Ctrl + Alt + C`.**

A tiny Windows tray app that holds up to 8 pieces of text at once, so you can
paste any of them instantly without going back to hunt for the original.

<img src="docs/GMMK3RGB.png" width="420"
     alt="The GMMK3 RGB control window: live device status showing the detected
     keyboard, protocol version, packet size and frames sent; colour pickers for
     the four LED zones with their index ranges; a Caps Lock indicator toggle;
     the reactive press colour and fade-back slider; and keymap learning.">

### ⬇ [**Download OctoCopy.exe**](https://github.com/OscarOdh/OctoCopy/raw/main/OctoCopy.exe) · 95 KB

Double-click it and you're running. No installer, no dependencies, nothing to
configure. Works on any up-to-date Windows 10 or 11.

The exe isn't code-signed, so Windows SmartScreen will show a blue
"Windows protected your PC" box the first time. Click **More info**, then
**Run anyway**. Or build it yourself from source below.

---

## What problem does this solve?

Windows gives you exactly one clipboard. Copy something new and the old thing is
gone. That's fine until you're doing the kind of work where you need four or five
pieces of text over and over: filling in a form with the same address, ticket
number and account ID, writing a report that keeps citing the same three figures,
testing a login with the same credentials fifty times.

The usual workaround is a scratch Notepad window you keep alt-tabbing to, select
from, copy from, and alt-tab back. That's four actions per paste.

OctoCopy makes it one. Put your snippets in once, then press `Ctrl + Z` (or `X`,
`C`, `V`, `A`, `S`, `D`, `F`) to load any of them into your clipboard. The app
doesn't need to be focused for the summon hotkey, doesn't need a save button, and
lives in the system tray where it costs you no screen space.

It is not a clipboard *history* tool, on purpose. Nothing is captured
automatically, nothing is logged in the background. You decide what goes in the
eight slots, and that's all it ever holds.

---

## Features

**Summon from anywhere.** `Ctrl + Alt + C` pulls OctoCopy to the front from any
application, whether it's minimized, hidden in the tray, or buried behind twelve
windows. `Escape` sends it straight back.

**Copy without clicking.** `Ctrl + Z / X / C / V / A / S / D / F` copies slot 1
through 8. The letters are the left-hand keyboard cluster, so your hand never
moves. And because "which letter was slot 5 again?" is a real problem, **holding
`Ctrl` turns every Copy button into its own shortcut letter.** Let go and they
turn back. The app teaches you its shortcuts while you use it.

**Never lose your text.** There is no Save button because there's nothing to
save. Stop typing for 800ms and your snippets are written to disk. Toggle a
setting, close the window, reboot, and it's all still there.

**Copy straight from the tray.** Click the tray icon and the menu lists your
snippets by their first 40 characters. Click one to copy it. You never have to
open the main window at all.

**Visible confirmation.** The button you copied from flashes green with a `✓` for
half a second, including when you triggered it with a keyboard shortcut and
weren't looking at the button. You always know the copy landed.

**Get out of the way automatically.** Turn on *Minimize to tray after copy* and
the window hides itself one second after a shortcut copy, so you can paste into
whatever you were doing without a window in the way.

**A real dark mode.** Not just repainted controls. The actual Windows title bar
goes dark too, and on Windows 11 it's color-matched to the app's own background
so there's no visible seam between the frame and the window.

**See-through mode.** An opacity slider down to 20%, for reading a reference
document through the app while you work.

**Remembers where you put it.** Window position survives reboots, with a
multi-monitor safety check: if you had it on a second screen that's now
unplugged, it won't open off-screen where you can't reach it.

**Sizes itself to its contents.** Two to eight rows, and the window grows and
shrinks to match. No wasted empty space.

**One instance, always.** Launching it twice just tells you it's already running
and points at the tray, instead of starting a second copy that fights the first
over the same save file and the same global hotkey.

---

## Requirements

- **Windows** (Windows 10 or 11. The dark title bar needs Windows 10 1809 or
  later, and degrades gracefully on older builds.)
- **.NET Framework 4.8**, already present on any up-to-date Windows 10/11
  install. Nothing to download for end users.

To build it yourself: **Visual Studio 2019+** with the .NET desktop development
workload, or MSBuild from the .NET Framework Developer Pack.

---

## Getting it running

### Just use it

[Download `OctoCopy.exe`](https://github.com/OscarOdh/OctoCopy/raw/main/OctoCopy.exe)
and double-click it. That's the entire install: no installer, no registry keys,
no dependencies. Settings go in `%APPDATA%\OctoCopy\`, which is the only thing it
writes outside itself.

To uninstall, delete the exe, and delete `%APPDATA%\OctoCopy\` if you want your
snippets gone too.

### Build from source

```bash
git clone https://github.com/OscarOdh/OctoCopy.git
```

Open `App.sln` in Visual Studio and press F5. Or from a Developer Command Prompt:

```bash
msbuild App.csproj /p:Configuration=Release
```

The output is `bin\Release\App.exe`. Note the assembly is named `App`, not
`OctoCopy`. The shipped `OctoCopy.exe` is that file renamed. If you want the
build to produce the right name directly, change `<AssemblyName>` in
`App.csproj`.

### Start it with Windows

There's no built-in setting for this. Press `Win + R`, type `shell:startup`, and
drop a shortcut to the exe in the folder that opens.

---

## How it works, file by file

### `Program.cs`, the entry point and single-instance guard

Fourteen lines that do one important thing. Before the window is created, it
tries to acquire a named `Mutex`, a system-wide lock identified by a GUID.

If it gets the lock, it's the first instance and starts normally. If it doesn't,
another OctoCopy is already running, so it shows a message pointing at the tray
and exits immediately.

This matters more than it looks. Two instances would both try to write the same
JSON settings file, corrupting it, and both try to register the same global
hotkey. Windows gives `Ctrl + Alt + C` to exactly one process, so the second one
silently wouldn't work, which is a maddening bug to diagnose.

### `Form1.cs`, essentially the whole application

One file, about 800 lines. Here's what's in it, by concern.

#### Building the UI at runtime

The snippet rows aren't in the designer. They're created in code by
`AddSnippetRow()`, which builds a `TextBox` and a `Button`, positions the row at
`index * 29` pixels down, and drops both into a scrolling `Panel`.

Each button stores its own row number in its `.Tag` property. That's the trick
that lets all eight buttons share one `DynamicCopy_Click` handler: the handler
reads `.Tag` off whichever button fired, and uses it to index into the textbox
list. Eight controls, one handler, no duplicated code.

`AdjustFormHeight()` recalculates the window height every time a row is added or
removed: `rows × 29 + 10`, plus 64 pixels of fixed overhead for the menu bar and
buttons.

There's a subtle layout bug this code fixes. When the panel gets a vertical
scrollbar, its usable width shrinks, and every textbox suddenly runs under the
scrollbar. So the panel's `Resize` event recalculates all textbox widths from
`snippetPanel.ClientSize.Width`, which already excludes the scrollbar. Edges stay
clean whether the bar is there or not.

`AddSnippetRow()` also saves and restores the panel's scroll position around the
insert, because adding a control to a scrolled panel makes WinForms compute the
new control's position against the scrolled origin and place it in the wrong
spot.

#### Saving, without a save button

`SaveData()` collects everything (the eight snippet strings, every menu toggle,
the opacity level, and the window's X/Y coordinates) into an `AppSettings`
object, serializes it with `JavaScriptSerializer`, and writes it to
`%APPDATA%\OctoCopy\OctoCopy_Data.json`.

What makes it feel invisible is **debouncing**. Every keystroke in any textbox
stops an 800ms timer and immediately restarts it. So while you're typing, the
save keeps getting pushed back. Stop for 800ms and it fires once. You get
save-on-every-change reliability with one disk write per edit instead of one per
keystroke.

`Form1_Load()` has migration logic for an older save format. Early versions
stored a bare JSON array of strings; the current one stores an object. The loader
peeks at the first character (a `[` means the old format) and reads it the old
way, so upgrading never loses anyone's snippets.

Window position restore has a guard worth calling out. Before using a saved
position, it loops over `Screen.AllScreens` and checks whether the saved
rectangle actually intersects a working area. If you saved the position on a
second monitor that's now unplugged, the app would otherwise open at coordinates
that don't exist on any display: invisible, unreachable, and apparently broken.
No intersection means it falls back to the default position.

#### The clipboard and the green flash

`Clipboard.SetText()` does the copy. An empty slot calls `Clipboard.Clear()`
instead, so "copying" an empty box gives you empty rather than silently leaving
the previous contents in place.

`TriggerVisualFeedback()` is declared `async void`. It sets the button green with
a `✓`, then `await Task.Delay(500)`, then restores it. The `await` is what makes
this work: it yields control back to the UI thread instead of blocking it. A
`Thread.Sleep(500)` here would freeze the entire window for half a second every
copy. It also checks `IsDisposed` before restoring, in case you removed that row
during the delay.

#### Keyboard handling, three different mechanisms

Three distinct problems, three different Windows mechanisms.

**The global hotkey (`Ctrl + Alt + C`)** has to work when OctoCopy isn't focused,
which normal .NET events can't do. So it calls into Windows directly with
P/Invoke. `RegisterHotKey` from `user32.dll` claims the combination system-wide,
and the form overrides `WndProc` (the raw Windows message handler) to watch for
`WM_HOTKEY`, message `0x0312`. When it arrives, the window shows, activates, and
comes to the front. `UnregisterHotKey` releases the claim on close, so the
combination isn't held hostage after the app exits.

**The local shortcuts (`Ctrl + Z/X/C/V/A/S/D/F`)** are handled by overriding
`ProcessCmdKey`, which sees keystrokes before any control does. That's necessary
because `Ctrl + C` and `Ctrl + V` already mean something inside a textbox.
`ProcessCmdKey` intercepts them first and returns `true` to stop them going
further. `Escape` is handled here too, hiding the window to the tray.

**The Ctrl-held letter hints** use `OnKeyDown` / `OnKeyUp` with the form's
`KeyPreview` set to `true`, so the form sees keys even while a textbox has focus.
A `ctrlHeld` flag gates `UpdateCopyButtonTexts()`, which swaps every button's
caption between `Copy` and its shortcut letter. `OnDeactivate` resets the flag as
well, because otherwise alt-tabbing away while holding Ctrl would leave the
buttons stuck showing letters forever, since the key-up event goes to whatever
window you switched to.

#### The system tray

A `NotifyIcon` with a `ContextMenuStrip`. The menu is rebuilt from scratch every
time it opens, in the `Opening` event, so it always reflects the current
snippets. Text is stripped of line breaks and truncated to 40 characters, and
each item gets a closure capturing its own text to copy.

Left-clicking a tray icon doesn't normally open its context menu; only
right-click does. There's no public API for it, so the code uses **reflection**
to find and call `NotifyIcon`'s private `ShowContextMenu` method. It's a hack,
and it's guarded with `?.Invoke` so a future .NET change that renames the method
degrades to "left-click does nothing" rather than crashing.

`Form1_FormClosing` intercepts the X button. If *Minimize to tray on close* is
on, it sets `e.Cancel = true`, vetoing the close, then saves, hides the window,
and shows the tray icon. The app keeps running with its hotkey registered. It
checks `CloseReason.UserClosing` first, so a Windows shutdown or a tray-menu Exit
still closes properly instead of blocking the shutdown.

#### Dark mode, including the title bar

Repainting controls is easy. The title bar isn't. It's drawn by Windows, not by
your app, and WinForms has no property for it.

`SetTitleBarTheme()` calls `DwmSetWindowAttribute` from `dwmapi.dll` (the Desktop
Window Manager, the part of Windows that composites window frames) and sets:

- `DWMWA_USE_IMMERSIVE_DARK_MODE` (attribute 20), plus attribute 19 as a
  fallback. The constant changed between Windows 10 builds, so it sets both and
  lets the one that isn't recognised fail harmlessly.
- `DWMWA_CAPTION_COLOR` (35) to `0x00121212` and `DWMWA_TEXT_COLOR` (36) to
  white. These are Windows 11 only, and this is what produces the seamless look:
  the title bar is set to the *exact* same hex as the app background, so there's
  no visible boundary at all.

Then `SetWindowPos` with `SWP_FRAMECHANGED` forces the frame to redraw so the
change appears immediately rather than on the next resize. The whole thing is
wrapped in a `try/catch` that swallows failures, so older Windows versions just
get a normal light title bar instead of an exception.

`ApplyTheme()` handles everything inside the window, walking every dynamic
control. It has one carve-out: it never recolors a button that's currently
`LightGreen`, so toggling dark mode mid-flash doesn't leave a button stuck green.

#### The help window

Built in memory rather than shown as a `MessageBox`, because a `MessageBox` can't
be styled, can't be dark, and can't have bold headings. So `btnHelp_Click`
constructs a `Form` containing a `FlowLayoutPanel`, and a local `AddSection`
function adds a bold `Label` and a regular one per topic.

Two details that were clearly bugs once. The help form inherits the main window's
`TopMost` value, so an always-on-top OctoCopy can't hide its own help window
behind itself. And it forces native handle creation (`IntPtr h = helpForm.Handle`)
before applying the dark title bar, because `DwmSetWindowAttribute` needs a real
window handle and WinForms creates one lazily.

### `Form1.Designer.cs`, the static layout

Designer-generated. Holds only the parts that never change: the menu strip
(Options → Always On Top / Minimize to tray on close / Minimize to tray after
copy / Opacity / Dark Mode, plus Help), the `+ Add` / `- Remove` / `Clear All`
buttons, and the scrolling panel the snippet rows get injected into.

### `AppSettings` (at the bottom of `Form1.cs`), the save format

A plain data class: the snippet array, four boolean toggles, the opacity double,
and nullable window coordinates. `WindowLocationX/Y` are nullable so that "never
saved a position" is distinguishable from "saved position 0,0".

### Supporting files

| File | What it is |
|---|---|
| `App.sln` / `App.csproj` | Visual Studio solution and project. Targets .NET Framework 4.8, `WinExe` output. |
| `Properties\AssemblyInfo.cs` | Version and metadata baked into the exe. |
| `Properties\Resources.resx` | Embedded resources. |
| `Form1.resx` | The window icon, embedded as base64, which is why `Form1.resx` is 54 KB. |
| `c.ico` | The source icon file. |
| `OctoCopy.exe` | Prebuilt release binary. What the download link at the top points at. |

---

## Known limits

- **Windows only.** WinForms, P/Invoke into `user32.dll` and `dwmapi.dll`, and
  the tray API. There is no cross-platform path from here.
- **Eight slots, hardcoded.** `MaxSnippets = 8` because there are eight
  comfortable letter keys. Raising it means finding more keys.
- **Text only.** No images, no files, no rich text.
- **Snippets are stored in plaintext.** `%APPDATA%\OctoCopy\OctoCopy_Data.json`
  is an unencrypted file readable by anything running as your user. Don't keep
  passwords in it.
- **The hotkey isn't configurable.** `Ctrl + Alt + C` and the eight letters are
  compiled in. If another app has already claimed `Ctrl + Alt + C`,
  `RegisterHotKey` fails silently and the global summon just won't work.
- **The tray left-click uses reflection.** It calls a private .NET method. A
  future framework change could break it. It would break safely, but it would
  break.
