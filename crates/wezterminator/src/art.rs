//! `wezterminator art`: generate theme art from its recipe.
//!
//! `art generate <theme>` renders every layer of the theme's recipe at one
//! device resolution, checks that the dim text stays legible over the result,
//! and writes the layer PNGs plus a manifest to
//! `<data>/wezterminator/art/<theme>/<W>x<H>/`.
//!
//! The resolution is `--size WxH`, or else the largest screen the Lua engine
//! recorded in `screens.json`. Asking for a resolution that was never
//! generated simply writes a new directory next to the old ones.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use wzt_art::{Device, GenerateReport, Options, generate_theme, render_theme};
use wzt_model::paths::{THEME_FILE, THEMES_DIR, builtin_dir};
use wzt_model::{Paths, Screens, Theme, read_document};

#[derive(Debug, Args)]
pub struct ArtArgs {
    #[command(subcommand)]
    pub command: ArtCommand,
}

/// Where to find the theme and what resolution to render.
#[derive(Debug, Args)]
pub struct Target {
    /// A theme slug (`cpc-cool`, or `builtin:cpc-cool` to pick the layer), or
    /// a path to a `theme.json` or the directory holding it.
    pub theme: String,

    /// Device resolution as WIDTHxHEIGHT. Defaults to the largest screen
    /// recorded in screens.json.
    #[arg(long, value_name = "WxH")]
    pub size: Option<Device>,

    /// Directory of a wezterminator checkout holding the built-in themes.
    /// Defaults to the current directory when it has a `themes/` folder, and
    /// otherwise to the plugin directory the engine recorded.
    #[arg(long, value_name = "DIR")]
    pub checkout: Option<PathBuf>,

    /// Render with this many threads. The pixels are identical for any value.
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,
}

#[derive(Debug, Subcommand)]
pub enum ArtCommand {
    /// Render a theme's layers to PNGs for one resolution.
    Generate {
        #[command(flatten)]
        target: Target,

        /// Write here instead of the per-resolution art directory.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,

        /// Write the art even when the legibility check fails.
        #[arg(long)]
        skip_legibility: bool,
    },
    /// Render a theme in memory and report legibility, writing nothing.
    Check {
        #[command(flatten)]
        target: Target,
    },
    /// Map an image onto a theme's palette (not implemented yet).
    Import(crate::Pending),
    /// Pack generated art for sharing (not implemented yet).
    Pack(crate::Pending),
}

impl ArtCommand {
    /// The unit that will implement a still-missing subcommand.
    pub fn stub(&self) -> Option<(&'static str, &'static str)> {
        match self {
            ArtCommand::Import(_) => Some(("art import", "U9")),
            ArtCommand::Pack(_) => Some(("art pack", "U9")),
            _ => None,
        }
    }
}

/// A theme found on disk.
#[derive(Debug)]
pub struct FoundTheme {
    pub theme: Theme,
    pub slug: String,
    /// Directory holding `theme.json`; relative image paths resolve here.
    pub dir: PathBuf,
}

/// Find `name` among explicit paths, or the local, fleet and built-in layers.
pub fn find_theme(name: &str, layers: &[(&str, Option<PathBuf>)]) -> Result<FoundTheme, String> {
    let as_path = Path::new(name);
    let load = |file: &Path, slug: &str| -> Result<FoundTheme, String> {
        let theme: Theme = read_document(file).map_err(|e| e.to_string())?;
        let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok(FoundTheme {
            theme,
            slug: slug.to_string(),
            dir,
        })
    };

    if as_path.is_file() || as_path.join(THEME_FILE).is_file() {
        let file = if as_path.is_file() {
            as_path.to_path_buf()
        } else {
            as_path.join(THEME_FILE)
        };
        let dir = file.parent().unwrap_or(Path::new("."));
        let slug = std::fs::canonicalize(dir)
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "theme".to_string());
        return load(&file, &slug);
    }

    let (namespace, slug) = match name.split_once(':') {
        Some((ns, slug)) => (Some(ns), slug),
        None => (None, name),
    };
    for (layer, dir) in layers {
        if namespace.is_some_and(|ns| ns != *layer) {
            continue;
        }
        let Some(dir) = dir else { continue };
        let file = dir.join(THEMES_DIR).join(slug).join(THEME_FILE);
        if file.is_file() {
            return load(&file, slug);
        }
    }
    let searched: Vec<String> = layers
        .iter()
        .filter(|(layer, dir)| dir.is_some() && namespace.is_none_or(|ns| ns == *layer))
        .map(|(layer, dir)| {
            format!(
                "{layer} ({})",
                dir.as_ref()
                    .map_or_else(String::new, |d| d.display().to_string())
            )
        })
        .collect();
    Err(format!(
        "no theme `{name}` found in {}",
        if searched.is_empty() {
            "any layer (is --checkout right?)".to_string()
        } else {
            searched.join(", ")
        }
    ))
}

/// The largest recorded screen by pixel area, ties to the first.
pub fn largest_screen(screens: &Screens) -> Option<Device> {
    let mut best: Option<(u64, Device)> = None;
    for s in &screens.screens {
        let Ok(device) = Device::new(s.width, s.height) else {
            continue;
        };
        let area = s.width.saturating_mul(s.height);
        if best.is_none_or(|(a, _)| area > a) {
            best = Some((area, device));
        }
    }
    best.map(|(_, d)| d)
}

fn resolve_device(paths: &Paths, size: Option<Device>) -> Result<Device, String> {
    if let Some(d) = size {
        return Ok(d);
    }
    let file = paths.screens_file();
    if !file.exists() {
        return Err(format!(
            "no screens recorded yet ({} does not exist). Open WezTerm once with the engine \
             loaded, or pass --size WIDTHxHEIGHT",
            file.display()
        ));
    }
    let screens: Screens = read_document(&file).map_err(|e| e.to_string())?;
    largest_screen(&screens).ok_or_else(|| {
        format!(
            "{} lists no usable screen. Pass --size WIDTHxHEIGHT",
            file.display()
        )
    })
}

fn find_for(paths: &Paths, target: &Target) -> Result<FoundTheme, String> {
    let checkout = target.checkout.clone().or_else(|| {
        std::env::current_dir()
            .ok()
            .filter(|cwd| cwd.join(THEMES_DIR).is_dir())
    });
    let recorded = wzt_model::io::read_value(&paths.state_file())
        .ok()
        .and_then(|s| {
            s.pointer("/engine/plugin_dir")
                .and_then(|v| v.as_str().map(str::to_string))
        });
    let layers = [
        ("local", Some(paths.local_layer_dir().to_path_buf())),
        ("fleet", Some(paths.fleet_layer_dir())),
        (
            "builtin",
            builtin_dir(checkout.as_deref(), recorded.as_deref()),
        ),
    ];
    find_theme(&target.theme, &layers)
}

fn print_report(found: &FoundTheme, device: Device, report: &GenerateReport) {
    println!("{} ({}) at {device}", found.theme.name, found.slug);
    for l in &report.manifest.layers {
        let (lw, lh) = (device.width / l.scale, device.height / l.scale);
        println!(
            "  {:<14} {lw}x{lh} x{} -> {device}  {}",
            l.id, l.scale, l.file
        );
    }
    if let Some(l) = &report.legibility {
        println!(
            "legibility: {} {:.2}:1 over the densest tile, needs {:.2}:1",
            l.limiting, l.contrast, l.required
        );
    }
    println!("recipe {}", &report.manifest.recipe_hash[..16]);
    println!("wrote {}", report.dir.display());
}

pub fn run(paths: &Paths, args: &ArtArgs) -> Result<(), String> {
    match &args.command {
        ArtCommand::Generate {
            target,
            out,
            skip_legibility,
        } => {
            let found = find_for(paths, target)?;
            let device = resolve_device(paths, target.size)?;
            let dir = out.clone().unwrap_or_else(|| {
                paths.art_dir(
                    &found.slug,
                    u64::from(device.width),
                    u64::from(device.height),
                )
            });
            let options = Options {
                threads: target.threads,
                theme_dir: Some(found.dir.clone()),
                skip_legibility: *skip_legibility,
            };
            let report = generate_theme(&found.theme, device, &dir, &options)
                .map_err(|e| format!("{}: {e}", found.slug))?;
            print_report(&found, device, &report);
            for w in &report.warnings {
                eprintln!("wezterminator art: {w}");
            }
            Ok(())
        }
        ArtCommand::Check { target } => {
            let found = find_for(paths, target)?;
            let device = resolve_device(paths, target.size)?;
            let options = Options {
                threads: target.threads,
                theme_dir: Some(found.dir.clone()),
                skip_legibility: false,
            };
            let layers = render_theme(&found.theme, device, &options)
                .map_err(|e| format!("{}: {e}", found.slug))?;
            let l = wzt_art::check_legibility(&found.theme, &layers, device)
                .map_err(|e| format!("{}: {e}", found.slug))?;
            println!(
                "{} ({}) at {device}: {} {:.2}:1 over the densest tile at device pixel {:?}, needs {:.2}:1 -> {}",
                found.theme.name,
                found.slug,
                l.limiting,
                l.contrast,
                l.densest,
                l.required,
                if l.passed { "ok" } else { "TOO DENSE" }
            );
            if l.passed {
                Ok(())
            } else {
                Err(format!("{}: legibility check failed", found.slug))
            }
        }
        ArtCommand::Import(_) | ArtCommand::Pack(_) => {
            unreachable!("stubs are handled before run")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use wzt_model::Screen;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn screen(w: u64, h: u64) -> Screen {
        Screen {
            name: None,
            width: w,
            height: h,
            comments: Default::default(),
        }
    }

    #[test]
    fn finds_builtin_themes_by_slug_and_by_namespaced_id() {
        let layers = [("local", None), ("builtin", Some(repo()))];
        for name in ["soft-nebula", "builtin:soft-nebula"] {
            let found = find_theme(name, &layers).unwrap();
            assert_eq!(found.slug, "soft-nebula");
            assert_eq!(found.theme.id, "builtin:soft-nebula");
        }
        // A namespace pins the layer.
        assert!(find_theme("local:soft-nebula", &layers).is_err());
        assert!(
            find_theme("nope", &layers)
                .unwrap_err()
                .contains("no theme `nope`")
        );
    }

    #[test]
    fn a_path_to_a_theme_file_or_directory_works() {
        let dir = repo().join("themes/ember");
        assert_eq!(
            find_theme(dir.to_str().unwrap(), &[]).unwrap().slug,
            "ember"
        );
        let file = dir.join("theme.json");
        assert_eq!(
            find_theme(file.to_str().unwrap(), &[]).unwrap().theme.id,
            "builtin:ember"
        );
    }

    #[test]
    fn higher_layers_shadow_lower_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("themes/soft-nebula");
        fs::create_dir_all(&local).unwrap();
        let mut theme: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo().join("themes/soft-nebula/theme.json")).unwrap(),
        )
        .unwrap();
        theme["id"] = "local:soft-nebula".into();
        theme["name"] = "Mine".into();
        fs::write(local.join("theme.json"), theme.to_string()).unwrap();
        let layers = [
            ("local", Some(tmp.path().to_path_buf())),
            ("builtin", Some(repo())),
        ];
        assert_eq!(
            find_theme("soft-nebula", &layers).unwrap().theme.name,
            "Mine"
        );
        assert_eq!(
            find_theme("builtin:soft-nebula", &layers)
                .unwrap()
                .theme
                .name,
            "Soft Nebula"
        );
    }

    #[test]
    fn picks_the_screen_with_the_most_pixels() {
        let screens = Screens {
            schema_version: 1,
            screens: vec![screen(2560, 1440), screen(6016, 3384), screen(3840, 2160)],
            comments: Default::default(),
        };
        assert_eq!(
            largest_screen(&screens),
            Some(Device {
                width: 6016,
                height: 3384
            })
        );
        let none = Screens {
            schema_version: 1,
            screens: vec![],
            comments: Default::default(),
        };
        assert_eq!(largest_screen(&none), None);
    }

    #[test]
    fn generate_writes_the_per_resolution_directory() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::from_roots(
            home.path().join("c"),
            home.path().join("d"),
            home.path().join("s"),
        );
        let args = ArtArgs {
            command: ArtCommand::Generate {
                target: Target {
                    theme: "soft-nebula".into(),
                    size: Some(Device {
                        width: 960,
                        height: 540,
                    }),
                    checkout: Some(repo()),
                    threads: Some(2),
                },
                out: None,
                skip_legibility: false,
            },
        };
        run(&paths, &args).unwrap();
        let dir = paths.art_dir("soft-nebula", 960, 540);
        for f in ["nebula.png", "stars.png", "haze.png", "manifest.json"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
    }

    #[test]
    fn no_size_and_no_screens_is_a_clear_error() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths::from_roots(
            home.path().join("c"),
            home.path().join("d"),
            home.path().join("s"),
        );
        let err = resolve_device(&paths, None).unwrap_err();
        assert!(err.contains("--size"), "{err}");
    }
}
