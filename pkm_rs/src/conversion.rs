//! Lookup tables converting between a generation's own indices and modern ones.
//!
//! These tables were written before anything consumed them and sat outside the
//! crate graph; the Generation I work is the first thing to need them, so only
//! the submodules that are actually read are declared here.

pub mod gameboy_string_encoding;
pub mod gen1_pokemon_index;
