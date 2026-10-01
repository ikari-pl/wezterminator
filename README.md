# wezterminator

Composable WezTerm presets: a Lua plugin engine, procedural art, a TUI with
live preview, and a Rust toolkit for install, doctor, fleet and push.

Minimum WezTerm: **20240203**. JSON is the shared data format for Lua and Rust.

## Quick start (add-on plugin)

Keep your existing `wezterm.lua` and load the plugin:

```lua
local wezterm = require 'wezterm'
local config = wezterm.config_builder()

-- Replace <owner> once the public remote exists.
local wzt = wezterm.plugin.require 'https://github.com/<owner>/wezterminator'
wzt.apply_to_config(config)

return config
```

Call `apply_to_config` last. In add-on mode the engine never overwrites keys
your config already set; the TUI and `doctor` list those as overruled.

Full install modes (add-on, replace, replace-and-import), binary installs, and
undo: **[docs/install.md](docs/install.md)**.

## Install the binary

From a release (after the first tagged cargo-dist build):

```bash
# Shell installer (macOS / Linux)
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/<owner>/wezterminator/releases/latest/download/wezterminator-installer.sh \
  | sh
```

Or Homebrew (once the tap is published):

```bash
brew install ikari/tap/wezterminator
```

From a checkout:

```bash
cargo install --path crates/wezterminator
wezterminator install --mode add-on --checkout .
wezterminator doctor
```

## What you get

| Piece | Role |
|---|---|
| Lua plugin (`plugin/`) | Resolves presets, owns window overrides, status, parallax |
| Built-in themes / presets | Origin looks plus the theme library (Phosphor, Abyssal, …) |
| `wezterminator` CLI | `tui`, `art`, `stats`, `doctor`, `install` / `uninstall`, `fleet`, `push` |

Layer precedence: **built-in → fleet → local** (later wins field by field).
Machine settings (project roots, VPN probes, push targets) stay out of
presets so public exports cannot leak hostnames.

Authoring themes: **[docs/authoring-themes.md](docs/authoring-themes.md)**.  
Data model contract: **[docs/data-model.md](docs/data-model.md)**.

## Development

```bash
# Rust (includes shared resolution fixtures + art goldens)
cargo test --workspace

# Lua harness under stock Lua (same fixtures)
lua tests/lua/run.lua
```

CI runs those on macOS and Linux; Windows is build-only. Releases use
[cargo-dist](https://opensource.axo.dev/cargo-dist/) (`dist-workspace.toml`)
for macOS and Linux musl binaries, a shell installer, and a Homebrew tap.
**AUR and Nix packaging are deferred.**

## License

MIT — see [LICENSE](LICENSE).
