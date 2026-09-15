# rsclip

rsclip is a small Rust clipboard manager for Wayland desktops. It uses a low-memory daemon to
capture clipboard content and a separate resident GTK4 UI that starts on demand, stays warm,
and is activated by later `rsclip` invocations.

![rsclip resident UI](assets/rsclip-ui.png)

## Current scope

- SQLite-backed text, image, and file-reference history.
- Built-in secrets vault with masked values, search, copying, and renaming.
- `rsclipd store --mime ...` for manual or watcher-driven ingestion.
- `rsclipd watch` to spawn `wl-paste --watch` text, PNG, and URI-list watchers.
- `rsclipd pin`, `delete`, `paste`, `ocr`, and `favicons` subcommands.
- Text, link, and color classification.
- Image payload storage under XDG data directories.
- Resident GTK4 history overlay with search, filters, preview, copy, auto-paste, and secrets.
- OCR command plumbing through `rsclipd ocr` and in-app OCR button.
- Performance profiling support via `RSCLIP_PROFILE`.

## Install

On Arch based installs:

```bash
paru -S rsclip-bin
```

## Build

```bash
cargo build
```

On Arch/CachyOS, the GTK4 layer-shell system dependency is required for the overlay UI:

```bash
sudo pacman -S gtk4-layer-shell
```

## Nix

The flake supports `x86_64-linux` and `aarch64-linux` and provides a packaged build,
development shell, checks, runnable apps, and a NixOS module:

```bash
nix develop
nix build
nix run .
nix flake check
```

On NixOS, import the flake’s `nixosModules.default` and enable `programs.rsclip` to
install the package and start the clipboard daemon with the graphical user session.

Runtime tools expected by the full flow:

```bash
wl-copy wl-paste wtype tesseract
```

## Try it

### Daemon commands (`rsclipd`)

Manual storage:

```bash
printf 'hello from rsclip' | rsclipd store --mime text/plain
printf 'file:///tmp/a.txt\r\n' | rsclipd store --mime text/uri-list
```

File entries store URI references, not file contents. The original files or directories must still
exist when the entry is restored and pasted.

Inspect and filter history:

```bash
rsclipd list
rsclipd list --filter images
rsclipd list --sort most-used
rsclipd list --query search-term --limit 20 --json
```

Pin, delete, and restore:

```bash
rsclipd pin 1               # pin entry 1 (pinned entries resist cleanup and sort first)
rsclipd pin 1 --off         # unpin entry 1
rsclipd delete 1            # soft-delete entry 1
rsclipd paste 1             # copy entry 1 to clipboard and trigger paste via wtype
rsclipd paste 1 --copy-only # copy entry 1 to clipboard without triggering paste
rsclipd paste 1 --delay-ms 200 # customize keypress delay before paste
```

OCR on image entries:

```bash
rsclipd ocr 2               # run Tesseract OCR on image entry 2
rsclipd ocr 2 --lang eng    # specify language for OCR
```

Favicon cache:

```bash
rsclipd favicons clear      # clear cached icons and failed-domain records
rsclipd favicons refresh    # clear and re-queue favicons for all stored link domains
```

Run the background clipboard watcher:

```bash
rsclipd watch
```

### Resident UI commands (`rsclip`)

Start or control the resident overlay UI:

```bash
rsclip              # show the resident UI
rsclip show         # show the resident UI
rsclip toggle       # hide if visible, show if hidden
rsclip preload      # start and warm the resident UI without showing it
rsclip quit-ui      # stop the resident UI process
rsclip list         # print history from SQLite without starting GTK
```

Options for `rsclip list` and `rsclipd list`:
- `--query <q>`: Filter by search query
- `--filter <filter>`: `all`, `text`, `images`, `files`, `links`, `colors`, `pinned`
- `--sort <sort>`: `default`, `recent` (or `newest`), `oldest`, `type`, `most-used`
- `--limit <n>`: Maximum entries to print (default 100)
- `--json`: Output full JSON array of entries

On boot, run the headless daemon and optionally preload the resident GTK UI:

```bash
systemctl --user enable --now rsclipd.service
systemctl --user enable --now rsclip-ui.service
```

`rsclipd.service` keeps clipboard capture running. `rsclip-ui.service` runs
`rsclip preload`, which creates the hidden resident UI and warms the initial list so the first
hotkey or `rsclip show` after login activates an existing process instead of cold-starting GTK.
The UI and daemon are separate processes; the daemon stores history in SQLite and notifies the
UI over the existing Unix datagram socket.

Install the service and desktop file by adapting the files under `packaging/`.

## Keyboard shortcuts

The resident GTK overlay supports keyboard navigation and shortcuts:

| Key | Action |
| --- | --- |
| `Tab` | Switch between Clipboard and Secrets view |
| `Enter` | Paste entry (Clipboard) / Copy secret (Secrets) and close overlay |
| `Ctrl+Enter` | Copy entry to clipboard without auto-pasting and close overlay |
| `Ctrl+C` | Copy selected entry or secret to clipboard |
| `Ctrl+P` | Toggle pin on selected clipboard entry |
| `Ctrl+D` | Delete selected entry or secret (preserves current list position) |
| `Ctrl+S` | Save entry as secret (Clipboard) / Copy secret (Secrets) |
| `Ctrl+E` | Rename selected secret (Secrets view) |
| `Ctrl+R` | Refresh entries list |
| `Ctrl+I` | Filter to images |
| `Ctrl+L` | Filter to links |
| `Up` / `Down` | Navigate entries |
| `Esc` | Close overlay |

## Secrets vault

rsclip includes a built-in secrets vault to safeguard credentials and tokens:
- Press `Ctrl+S` on any text, image OCR, or link entry to move it into the Secrets vault under a custom alias. Moving an entry into secrets removes its raw content from clipboard history.
- Switch to the Secrets tab using `Tab` or by clicking the **Secrets** mode button in the top bar.
- Secret values are masked in the preview panel (`********tail`).
- Press `Enter` or `Ctrl+C` on a secret to copy its raw value to the clipboard.
- Press `Ctrl+E` to rename a secret's alias.
- Press `Ctrl+D` to delete a secret. Deleting a secret that was saved from clipboard automatically restores the original clipboard entry.

## Configuration

rsclip reads `~/.config/rsclip/config.toml`. Start from `config.example.toml`
for the full set of options.

History settings control UI list size, payload caps, dedupe behavior, and optional
soft cleanup for old unpinned entries. Byte caps and cleanup use `0` as disabled.

```toml
[history]
max_entries = 5000
max_text_bytes = 1048576
max_image_bytes = 10485760
dedupe = true
cleanup_unpinned_after_days = 0
```

Set either byte limit to `0` to explicitly allow unlimited payloads.

Paste behavior is configurable for the resident UI:

```toml
[paste]
auto_paste = true
paste_delay_ms = 140
method = "wtype"
```

OCR defaults are shared by the UI button and `rsclipd ocr`:

```toml
[ocr]
enabled = true
command = "tesseract"
default_language = "eng"
timeout_seconds = 20
auto_index = false
```

The resident UI also supports geometry and behavior settings:

```toml
[ui]
theme = "nonchalant-dark"
window_width = 920
window_height = 620
background_opacity = 0.70
resizable = false
preview_default = true
sidebar_width = 320
show_footer_hints = true
reset_on_show = true
auto_focus_search = true
start_view = "clipboard"
default_filter = "all"
default_sort = "default"
search_placeholder = "Search clipboard..."
secrets_search_placeholder = "Search secrets by name..."
```

UI configuration details:
- `start_view`: Initial tab when opening the overlay (`"clipboard"` or `"secrets"`).
- `default_filter`: Filter applied on launch/reset (`all`, `text`, `images`, `files`, `links`, `colors`, `pinned`).
- `default_sort`: History ordering (`default` [pinned first, then newest], `recent`/`newest`, `oldest`, `type`, `most-used`).
- `reset_on_show`: Reset search query, view, and filters whenever the overlay is shown.
- `auto_focus_search`: Automatically focus the search bar upon opening.
- `show_footer_hints`: Display keyboard shortcut hints in the bottom bar.
- `background_opacity`: Backdrop opacity (0.0 to 1.0) applied over `shell_bg`.

The resident UI watches `config.toml` and reloads UI settings automatically.

## Theme colors

The resident UI reads optional theme colors from `~/.config/rsclip/config.toml`.
All keys under `[ui.colors]` are optional; missing keys keep the built-in
`nonchalant-dark` defaults. Supported color formats are `#rgb`, `#rrggbb`,
`#rrggbbaa`, `rgb(r, g, b)`, and `rgba(r, g, b, a)`.
Use `ui.background_opacity` for the shell backdrop transparency, or set
`ui.colors.shell_bg` directly for full RGBA control.

```toml
[ui.colors]
accent = "#ff00aa"
accent_text = "#000000"
```

Color changes are hot-reloaded by the resident UI.

## Performance profiling

rsclip includes built-in hierarchical phase and memory profiling. Set `RSCLIP_PROFILE=1`
(or `RSCLIP_PROFILE=verbose`) to view phase elapsed times and memory deltas in stderr:

```bash
RSCLIP_PROFILE=1 rsclip
```

## Link favicons

rsclip can optionally fetch real favicons for copied links. Network activity is disabled
by default.

```toml
[links]
favicon_cache = true
```

Favicon fetching is handled by the resident `rsclipd watch` daemon in the background.
The UI never performs network requests. Icons are cached by domain, not by full URL,
and are fetched once with no automatic refresh. Failed domains are not retried
automatically. Missing icons use generated domain initials.

Manage cached icons:

```bash
rsclipd favicons clear      # clear cached icons and failed-domain records
rsclipd favicons refresh    # re-queue favicon fetches for all link domains in history
```

## Release notes

### v0.1.14

- Kept startup, search, filters, notifications, and virtual-list paging off the GTK thread.
- Rendered full entry text in a scrollable preview pane while keeping list paging light.
- Coalesced queued list requests and rejected stale results while the user keeps typing.

## Release and AUR

Build the release archive locally with `./scripts/build-release-archive.sh 0.1.17`.
Pushing a matching `v0.1.17` tag runs the release workflow, publishes the archive,
and updates the `rsclip-bin` AUR package.
