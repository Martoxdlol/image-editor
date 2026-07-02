use serde::{Deserialize, Serialize};

/// Working color spaces (spec §5.1). Values are stored linear, unclamped
/// (HDR-safe); conversion to display happens at the output transform.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorSpace {
    #[default]
    SrgbLinear,
    DisplayP3Linear,
    Rec2020Linear,
    AcesCg,
}

/// HDR-safe color: linear-light components in the document working space,
/// unclamped f32, straight (non-premultiplied) alpha (spec §3.4).
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Debug)]
pub struct Color {
    pub space: ColorSpace,
    pub rgba: [f32; 4],
}

impl Default for Color {
    fn default() -> Self {
        Color::BLACK
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

impl Color {
    pub const TRANSPARENT: Color = Color { space: ColorSpace::SrgbLinear, rgba: [0.0, 0.0, 0.0, 0.0] };
    pub const BLACK: Color = Color { space: ColorSpace::SrgbLinear, rgba: [0.0, 0.0, 0.0, 1.0] };
    pub const WHITE: Color = Color { space: ColorSpace::SrgbLinear, rgba: [1.0, 1.0, 1.0, 1.0] };

    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { space: ColorSpace::SrgbLinear, rgba: [r, g, b, a] }
    }

    /// From 8-bit sRGB-encoded values (what UI color pickers and hex send).
    pub fn from_srgb8(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::linear(
            srgb_to_linear(r as f32 / 255.0),
            srgb_to_linear(g as f32 / 255.0),
            srgb_to_linear(b as f32 / 255.0),
            a as f32 / 255.0,
        )
    }

    /// To 8-bit sRGB for SDR display/export — the output transform (clamps).
    pub fn to_srgb8(&self) -> [u8; 4] {
        let enc = |c: f32| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        [
            enc(self.rgba[0]),
            enc(self.rgba[1]),
            enc(self.rgba[2]),
            (self.rgba[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    /// Parse `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (SDR, sRGB-encoded).
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.trim().trim_start_matches('#');
        let (r, g, b, a) = match s.len() {
            3 | 4 => {
                let d = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok().map(|v| v * 17);
                (d(0)?, d(1)?, d(2)?, if s.len() == 4 { d(3)? } else { 255 })
            }
            6 | 8 => {
                let d = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
                (d(0)?, d(2)?, d(4)?, if s.len() == 8 { d(6)? } else { 255 })
            }
            _ => return None,
        };
        Some(Color::from_srgb8(r, g, b, a))
    }

    pub fn to_hex(&self) -> String {
        let [r, g, b, a] = self.to_srgb8();
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    pub fn with_alpha(&self, a: f32) -> Color {
        Color { space: self.space, rgba: [self.rgba[0], self.rgba[1], self.rgba[2], a] }
    }

    pub fn lerp(&self, other: &Color, t: f32) -> Color {
        let mut rgba = [0.0f32; 4];
        for i in 0..4 {
            rgba[i] = self.rgba[i] + (other.rgba[i] - self.rgba[i]) * t;
        }
        Color { space: self.space, rgba }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Color::from_hex("#ff8000").unwrap();
        assert_eq!(c.to_hex(), "#ff8000");
        let c = Color::from_hex("#ff800080").unwrap();
        assert_eq!(c.to_hex(), "#ff800080");
        let c = Color::from_hex("f00").unwrap();
        assert_eq!(c.to_hex(), "#ff0000");
        assert!(Color::from_hex("zzz").is_none());
    }

    #[test]
    fn srgb_transfer_is_linear_light() {
        // mid-gray sRGB 128 ≈ 0.2158 linear, NOT 0.5
        let c = Color::from_srgb8(128, 128, 128, 255);
        assert!((c.rgba[0] - 0.2158).abs() < 1e-3);
        assert_eq!(c.to_srgb8()[0], 128);
    }

    #[test]
    fn hdr_values_unclamped_in_storage() {
        let c = Color::linear(2.5, 1.0, 0.5, 1.0);
        assert_eq!(c.rgba[0], 2.5); // preserved
        assert_eq!(c.to_srgb8()[0], 255); // clamped only at output
    }
}
