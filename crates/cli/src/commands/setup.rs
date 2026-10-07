//! `bip110-miner setup` — save settings.
//!
//! For now this takes its answers as flags. A friendlier version that asks
//! for whatever is missing, right in the terminal, replaces it.

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
