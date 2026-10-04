//! `apple-kb-monitord` — headless owner of the Apple keyboard.

#[macro_use]
extern crate akm_core;

pub mod actor;
pub mod alias;
pub mod bluez;
pub mod client;
pub mod config_api;
pub mod devices;
pub mod diagnose;
pub mod events;
pub mod forget;
pub mod keymap;
pub mod linkq;
pub mod notify;
pub mod notify_policy;
pub mod origin;
pub mod osd;
pub mod passive;
pub mod powerdevil;
pub mod privileged;
pub mod reapply;
pub mod repair;
pub mod service;
pub mod settings;
pub mod shutdown;
pub mod sleep;
pub mod spawn_ui;
#[doc(hidden)]
/// Private session bus of the tests: never in the shipped binary (feature `testbus`, on for tests).
#[cfg(any(test, feature = "testbus"))]
pub mod testbus;
pub mod usage;
pub mod watcher;
