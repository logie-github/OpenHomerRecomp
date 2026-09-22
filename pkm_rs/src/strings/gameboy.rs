use serde::Serialize;
use std::fmt::Display;

use crate::conversion::gameboy_string_encoding;

#[cfg(feature = "randomize")]
use pkm_rs_types::randomize::Randomize;
#[cfg(feature = "randomize")]
use rand::{
    RngExt,
    distr::{Alphanumeric, SampleString},
};

/// Generation I and II end a name with 0x50 and pad the rest of the field
/// with it; 0x00 is a legitimate character, so it cannot serve as the end.
const TERMINATOR: u8 = 0x50;

/// What the games store for a character they have no glyph for.
const UNKNOWN_CHARACTER: u8 = 0xe6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GbString<const N: usize> {
    raw: [u8; N],
}

impl<const N: usize> From<&str> for GbString<N> {
    fn from(value: &str) -> Self {
        // The games fill a name field with the terminator and overwrite the
        // front of it, so an unused tail reads back as the end of the string
        // rather than as padding that has to be trimmed.
        let mut raw = [TERMINATOR; N];

        let encoded: Vec<u8> = value
            .chars()
            .map(|character| {
                gameboy_string_encoding::encode(character).unwrap_or(UNKNOWN_CHARACTER)
            })
            .collect();

        // One byte is always kept back so a full-length name still terminates.
        let len = encoded.len().min(N.saturating_sub(1));
        raw[..len].copy_from_slice(&encoded[..len]);

        GbString { raw }
    }
}

impl<const N: usize> From<String> for GbString<N> {
    fn from(value: String) -> Self {
        GbString::from(value.as_str())
    }
}

impl<const N: usize> Display for GbString<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let decoded: String = self
            .raw
            .iter()
            .copied()
            .take_while(|c| *c != TERMINATOR)
            .map(gameboy_string_encoding::decode)
            .map(|o| o.unwrap_or('\u{FFFD}'))
            .collect();

        write!(f, "{decoded}")
    }
}

impl<const N: usize> Default for GbString<N> {
    fn default() -> Self {
        Self { raw: [0; N] }
    }
}

impl<const N: usize> Serialize for GbString<N> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<const N: usize> GbString<N> {
    pub const fn from_bytes(bytes: [u8; N]) -> Self {
        GbString { raw: bytes }
    }

    pub const fn bytes(&self) -> [u8; N] {
        self.raw
    }
}

#[cfg(feature = "randomize")]
impl<const N: usize> Randomize for GbString<N> {
    fn randomized<R: rand::Rng>(rng: &mut R) -> Self {
        let length: usize = rng.random_range(0..N);
        let utf8: String = Alphanumeric.sample_string(rng, length);
        Self::from(utf8)
    }
}
