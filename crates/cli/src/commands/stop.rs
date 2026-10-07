//! `bip110-miner stop` — shut the node down.
//!
//! Mining stops when you quit it; the node does not, because a running node
//! stays in sync and the next start is immediate. This is for when you want
//! it gone: before an update, or to free the machine entirely.

use crate::config::{Settings, network_key};
use crate::node;

/// Runs the command.
pub fn run(settings: &Settings) -> Result<(), String> {
    if !node::is_running(settings) {
        println!("the {} node is not running", network_key(settings.network));
        return Ok(());
    }
    node::stop(settings)?;
    println!(
        "the {} node is shutting down — it saves its state first, which can take a minute",
        network_key(settings.network)
    );
    Ok(())
}
