# git-foreach

Run any command across many local git repositories at once, from a mouse-driven TUI.

Status: early development.

## Features

- **Mouse-first TUI** (ratatui) — clickable checkboxes, output focus, scrollbar, and preset chips. Mouse, default keys, and the config keymap all funnel through a single action hub, so they share one code path.
- **Repository tree** — discovers repos under `~/src` (`{fqdn}/{user}/{repo}`), grouped fqdn → user → repo with tri-state checkboxes for picking targets.
- **Parallel execution** — runs your command in every checked repo via `$SHELL -c` (cwd = the repo), gated by a concurrency limit. `stdin` is `/dev/null` and pagers / credential prompts are disabled, so nothing hangs without a TTY.
- **Per-repo output boxes** — each repo's stdout/stderr lands in its own box (stderr colored); focus one repo or scroll the whole stack. Rendering stays O(screen height) even on huge output. A floating button at the pane's top-right copies the visible output to the clipboard.
- **Live status** — per-repo idle / running / done / exit code / cancelled / skipped, shown by color and glyph.
- **Cancel that actually stops** — children run in their own process group, so cancel reaches grandchildren. It distinguishes *cancelled* (was running, killed) from *skipped* (never started).
- **Presets** — frequently used commands appear as chips above the prompt and load into the input on click.
- **Nerd Font styling** with a plain **ASCII fallback** (`icons = "ascii"`).

## Install

```bash
brew install m5d215/tap/git-foreach                       # prebuilt binary (macOS / Linux)
cargo install --git https://github.com/m5d215/git-foreach  # build from source
```

## Usage

```bash
git-foreach        # launch the TUI (scans ~/src)
```

Pick target repos with the checkboxes, type a command in the prompt, and press `Enter`
to run it across all of them. Each repo's output appears in its own box; click a repo
name to focus its output, or scroll through all of them.

## Controls

The UI is mouse-first; default keys are intentionally sparse and can be extended via config.

**Mouse**

- Click a checkbox to toggle a repo (or a group node to toggle all under it)
- Click a repo name to focus its output (click again to clear)
- Click the expand marker to fold / unfold a group
- Click a preset chip to load it into the prompt
- Wheel to scroll the output / tree

**Keys (defaults)**

| Key | Action |
|---|---|
| `Enter` (in prompt) | Run on all checked repos |
| `i` / `/` | Focus the command input |
| `Tab` | Cycle panes |
| `Space` | Toggle the cursor repo |
| `a` / `A` | Check all / uncheck all |
| `c` | Cancel |
| `r` | Rescan `~/src` |
| `q` / `Ctrl-C` | Quit |
| `Esc` | Leave input / clear focus |

## Configuration

Optional, at `~/.config/git-foreach/config.toml` (or `$XDG_CONFIG_HOME/git-foreach/config.toml`).
See [`config.example.toml`](config.example.toml).

```toml
# Repos checked by default at startup (globs over "fqdn/user/repo")
default_checked = ["github.com/m5d215/*"]

# "nerd" (default) or "ascii"
icons = "nerd"

# key -> Action name; the UI is mouse-first, so add keys you want here
[keymap]
j = "cursor_down"
k = "cursor_up"
space = "toggle_check"

# frequently used commands, shown as chips above the prompt
[[preset]]
label = "pull"
command = "git pull --ff-only"
key = "p"
```

## License

[MIT](LICENSE-MIT).
