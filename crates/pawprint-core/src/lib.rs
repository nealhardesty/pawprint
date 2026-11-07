//! `pawprint-core`: every decision PAWPRINT makes, with no platform
//! dependencies (SPEC §10). Backends translate native events into
//! [`badge::Event`]s, execute [`badge::Effect`]s through a
//! [`surface::Surface`], and sleep until [`badge::Badge::next_deadline`].

pub mod accent;
pub mod anim;
pub mod badge;
pub mod catalog;
pub mod config;
pub mod config_edit;
pub mod geom;
pub mod hash;
pub mod host;
pub mod identity;
pub mod layout;
pub mod matching;
pub mod menu;
pub mod movemode;
pub mod render;
pub mod state;
pub mod surface;

pub use pawprint_sprite as sprite;
