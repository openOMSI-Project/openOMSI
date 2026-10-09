//! The game's side of the plugin API's functions (`omsi_plugin::PluginIo`, implemented in
//! `plugins.rs`): what each one reads of the game and how it changes it, a file per part.

pub(crate) mod bus;
pub(crate) mod duty;
pub(crate) mod world;
