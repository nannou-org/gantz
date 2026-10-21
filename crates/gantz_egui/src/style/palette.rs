//! The colours a style picks from. See [`Palette`].

/// The number of neutral swatches in a palette.
pub const NEUTRALS: usize = 8;

/// The number of accent swatches in a palette.
pub const ACCENTS: usize = 8;

/// A named set of opaque colours. Every colour in the UI is one of these
/// swatches, optionally with reduced opacity. See [`ColorRef`].
///
/// The neutrals form a ramp. `n0` is the deepest background and `n7` is the
/// strongest foreground. A light palette runs the same way, so its `n0` is
/// near white. One role mapping can then serve dark and light palettes alike.
/// The accents are named by hue.
///
/// The layout matches base16. The neutrals are `base00` to `base07` and the
/// accents are `base08` to `base0F`, in order.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Palette {
    /// A display name, usually the name of the preset it started from.
    pub name: String,
    #[serde(with = "hex")]
    pub neutrals: [egui::Color32; NEUTRALS],
    #[serde(with = "hex")]
    pub accents: [egui::Color32; ACCENTS],
}

/// One swatch of a [`Palette`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub enum Swatch {
    N0,
    N1,
    N2,
    N3,
    N4,
    N5,
    N6,
    N7,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Magenta,
}

/// A reference to a palette swatch, with an opacity.
///
/// Styles store these rather than colours, so a change to a swatch reaches
/// everything that refers to it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ColorRef {
    pub swatch: Swatch,
    /// From `0.0`, fully transparent, to `1.0`, opaque.
    #[serde(default = "opaque", skip_serializing_if = "is_opaque")]
    pub opacity: f32,
}

impl Palette {
    /// The default dark palette, "gantz Dark".
    pub fn dark() -> Self {
        Self::from(&super::base16::GANTZ_DARK)
    }

    /// The default light palette, "gantz Light".
    pub fn light() -> Self {
        Self::from(&super::base16::GANTZ_LIGHT)
    }

    /// The colour of `swatch`.
    pub fn get(&self, swatch: Swatch) -> egui::Color32 {
        let i = swatch.index();
        match i.checked_sub(NEUTRALS) {
            None => self.neutrals[i],
            Some(i) => self.accents[i],
        }
    }

    /// The colour of `swatch`, for editing.
    pub fn get_mut(&mut self, swatch: Swatch) -> &mut egui::Color32 {
        let i = swatch.index();
        match i.checked_sub(NEUTRALS) {
            None => &mut self.neutrals[i],
            Some(i) => &mut self.accents[i],
        }
    }
}

impl Swatch {
    pub const NEUTRALS: [Self; NEUTRALS] = [
        Self::N0,
        Self::N1,
        Self::N2,
        Self::N3,
        Self::N4,
        Self::N5,
        Self::N6,
        Self::N7,
    ];

    pub const ACCENTS: [Self; ACCENTS] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Purple,
        Self::Magenta,
    ];

    /// The swatch's position in a palette, neutrals first. This is also its
    /// base16 index.
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::N0 => "n0",
            Self::N1 => "n1",
            Self::N2 => "n2",
            Self::N3 => "n3",
            Self::N4 => "n4",
            Self::N5 => "n5",
            Self::N6 => "n6",
            Self::N7 => "n7",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Cyan => "cyan",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Magenta => "magenta",
        }
    }
}

impl ColorRef {
    /// An opaque reference to `swatch`.
    pub const fn new(swatch: Swatch) -> Self {
        Self {
            swatch,
            opacity: 1.0,
        }
    }

    /// The same swatch with the given opacity.
    pub const fn with_opacity(self, opacity: f32) -> Self {
        Self { opacity, ..self }
    }

    /// The colour this refers to in `palette`.
    pub fn resolve(self, palette: &Palette) -> egui::Color32 {
        palette.get(self.swatch).gamma_multiply(self.opacity)
    }
}

fn opaque() -> f32 {
    1.0
}

fn is_opaque(opacity: &f32) -> bool {
    *opacity == 1.0
}

/// Format an opaque colour as `#rrggbb`.
pub fn to_hex(color: egui::Color32) -> String {
    let [r, g, b, _] = color.to_array();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Parse an opaque colour from `rrggbb`, with or without a leading `#`.
pub fn from_hex(s: &str) -> Option<egui::Color32> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some(egui::Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

/// Serde for a swatch array as a list of `#rrggbb` strings.
mod hex {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S, const N: usize>(
        colors: &[egui::Color32; N],
        s: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.collect_seq(colors.iter().map(|&c| super::to_hex(c)))
    }

    pub fn deserialize<'de, D, const N: usize>(d: D) -> Result<[egui::Color32; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let strs = Vec::<String>::deserialize(d)?;
        let colors = strs
            .iter()
            .map(|s| {
                super::from_hex(s).ok_or_else(|| D::Error::custom(format!("bad colour {s:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let n = colors.len();
        colors
            .try_into()
            .map_err(|_| D::Error::invalid_length(n, &format!("{N} colours").as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorRef, Palette, Swatch, from_hex, to_hex};

    #[test]
    fn swatches_index_neutrals_then_accents() {
        let all = Swatch::NEUTRALS.into_iter().chain(Swatch::ACCENTS);
        for (i, swatch) in all.enumerate() {
            assert_eq!(swatch.index(), i);
        }
    }

    #[test]
    fn get_mut_edits_the_swatch_get_reads() {
        let mut palette = Palette::dark();
        for swatch in Swatch::NEUTRALS.into_iter().chain(Swatch::ACCENTS) {
            let color = egui::Color32::from_rgb(swatch.index() as u8, 1, 2);
            *palette.get_mut(swatch) = color;
            assert_eq!(palette.get(swatch), color);
        }
    }

    #[test]
    fn hex_round_trips() {
        let color = egui::Color32::from_rgb(0x12, 0xab, 0xff);
        assert_eq!(to_hex(color), "#12abff");
        assert_eq!(from_hex("#12abff"), Some(color));
        assert_eq!(from_hex("12ABFF"), Some(color));
        assert_eq!(from_hex("#12abf"), None);
        assert_eq!(from_hex("#12abfg"), None);
    }

    #[test]
    fn opacity_scales_alpha() {
        let palette = Palette::dark();
        let half = ColorRef::new(Swatch::N7)
            .with_opacity(0.5)
            .resolve(&palette);
        assert!((half.a() as i32 - 128).abs() <= 1);
    }
}
