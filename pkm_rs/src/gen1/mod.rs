//! Generation I Pokémon: Red, Blue, Yellow and their Japanese counterparts.
//!
//! The Game Boy stores a Pokémon as a flat, unencrypted, unchecksummed record
//! whose stat fields are recomputed from DVs, EVs and level on every load, so
//! reading is a matter of fixed offsets rather than the section shuffling the
//! later generations need.

mod pk1;
mod pokemon_index;

pub use pk1::*;
pub use pokemon_index::{Gen1PokemonIndex, InvalidGen1PokemonIndex};

/// A Pokémon as it sits in a Western box: the record with no names attached.
pub(crate) const BOX_SIZE: usize = 33;
/// The same record plus the trainer name and nickname the box keeps beside it.
pub(crate) const BOX_SIZE_WESTERN: usize = 69;
/// The Japanese boxes give both names five characters instead of eleven.
pub(crate) const BOX_SIZE_JAPAN: usize = 59;
