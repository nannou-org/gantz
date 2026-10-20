//! Palettes in the base16 layout. The built-in [`PRESETS`], and [`parse`]
//! for scheme files from the base16 ecosystem.

use super::luminance;
use super::palette::{ACCENTS, NEUTRALS, Palette, from_hex, to_hex};

/// The number of colours in a base16 scheme.
const BASES: usize = NEUTRALS + ACCENTS;

/// A palette as `0xrrggbb` colours in base16 order.
#[derive(Clone, Copy, Debug)]
pub struct Scheme {
    pub name: &'static str,
    pub colors: [u32; BASES],
}

/// A base16 scheme file that does not describe a palette.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ParseError {
    #[error("the scheme lacks {}", .0.join(", "))]
    Missing(Vec<String>),
    #[error("`{key}` has no `rrggbb` colour: {value:?}")]
    BadColor { key: String, value: String },
}

pub const EGUI_DARK: Scheme = Scheme {
    name: "egui Dark",
    colors: [
        0x0a0a0a, 0x1b1b1b, 0x3c3c3c, 0x4b4b4b, 0x8c8c8c, 0xb4b4b4, 0xe6e6e6, 0xffffff, 0xe05252,
        0xf0902a, 0xd4aa3a, 0x5cbf5c, 0x4cc2c2, 0x5aaaff, 0xb070e0, 0xe070b8,
    ],
};

pub const EGUI_LIGHT: Scheme = Scheme {
    name: "egui Light",
    colors: [
        0xffffff, 0xf8f8f8, 0xdadada, 0xc8c8c8, 0x505050, 0x3c3c3c, 0x141414, 0x000000, 0xc82828,
        0xb05400, 0x8c6400, 0x1e781e, 0x0a8a8a, 0x0a6ed1, 0x8040c0, 0xb03080,
    ],
};

/// The built-in palettes.
pub const PRESETS: &[Scheme] = &[
    EGUI_DARK,
    EGUI_LIGHT,
    Scheme {
        name: "Gruvbox Dark",
        colors: [
            0x282828, 0x3c3836, 0x504945, 0x665c54, 0xbdae93, 0xd5c4a1, 0xebdbb2, 0xfbf1c7,
            0xfb4934, 0xfe8019, 0xfabd2f, 0xb8bb26, 0x8ec07c, 0x83a598, 0xd3869b, 0xd65d0e,
        ],
    },
    Scheme {
        name: "Gruvbox Light",
        colors: [
            0xfbf1c7, 0xebdbb2, 0xd5c4a1, 0xbdae93, 0x665c54, 0x504945, 0x3c3836, 0x282828,
            0x9d0006, 0xaf3a03, 0xb57614, 0x79740e, 0x427b58, 0x076678, 0x8f3f71, 0xd65d0e,
        ],
    },
    Scheme {
        name: "Solarized Dark",
        colors: [
            0x002b36, 0x073642, 0x586e75, 0x657b83, 0x839496, 0x93a1a1, 0xeee8d5, 0xfdf6e3,
            0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682,
        ],
    },
    Scheme {
        name: "Solarized Light",
        colors: [
            0xfdf6e3, 0xeee8d5, 0x93a1a1, 0x839496, 0x657b83, 0x586e75, 0x073642, 0x002b36,
            0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682,
        ],
    },
];

impl From<&Scheme> for Palette {
    fn from(scheme: &Scheme) -> Self {
        let color = |i: usize| {
            let [_, r, g, b] = scheme.colors[i].to_be_bytes();
            egui::Color32::from_rgb(r, g, b)
        };
        Palette {
            name: scheme.name.to_string(),
            neutrals: std::array::from_fn(color),
            accents: std::array::from_fn(|i| color(NEUTRALS + i)),
        }
    }
}

/// The preset called `name`.
pub fn preset(name: &str) -> Option<&'static Scheme> {
    PRESETS.iter().find(|s| s.name == name)
}

/// Write `palette` as a base16 scheme file, in the `palette:` layout that
/// [`parse`] reads back.
///
/// The variant is dark when `n0` is darker than `n7`. Quotes are dropped
/// from the name, since scheme files do not escape them.
pub fn to_yaml(palette: &Palette) -> String {
    let [n0, n7] = [palette.neutrals[0], palette.neutrals[NEUTRALS - 1]];
    let variant = match luminance(n0) < luminance(n7) {
        true => "dark",
        false => "light",
    };
    let name: String = palette.name.chars().filter(|&c| c != '"').collect();
    let mut yaml =
        format!("system: \"base16\"\nname: \"{name}\"\nvariant: \"{variant}\"\npalette:\n");
    let colors = palette.neutrals.iter().chain(&palette.accents);
    for (i, &color) in colors.enumerate() {
        yaml.push_str(&format!("  base{i:02X}: \"{}\"\n", to_hex(color)));
    }
    yaml
}
/// Parse a base16 or base24 scheme file in its YAML form.
///
/// Both the classic layout, with `base00` to `base0F` at the top level, and
/// the newer layout, with the colours under `palette:`, parse. The name comes
/// from the `name` or `scheme` key. Other keys, including the extra base24
/// colours, are ignored. Only the line-based subset of YAML that scheme files
/// use is understood.
pub fn parse(text: &str) -> Result<Palette, ParseError> {
    let mut name = None;
    let mut colors: [Option<egui::Color32>; BASES] = [None; BASES];
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = unquote(key.trim());
        let value = unquote(value.trim());
        if key == "name" || (key == "scheme" && name.is_none()) {
            name = Some(value.to_string());
        } else if let Some(i) = base_index(key) {
            let color = from_hex(value).ok_or_else(|| ParseError::BadColor {
                key: key.to_string(),
                value: value.to_string(),
            })?;
            colors[i] = Some(color);
        }
    }
    let missing: Vec<String> = (0..BASES)
        .filter(|&i| colors[i].is_none())
        .map(|i| format!("base{i:02X}"))
        .collect();
    if !missing.is_empty() {
        return Err(ParseError::Missing(missing));
    }
    let color = |i: usize| colors[i].unwrap_or_default();
    Ok(Palette {
        name: name.unwrap_or_else(|| "Imported".to_string()),
        neutrals: std::array::from_fn(color),
        accents: std::array::from_fn(|i| color(NEUTRALS + i)),
    })
}

/// The index of a `base00` to `base0F` key.
fn base_index(key: &str) -> Option<usize> {
    let digits = key.strip_prefix("base")?;
    if digits.len() != 2 {
        return None;
    }
    let i = usize::from_str_radix(digits, 16).ok()?;
    (i < BASES).then_some(i)
}

/// A YAML scalar without its quotes or trailing comment.
fn unquote(s: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(rest) = s.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or(rest);
        }
    }
    s.split(" #").next().unwrap_or(s).trim()
}

#[cfg(test)]
mod tests {
    use super::{PRESETS, ParseError, parse, preset, to_yaml};
    use crate::style::Palette;

    /// An exported palette parses back to itself, in either variant.
    #[test]
    fn yaml_round_trips() {
        for scheme in PRESETS {
            let palette = Palette::from(scheme);
            let yaml = to_yaml(&palette);
            assert_eq!(parse(&yaml), Ok(palette), "{yaml}");
        }
        assert!(to_yaml(&Palette::dark()).contains("variant: \"dark\""));
        assert!(to_yaml(&Palette::light()).contains("variant: \"light\""));
    }

    #[test]
    fn presets_have_unique_names() {
        for scheme in PRESETS {
            assert_eq!(preset(scheme.name).map(|s| s.colors), Some(scheme.colors));
        }
    }

    #[test]
    fn parses_the_classic_layout() {
        let text = "scheme: \"Gruvbox Dark\" # medium\n\
                    author: \"Dawid Kurek\"\n\
                    base00: \"282828\"\nbase01: \"3c3836\"\nbase02: \"504945\"\n\
                    base03: \"665c54\"\nbase04: \"bdae93\"\nbase05: \"d5c4a1\"\n\
                    base06: \"ebdbb2\"\nbase07: \"fbf1c7\"\nbase08: \"fb4934\"\n\
                    base09: \"fe8019\"\nbase0A: \"fabd2f\"\nbase0B: \"b8bb26\"\n\
                    base0C: \"8ec07c\"\nbase0D: \"83a598\"\nbase0E: \"d3869b\"\n\
                    base0F: \"d65d0e\"\n";
        assert_eq!(parse(text), Ok(Palette::from(&PRESETS[2])));
    }

    #[test]
    fn parses_the_palette_layout() {
        let mut text = String::from(
            "system: \"base24\"\nname: \"Solarized Dark\"\nvariant: \"dark\"\npalette:\n",
        );
        for (i, color) in PRESETS[4].colors.iter().enumerate() {
            text.push_str(&format!("  base{i:02X}: \"#{color:06x}\" # comment\n"));
        }
        text.push_str("  base10: \"#000000\"\n");
        assert_eq!(parse(&text), Ok(Palette::from(&PRESETS[4])));
    }

    #[test]
    fn reports_missing_and_bad_colours() {
        let missing = parse("base00: \"282828\"").expect_err("most bases are missing");
        let ParseError::Missing(keys) = missing else {
            panic!("expected missing keys");
        };
        assert_eq!(keys.len(), 15);
        assert_eq!(keys[0], "base01");
        let bad = parse("base03: \"nope\"").expect_err("bad colour");
        assert!(matches!(bad, ParseError::BadColor { .. }));
    }
}
