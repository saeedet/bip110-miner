//! `bip110-miner status` — where the node and the chain are.
//!
//! Reports both chainstates when there are two. A node started from a
//! snapshot mines on one while re-checking history on the other, and
//! `getblockchaininfo` only ever describes the first — so without this, several
//! hundred gigabytes of background work is invisible.

use node_rpc::Network;
use serde_json::{Value, json};

use super::{grouped, short_address};
use crate::config::{Settings, network_key};
use crate::node;

/// Runs the command.
pub fn run(settings: &Settings) -> Result<(), String> {
    println!("network   {}", network_key(settings.network));
    println!("power     {} ({} threads)", settings.power.name(), settings.threads);

    let Ok(client) = node::client(settings).and_then(|client| {
        client.get_blockchain_info().map_err(|error| error.to_string())?;
        Ok(client)
    }) else {
        println!("node      not running — `bip110-miner` starts it");
        return Ok(());
    };

    let info = client.get_blockchain_info().map_err(|error| error.to_string())?;
    let peers: Vec<Value> = client.call("getpeerinfo", json!([])).unwrap_or_default();
    let fork_peers = peers.iter().filter(|peer| advertises_blake2b(peer)).count();
    let version = node::running_version(&client).map_or("?".to_owned(), node::version_string);
    println!(
        "node      running · Knots {version} · {} peers ({fork_peers} on BIP-110)",
        peers.len()
    );

    let state = if info.initial_block_download || info.headers > info.blocks + 2 {
        format!("catching up, {} blocks to go", grouped(u64::from(info.headers.saturating_sub(info.blocks))))
    } else {
        "in sync".to_owned()
    };
    let age = unix_now().saturating_sub(info.time) / 60;
    println!("chain     block {} · {state} · newest block {age} min old", grouped(u64::from(info.blocks)));

    let chainstates: Value = client.call("getchainstates", json!([])).unwrap_or(Value::Null);
    println!("history   {}", history(&chainstates));

    let disk: Value = client.call("getblockchaininfo", json!([])).unwrap_or(Value::Null);
    if let Some(bytes) = disk.get("size_on_disk").and_then(Value::as_f64) {
        let pruned = disk.get("pruned").and_then(Value::as_bool).unwrap_or(false);
        println!("disk      {:.1} GB{}", bytes / 1e9, if pruned { ", pruned" } else { "" });
    }

    match &settings.address {
        Some(address) => println!("reward    {}", short_address(address)),
        None if settings.network == Network::Regtest => println!("reward    a throwaway address (regtest)"),
        None => println!("reward    not set — `bip110-miner setup --address <address>`"),
    }
    Ok(())
}

/// The background history check, in words.
fn history(chainstates: &Value) -> String {
    let states = chainstates.get("chainstates").and_then(Value::as_array);
    let Some(states) = states else { return "unknown".to_owned() };

    let background = states.iter().find(|state| state.get("snapshot_blockhash").is_none());
    let snapshot = states.iter().find(|state| state.get("snapshot_blockhash").is_some());

    match (background, snapshot, states.len()) {
        // One chainstate, validated: either a full sync, or a snapshot whose
        // history check has finished and retired itself.
        (_, _, 1) => "verified from 2009".to_owned(),
        (Some(background), Some(_), _) => {
            let blocks = background.get("blocks").and_then(Value::as_u64).unwrap_or(0);
            let work = background.get("verificationprogress").and_then(Value::as_f64).unwrap_or(0.0);
            format!(
                "re-checking from 2009 in the background: block {} · {:.1}% of the work",
                grouped(blocks),
                work * 100.0
            )
        }
        _ => "unknown".to_owned(),
    }
}

/// Whether a peer advertises `NODE_BLAKE2B`, the fork's service bit.
pub fn advertises_blake2b(peer: &Value) -> bool {
    peer.get("services")
        .and_then(Value::as_str)
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .is_some_and(|services| services & (1 << 28) != 0)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
