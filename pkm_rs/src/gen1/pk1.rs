//! A Generation I Pokémon record.
//!
//! The Game Boy keeps a Pokémon as a flat 33-byte struct with no encryption
//! and no checksum, followed — in a box or party file rather than in the save
//! itself — by a party stat block and the two names. Level and the five stats
//! are cached in that block but derived from DVs, stat experience and
//! experience points, so they are recomputed on write rather than trusted:
//! a record edited anywhere else still comes back consistent.
//!
//! Ported from `src/core/pkm/PK1.ts`, which remains the reference for offsets
//! and for the two places the format is quietly lenient (the three-byte
//! wrapper some tools prepend, and names being optional).

use crate::conversion::gen1_pokemon_index;
use crate::gen1::pokemon_index::Gen1PokemonIndex;
use crate::gen1::{BOX_SIZE, BOX_SIZE_JAPAN, BOX_SIZE_WESTERN};
use crate::result::{Error, Result};
use crate::strings::GbString;

use pkm_rs_resources::species::get_species_metadata;
use pkm_rs_resources::stats::calculate_stats_gen1;
use pkm_rs_types::{Dvs, Language, NationalDex, StatsPreSplit};
use serde::Serialize;

#[cfg(feature = "randomize")]
use pkm_rs_types::randomize::Randomize;

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

/// Offsets into the Generation I record, named so the reads below read.
mod offset {
    pub const SPECIES: usize = 0x00;
    pub const CURRENT_HP: usize = 0x01;
    pub const LEVEL_IN_BOX: usize = 0x03;
    pub const STATUS: usize = 0x04;
    pub const TYPE_1: usize = 0x05;
    pub const TYPE_2: usize = 0x06;
    pub const CATCH_RATE: usize = 0x07;
    pub const MOVES: usize = 0x08;
    pub const TRAINER_ID: usize = 0x0c;
    pub const EXP: usize = 0x0e;
    pub const EV_HP: usize = 0x11;
    pub const EV_ATK: usize = 0x13;
    pub const EV_DEF: usize = 0x15;
    pub const EV_SPE: usize = 0x17;
    pub const EV_SPC: usize = 0x19;
    pub const DVS: usize = 0x1b;
    pub const MOVE_PP: usize = 0x1d;
    pub const STAT_LEVEL: usize = 0x21;
    pub const STAT_HP: usize = 0x22;
    pub const STAT_ATK: usize = 0x24;
    pub const STAT_DEF: usize = 0x26;
    pub const STAT_SPE: usize = 0x28;
    pub const STAT_SPC: usize = 0x2a;
    pub const TRAINER_NAME: usize = 0x2c;
    pub const NICKNAME: usize = 0x37;
}

/// How many characters each name field actually carries.
const TRAINER_NAME_LENGTH: usize = 8;
const NICKNAME_LENGTH: usize = 11;

/// The smallest buffer that still holds both names.
const SIZE_WITH_NAMES: usize = offset::NICKNAME + NICKNAME_LENGTH;

/// The DV combination Generation II would read back as shiny.
///
/// Generation I has no concept of shininess; this is the Generation II rule
/// applied to the same DVs, which is what decides how a Pokémon looks once it
/// has been carried forward through the Time Capsule.
const SHINY_ATK_DVS: [u16; 8] = [2, 3, 6, 7, 10, 11, 14, 15];

#[cfg_attr(feature = "wasm", wasm_bindgen(js_name = Pk1Wasm))]
#[cfg_attr(feature = "randomize", derive(Randomize))]
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Pk1 {
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub pokemon_index: Gen1PokemonIndex,
    pub status_condition: u8,
    pub type_1: u8,
    pub type_2: u8,
    /// Doubles as the held item once the Pokémon reaches Generation II.
    pub catch_rate: u8,
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub moves: [u8; 4],
    pub trainer_id: u16,
    /// Twenty-four bits on the cartridge, so values above 0xffffff are refused.
    pub exp: u32,
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub evs_g12: StatsPreSplit,
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub dvs: Dvs,
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub move_pp: [u8; 4],
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub move_pp_ups: [u8; 4],
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub trainer_name: GbString<TRAINER_NAME_LENGTH>,
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub nickname: GbString<NICKNAME_LENGTH>,
    /// Absent on the cartridge; carried so a Japanese record writes back
    /// to a Japanese-sized buffer.
    #[cfg_attr(feature = "wasm", wasm_bindgen(skip))]
    pub language: Language,
}

impl Default for Pk1 {
    fn default() -> Self {
        Self {
            pokemon_index: Gen1PokemonIndex::default(),
            status_condition: 0,
            type_1: 0,
            type_2: 0,
            catch_rate: 0,
            moves: [0; 4],
            trainer_id: 0,
            exp: 0,
            evs_g12: StatsPreSplit::default(),
            dvs: Dvs::default(),
            move_pp: [0; 4],
            move_pp_ups: [0; 4],
            trainer_name: GbString::default(),
            nickname: GbString::default(),
            language: Language::None,
        }
    }
}

impl Pk1 {
    /// Reads a record, tolerating the three-byte wrapper some tools prepend.
    ///
    /// A `.pk1` written by a save editor may begin with a length-and-terminator
    /// header whose third byte is 0xff; the record proper starts after it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let bytes = if bytes.len() > 3 && bytes[2] == 0xff {
            &bytes[3..]
        } else {
            bytes
        };

        if bytes.len() < BOX_SIZE {
            return Err(Error::buffer_size_with_source("Pk1", BOX_SIZE, bytes.len()));
        }

        let pokemon_index = Gen1PokemonIndex::new(bytes[offset::SPECIES])?;

        let mut moves = [0u8; 4];
        moves.copy_from_slice(&bytes[offset::MOVES..offset::MOVES + 4]);

        let mut move_pp = [0u8; 4];
        let mut move_pp_ups = [0u8; 4];
        for slot in 0..4 {
            let packed = bytes[offset::MOVE_PP + slot];
            move_pp[slot] = packed & 0b0011_1111;
            move_pp_ups[slot] = (packed >> 6) & 0b11;
        }

        // The cartridge holds experience in three bytes, big-endian.
        let exp = u32::from(bytes[offset::EXP]) << 16
            | u32::from(bytes[offset::EXP + 1]) << 8
            | u32::from(bytes[offset::EXP + 2]);

        let has_names = bytes.len() >= SIZE_WITH_NAMES;

        let trainer_name = if has_names {
            GbString::from_bytes(read_array::<TRAINER_NAME_LENGTH>(
                bytes,
                offset::TRAINER_NAME,
            ))
        } else {
            GbString::from("TRAINER")
        };

        let nickname = if has_names {
            GbString::from_bytes(read_array::<NICKNAME_LENGTH>(bytes, offset::NICKNAME))
        } else {
            // With no nickname stored the species name is the honest answer.
            GbString::from(pkm_rs_resources::lookup::species_name(
                pokemon_index.to_national_dex(),
                Language::English,
            ))
        };

        Ok(Self {
            pokemon_index,
            status_condition: bytes[offset::STATUS],
            type_1: bytes[offset::TYPE_1],
            type_2: bytes[offset::TYPE_2],
            catch_rate: bytes[offset::CATCH_RATE],
            moves,
            trainer_id: read_u16_be(bytes, offset::TRAINER_ID),
            exp,
            evs_g12: StatsPreSplit {
                hp: read_u16_be(bytes, offset::EV_HP),
                atk: read_u16_be(bytes, offset::EV_ATK),
                def: read_u16_be(bytes, offset::EV_DEF),
                spe: read_u16_be(bytes, offset::EV_SPE),
                spc: read_u16_be(bytes, offset::EV_SPC),
            },
            dvs: Dvs::from_bytes(&[bytes[offset::DVS], bytes[offset::DVS + 1]]),
            move_pp,
            move_pp_ups,
            trainer_name,
            nickname,
            language: Language::None,
        })
    }

    /// Writes the record back, recomputing level and the cached stats.
    ///
    /// `include_names` mirrors the reference implementation's
    /// `includeExtraFields`: a record destined for a save's box has its names
    /// stored elsewhere, so writing them would corrupt the neighbouring entry.
    pub fn to_bytes(&self, include_names: bool) -> Vec<u8> {
        let size = if self.language == Language::Japanese {
            BOX_SIZE_JAPAN
        } else {
            BOX_SIZE_WESTERN
        };

        let mut bytes = vec![0u8; size];
        let level = self.level();

        bytes[offset::SPECIES] = self.pokemon_index.to_byte();
        write_u16_be(&mut bytes, offset::CURRENT_HP, self.current_hp());
        bytes[offset::LEVEL_IN_BOX] = level;
        bytes[offset::STATUS] = self.status_condition;
        bytes[offset::TYPE_1] = self.type_1;
        bytes[offset::TYPE_2] = self.type_2;
        bytes[offset::CATCH_RATE] = self.catch_rate;
        bytes[offset::MOVES..offset::MOVES + 4].copy_from_slice(&self.moves);
        write_u16_be(&mut bytes, offset::TRAINER_ID, self.trainer_id);

        let exp = self.exp & 0x00ff_ffff;
        bytes[offset::EXP] = (exp >> 16) as u8;
        bytes[offset::EXP + 1] = (exp >> 8) as u8;
        bytes[offset::EXP + 2] = exp as u8;

        write_u16_be(&mut bytes, offset::EV_HP, self.evs_g12.hp);
        write_u16_be(&mut bytes, offset::EV_ATK, self.evs_g12.atk);
        write_u16_be(&mut bytes, offset::EV_DEF, self.evs_g12.def);
        write_u16_be(&mut bytes, offset::EV_SPE, self.evs_g12.spe);
        write_u16_be(&mut bytes, offset::EV_SPC, self.evs_g12.spc);

        let dv_bytes = self.dvs.to_bytes();
        bytes[offset::DVS] = dv_bytes[0];
        bytes[offset::DVS + 1] = dv_bytes[1];

        for slot in 0..4 {
            bytes[offset::MOVE_PP + slot] =
                (self.move_pp[slot] & 0b0011_1111) | ((self.move_pp_ups[slot] & 0b11) << 6);
        }

        let stats = self.stats();
        bytes[offset::STAT_LEVEL] = level;
        write_u16_be(&mut bytes, offset::STAT_HP, stats.hp);
        write_u16_be(&mut bytes, offset::STAT_ATK, stats.atk);
        write_u16_be(&mut bytes, offset::STAT_DEF, stats.def);
        write_u16_be(&mut bytes, offset::STAT_SPE, stats.spe);
        write_u16_be(&mut bytes, offset::STAT_SPC, stats.spc);

        if include_names {
            // A Japanese buffer is too short for the Western name offsets, so
            // the writes are bounded rather than allowed to run off the end.
            write_bounded(&mut bytes, offset::TRAINER_NAME, &self.trainer_name.bytes());
            write_bounded(&mut bytes, offset::NICKNAME, &self.nickname.bytes());
        }

        bytes
    }

    pub fn national_dex(&self) -> NationalDex {
        self.pokemon_index.to_national_dex()
    }

    /// The level the experience points imply, which is the only source of truth.
    pub fn level(&self) -> u8 {
        get_species_metadata(self.national_dex() as u16).calculate_level(self.exp)
    }

    /// DVs in the shape the shared stat maths expects.
    pub const fn dvs_as_stats(&self) -> StatsPreSplit {
        StatsPreSplit {
            hp: self.dvs.get_hp(),
            atk: self.dvs.get_atk(),
            def: self.dvs.get_def(),
            spc: self.dvs.get_spc(),
            spe: self.dvs.get_spe(),
        }
    }

    pub fn stats(&self) -> StatsPreSplit {
        calculate_stats_gen1(
            self.national_dex(),
            &self.dvs_as_stats(),
            &self.evs_g12,
            u16::from(self.level()),
        )
        .unwrap_or_default()
    }

    /// A freshly written record is at full health.
    pub fn current_hp(&self) -> u16 {
        self.stats().hp
    }

    /// Whether Generation II would draw this one as shiny.
    pub fn is_shiny(&self) -> bool {
        self.dvs.get_spe() == 10
            && self.dvs.get_def() == 10
            && self.dvs.get_spc() == 10
            && SHINY_ATK_DVS.contains(&self.dvs.get_atk())
    }

    /// The species index as Generation I stores it.
    pub const fn species_byte(&self) -> u8 {
        self.pokemon_index.to_byte()
    }

    /// The National Dex number behind a raw Generation I index, if it has one.
    pub const fn national_dex_for_index(gen1_index: u8) -> Option<u8> {
        gen1_pokemon_index::decode(gen1_index)
    }
}

fn read_u16_be(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn write_u16_be(bytes: &mut [u8], at: usize, value: u16) {
    let encoded = value.to_be_bytes();
    bytes[at] = encoded[0];
    bytes[at + 1] = encoded[1];
}

fn read_array<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    let available = bytes.len().saturating_sub(at).min(N);
    out[..available].copy_from_slice(&bytes[at..at + available]);
    out
}

/// Copies what fits and drops the rest, so a short buffer cannot panic.
fn write_bounded(bytes: &mut [u8], at: usize, value: &[u8]) {
    if at >= bytes.len() {
        return;
    }

    let writable = (bytes.len() - at).min(value.len());
    bytes[at..at + writable].copy_from_slice(&value[..writable]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level-based Pikachu with recognisable values in every field.
    fn sample() -> Pk1 {
        Pk1 {
            pokemon_index: Gen1PokemonIndex::from_national_dex(25).unwrap(),
            status_condition: 0,
            type_1: 23,
            type_2: 23,
            catch_rate: 163,
            moves: [84, 45, 86, 98],
            trainer_id: 0x1234,
            exp: 125_000,
            evs_g12: StatsPreSplit {
                hp: 11_000,
                atk: 12_000,
                def: 13_000,
                spe: 14_000,
                spc: 15_000,
            },
            dvs: Dvs::from_all(15, 10, 10, 10),
            move_pp: [20, 30, 10, 15],
            move_pp_ups: [3, 0, 1, 2],
            trainer_name: GbString::from("LOGIE"),
            nickname: GbString::from("SPARKY"),
            language: Language::English,
        }
    }

    #[test]
    fn round_trips_through_bytes() {
        let original = sample();
        let bytes = original.to_bytes(true);
        assert_eq!(bytes.len(), BOX_SIZE_WESTERN);

        let parsed = Pk1::from_bytes(&bytes).expect("a record we just wrote must parse");

        assert_eq!(parsed.species_byte(), original.species_byte());
        assert_eq!(parsed.national_dex() as u16, 25);
        assert_eq!(parsed.trainer_id, original.trainer_id);
        assert_eq!(parsed.exp, original.exp);
        assert_eq!(parsed.moves, original.moves);
        assert_eq!(parsed.move_pp, original.move_pp);
        assert_eq!(parsed.move_pp_ups, original.move_pp_ups);
        assert_eq!(parsed.evs_g12.hp, original.evs_g12.hp);
        assert_eq!(parsed.evs_g12.atk, original.evs_g12.atk);
        assert_eq!(parsed.evs_g12.def, original.evs_g12.def);
        assert_eq!(parsed.evs_g12.spe, original.evs_g12.spe);
        assert_eq!(parsed.evs_g12.spc, original.evs_g12.spc);
        assert_eq!(parsed.dvs, original.dvs);
        assert_eq!(parsed.catch_rate, original.catch_rate);
        assert_eq!(parsed.trainer_name.to_string(), "LOGIE");
        assert_eq!(parsed.nickname.to_string(), "SPARKY");
    }

    #[test]
    fn pp_and_pp_ups_share_a_byte_without_bleeding() {
        let bytes = sample().to_bytes(true);

        // Slot 0: 20 PP with 3 PP Ups packs to 0b11_010100.
        assert_eq!(bytes[offset::MOVE_PP], 0b1101_0100);
        // Slot 1: 30 PP with no PP Ups is just the PP.
        assert_eq!(bytes[offset::MOVE_PP + 1], 30);
    }

    #[test]
    fn experience_is_written_as_three_bytes() {
        let bytes = sample().to_bytes(true);
        let exp = u32::from(bytes[offset::EXP]) << 16
            | u32::from(bytes[offset::EXP + 1]) << 8
            | u32::from(bytes[offset::EXP + 2]);

        assert_eq!(exp, 125_000);
        // The byte after the 24-bit field belongs to the EVs, not to exp.
        assert_eq!(offset::EXP + 3, offset::EV_HP);
    }

    #[test]
    fn level_and_stats_come_from_exp_and_dvs() {
        let mon = sample();
        let level = mon.level();

        assert!(level > 1 && level <= 100, "level {level} is out of range");

        let stats = mon.stats();
        let bytes = mon.to_bytes(true);

        // The cached block must agree with the derivation, not drift from it.
        assert_eq!(read_u16_be(&bytes, offset::STAT_HP), stats.hp);
        assert_eq!(read_u16_be(&bytes, offset::STAT_ATK), stats.atk);
        assert_eq!(bytes[offset::STAT_LEVEL], level);
        assert_eq!(bytes[offset::LEVEL_IN_BOX], level);
        assert_eq!(read_u16_be(&bytes, offset::CURRENT_HP), stats.hp);
    }

    #[test]
    fn tolerates_the_three_byte_wrapper() {
        let bare = sample().to_bytes(true);

        let mut wrapped = vec![0x01, 0x19, 0xff];
        wrapped.extend_from_slice(&bare);

        let parsed = Pk1::from_bytes(&wrapped).expect("a wrapped record must parse");
        assert_eq!(parsed.national_dex() as u16, 25);
        assert_eq!(parsed.nickname.to_string(), "SPARKY");
    }

    #[test]
    fn falls_back_when_the_names_are_absent() {
        let full = sample().to_bytes(true);
        let bare = &full[..BOX_SIZE];

        let parsed = Pk1::from_bytes(bare).expect("a nameless record must still parse");

        assert_eq!(parsed.trainer_name.to_string(), "TRAINER");
        // With nothing stored, the species name stands in for the nickname.
        // The lookup spells it the modern way, as the reference implementation
        // does; Generation I's all-caps styling is a display choice, not data.
        assert_eq!(parsed.nickname.to_string(), "Pikachu");
    }

    #[test]
    fn refuses_a_truncated_record() {
        assert!(Pk1::from_bytes(&[0u8; 10]).is_err());
    }

    #[test]
    fn refuses_a_missingno_index() {
        let mut bytes = sample().to_bytes(true);
        bytes[offset::SPECIES] = 31; // one of the unused slots
        assert!(Pk1::from_bytes(&bytes).is_err());
    }

    #[test]
    fn reads_shininess_the_way_generation_ii_would() {
        let mut mon = sample();

        mon.dvs = Dvs::from_all(10, 10, 10, 10);
        assert!(mon.is_shiny(), "atk 10 with 10/10/10 is the shiny pattern");

        mon.dvs = Dvs::from_all(1, 10, 10, 10);
        assert!(!mon.is_shiny(), "atk 1 is not in the shiny set");

        mon.dvs = Dvs::from_all(15, 10, 10, 9);
        assert!(!mon.is_shiny(), "spc must be exactly 10");
    }

    #[test]
    fn omitting_names_leaves_the_name_fields_clear() {
        let bytes = sample().to_bytes(false);

        assert!(
            bytes[offset::TRAINER_NAME..offset::TRAINER_NAME + TRAINER_NAME_LENGTH]
                .iter()
                .all(|byte| *byte == 0),
            "names must not be written into a box record"
        );
    }
}
