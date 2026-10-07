//! `bip110-miner setup` — save settings.
//!
//! In a terminal and with nothing else given, it walks through the setup
//! checklist and asks where rewards go, even if an address is already set —
//! which makes it the way to change it. With flags, it saves them directly,
//! for scripts and anywhere without a terminal.

use std::io::IsTerminal;

use node_rpc::Network;

use crate::config::{self, File, Power, network_key};
use crate::node;

/// What to save.
pub struct Changes {
    /// The network the other changes apply to.
    pub network: Network,
    /// Whether `--network` was given, making it the default from now on.
    pub make_default: bool,
    /// A reward address for `network`.
    pub address: Option<String>,
    /// How hard to work.
    pub power: Option<Power>,
}

/// Runs the command.
pub fn run(changes: &Changes, settings: &config::Settings) -> Result<(), String> {
    let path = config::path();
    let mut file = File::load(&path)?;
    let mut said = Vec::new();

    if changes.make_default {
        file.network = Some(network_key(changes.network).to_owned());
        said.push(format!("network   {}", network_key(changes.network)));
    }

    if let Some(address) = &changes.address {
        // Checked against the node when it is up. A mistyped or wrong-network
        // address produces perfectly valid blocks paying nobody, so it is worth
        // catching here rather than after a lucky night.
        match node::client(settings).ok().and_then(|client| client.validate_address(address).ok()) {
            Some(info) if !info.is_valid => {
                return Err(format!("{address} is not a valid {} address", network_key(changes.network)));
            }
            Some(_) => said.push(format!("address   {address} (checked)")),
            None => said.push(format!("address   {address} (checked when mining starts)")),
        }
        file.address.insert(network_key(changes.network).to_owned(), address.clone());
    }

    if let Some(power) = changes.power {
        file.power = Some(power);
        said.push(format!("power     {}", power.name()));
    }

    // Nothing but perhaps a network: ask, interactively, where possible.
    if changes.address.is_none() && changes.power.is_none() && std::io::stdout().is_terminal() {
        if !said.is_empty() {
            file.save(&path)?;
        }
        let mut settings = settings.clone();
        if let crate::setup::Outcome::Ready = crate::setup::run(&mut settings, true)? {
            println!("all set — run `bip110-miner` to mine");
        }
        return Ok(());
    }

    if said.is_empty() {
        println!("nothing to change. For example:\n");
        println!("    bip110-miner setup --address <your address>");
        println!("    bip110-miner setup --power eco");
        println!("    bip110-miner --network testnet4 setup");
        return Ok(());
    }

    file.save(&path)?;
    println!("saved to {}", path.display());
    for line in said {
        println!("  {line}");
    }
    Ok(())
}
