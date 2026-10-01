# Installing wezterminator

WezTerm **20240203** or newer. The Lua engine is a WezTerm plugin; the optional
`wezterminator` binary drives the TUI, art generation, stats cache, install /
undo, doctor, fleet and push.

## Plugin one-liner (add-on)

Add-on mode keeps your existing config. Put this near the end of `wezterm.lua`
(or let `wezterminator install --mode add-on` append a marked block):

```lua
local wzt = wezterm.plugin.require 'https://github.com/<owner>/wezterminator'
wzt.apply_to_config(config)
```

With a local checkout instead of a git URL:

```lua
local checkout = '/path/to/wezterminator'
local wzt = dofile(checkout .. '/plugin/init.lua')
wzt.apply_to_config(config, { dir = checkout })
```

Keys already set on `config` before `apply_to_config` stay yours.

## Binary

### From a release

Shell installer (macOS / Linux musl builds via cargo-dist):

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/<owner>/wezterminator/releases/latest/download/wezterminator-installer.sh \
  | sh
```

Homebrew (after the tap publishes):

```bash
brew install ikari/tap/wezterminator
```

AUR and Nix packages are **not** shipped yet.

### From source

```bash
git clone https://github.com/<owner>/wezterminator.git
cd wezterminator
cargo install --path crates/wezterminator
```

Requires Rust **1.93+** (edition 2024 workspace).

## Install modes (`wezterminator install`)

```text
wezterminator install --mode <MODE> [OPTIONS]
```

| Mode | Effect |
|---|---|
| `add-on` | Append a marked `apply_to_config` block; keep the rest of the file |
| `replace` | Back up touched files, write a checkout shim at the resolved config path |
| `replace-import` | Same as replace, plus import literal font / colours / keys into a local preset |

Options (see `wezterminator install --help`):

| Flag | Meaning |
|---|---|
| `--mode <MODE>` | Required. One of `add-on`, `replace`, `replace-import` |
| `--checkout <DIR>` | Checkout with `plugin/init.lua` (defaults to `.` when it looks like one) |
| `--plugin-url <URL>` | Add-on only: `wezterm.plugin.require` URL; omit to `dofile` the checkout |
| `--skip-migrate` | Do not migrate `~/.wezterm-*` state into the local layer |

Examples:

```bash
# Add-on from this checkout (dofile)
wezterminator install --mode add-on --checkout .

# Add-on via plugin URL
wezterminator install --mode add-on --plugin-url 'https://github.com/<owner>/wezterminator'

# Full replace from a clone
wezterminator install --mode replace --checkout ~/src/wezterminator

# Replace and import what the literal parser can see
wezterminator install --mode replace-import --checkout .
```

Config path resolution follows WezTerm's search order. If
`WEZTERM_CONFIG_FILE` is set, install targets that file and warns.

Install writes a manifest under the engine state directory and Blake3 hashes
of backed-up files. Running the same mode twice is idempotent when nothing
changed.

### Undo

```bash
wezterminator uninstall
```

Restores files from the install manifest. If you edited a managed file after
install, uninstall keeps the edited copy beside the restored original and
warns.

## After install

```bash
wezterminator doctor          # fonts, art, screens, install currency
wezterminator tui             # browse presets (live preview inside WezTerm)
wezterminator stats           # one-shot status cache line (Lua launches this)
```

Art for your recorded screen resolution:

```bash
wezterminator art generate phosphor --size 3024x1964
wezterminator art check phosphor
```

## Fleet and push (high level)

Optional private **fleet** layer (git clone shared across your machines):
attach, pull (`--ff-only`), and promote local presets into the fleet clone.
`wezterminator push` syncs the fleet layer to another host over SSH/rsync;
local layers are never pushed. See `wezterminator fleet --help` and
`wezterminator push --help` once those subcommands are wired on your build.

## Platform notes

| Platform | Support |
|---|---|
| macOS | Full |
| Linux | Full; chrome blur needs WezTerm nightly (`wayland_window_background_blur`) |
| Windows | Best-effort binary build; status probes / chrome parity deferred |

## Deferred packaging

- AUR
- Nix
- Pre-generated art packs for common resolutions (storage choice TBD)
