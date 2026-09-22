//! A Generation I internal species index, validated on the way in.
//!
//! Generation I addresses species by an internal index that bears no relation
//! to Pokédex order, and leaves 39 of the 190 slots unused — the gaps the
//! community calls MissingNo. Those gaps are rejected rather than passed
//! through, so a corrupt index becomes an error at the edge of the format
//! instead of a nonsense species deeper in.
//!
//! The table itself lives in [`crate::conversion::gen1_pokemon_index`], which
//! already carried both directions; this is the typed wrapper around it, in
//! the shape [`crate::gen3::Gen3PokemonIndex`] set.

use crate::conversion::gen1_pokemon_index;
use crate::result::{Error, PokemonIndexType};

use pkm_rs_types::NationalDex;
use serde::Serialize;
use std::num::NonZeroU8;

#[cfg(feature = "randomize")]
use pkm_rs_types::randomize::Randomize;
#[cfg(feature = "randomize")]
use rand::RngExt;

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg_attr(feature = "wasm", wasm_bindgen)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Gen1PokemonIndex(NonZeroU8);

const INVALID_INDEX_MESSAGE: &str =
    "Gen1PokemonIndex should always be valid for conversion to NationalDex";

impl Gen1PokemonIndex {
    /// The index as Generation I writes it, rejecting the MissingNo gaps.
    pub const fn new(gen1_index: u8) -> Result<Self, InvalidGen1PokemonIndex> {
        if let Some(non_zero) = NonZeroU8::new(gen1_index)
            && gen1_pokemon_index::decode(gen1_index).is_some()
        {
            Ok(Self(non_zero))
        } else {
            Err(InvalidGen1PokemonIndex(gen1_index as u16))
        }
    }

    /// The index for a National Dex number, where Generation I has one.
    pub const fn from_national_dex(national_dex: u16) -> Result<Self, InvalidGen1PokemonIndex> {
        if national_dex > u8::MAX as u16 {
            return Err(InvalidGen1PokemonIndex(national_dex));
        }

        if let Some(gen1_index) = gen1_pokemon_index::encode(national_dex as u8)
            && let Some(non_zero) = NonZeroU8::new(gen1_index)
        {
            Ok(Self(non_zero))
        } else {
            Err(InvalidGen1PokemonIndex(national_dex))
        }
    }

    pub fn to_national_dex(self) -> NationalDex {
        gen1_pokemon_index::decode(self.0.get())
            .map(|ndex| NationalDex::new(ndex as u16))
            .expect(INVALID_INDEX_MESSAGE)
            .expect(INVALID_INDEX_MESSAGE)
    }

    pub const fn to_byte(self) -> u8 {
        self.0.get()
    }
}

impl Default for Gen1PokemonIndex {
    fn default() -> Self {
        // Index 1 is Rhydon, the first entry in Generation I's internal table.
        Self(unsafe { NonZeroU8::new_unchecked(1) })
    }
}

#[cfg(feature = "randomize")]
impl Randomize for Gen1PokemonIndex {
    fn randomized<R: rand::prelude::Rng>(rng: &mut R) -> Self {
        loop {
            if let Ok(index) = Self::from_national_dex(rng.random_range(1..=151)) {
                return index;
            }
        }
    }
}

impl From<Gen1PokemonIndex> for u8 {
    fn from(gen1_index: Gen1PokemonIndex) -> Self {
        gen1_index.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidGen1PokemonIndex(u16);

impl From<InvalidGen1PokemonIndex> for Error {
    fn from(error: InvalidGen1PokemonIndex) -> Self {
        Error::PokemonGameIndex {
            value: error.0,
            source: PokemonIndexType::Gen1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_valid_index() {
        let mut valid = 0;

        for raw in 1..=u8::MAX {
            let Ok(index) = Gen1PokemonIndex::new(raw) else {
                continue;
            };

            valid += 1;
            let ndex = index.to_national_dex();
            let back = Gen1PokemonIndex::from_national_dex(ndex as u16)
                .expect("a decoded species must encode back");

            assert_eq!(back.to_byte(), raw, "index {raw} did not survive the trip");
        }

        // 152 table entries, less the 0 => 0 pair that NonZeroU8 rejects.
        assert_eq!(valid, 151);
    }

    #[test]
    fn rejects_missingno_gaps() {
        // 31, 32 and 50 are three of the 39 unused Generation I slots.
        for gap in [31u8, 32, 50, 52, 56] {
            assert!(
                Gen1PokemonIndex::new(gap).is_err(),
                "index {gap} is a MissingNo slot and should be rejected"
            );
        }
    }

    #[test]
    fn rejects_zero_and_out_of_range() {
        assert!(Gen1PokemonIndex::new(0).is_err());
        assert!(Gen1PokemonIndex::from_national_dex(0).is_err());
        // Nothing past Mew has a Generation I index.
        assert!(Gen1PokemonIndex::from_national_dex(152).is_err());
        assert!(Gen1PokemonIndex::from_national_dex(1000).is_err());
    }

    #[test]
    fn maps_the_landmark_species() {
        // Rhydon sits at index 1, the quirk every Gen I hacker knows.
        assert_eq!(
            Gen1PokemonIndex::new(1).unwrap().to_national_dex() as u16,
            112
        );
        // Mew is index 21.
        assert_eq!(
            Gen1PokemonIndex::new(21).unwrap().to_national_dex() as u16,
            151
        );
        // Bulbasaur, index 153.
        assert_eq!(
            Gen1PokemonIndex::from_national_dex(1).unwrap().to_byte(),
            153
        );
    }
}
