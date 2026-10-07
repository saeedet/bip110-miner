//! `bip110-miner doctor` — check everything mining depends on.
//!
//! Each check says what it found and, when it fails, what to do about it.
//! Exits non-zero if anything is wrong, so it also works in a script.

use std::net::TcpListener;

use node_rpc::Network;
use serde_json::{Value, json};

use super::{grouped, short_address};
use crate::chain::advertises_blake2b;
use crate::config::{self, Settings, network_key};
use crate::node;

/// How a check came out.
enum Outcome {
    Ok(String),
    Problem { found: String, fix: String },
    /// Not checked, because something it depends on failed first.
    Skipped(String),
}

/// Runs the command.
pub fn run(settings: &Settings) -> Result<(), String> {
    let mut problems = 0;
    let mut report = |name: &str, outcome: Outcome| match outcome {
        Outcome::Ok(found) => println!("  ✔ {name:<14} {found}"),
        Outcome::Skipped(why) => println!("  · {name:<14} {why}"),
        Outcome::Problem { found, fix } => {
            problems += 1;
            println!("  ✗ {name:<14} {found}\n  {:<16} → {fix}", "");
        }
    };

    println!("checking {} mining on this machine\n", network_key(settings.network));

    let path = config::path();
    report(
        "settings",
        Outcome::Ok(if path.exists() { path.display().to_string() } else { "defaults (no file yet)".into() }),
    );

    report("node software", match node::installed_version(settings) {
        Ok(version) if version >= node::MIN_KNOTS => {
            Outcome::Ok(format!("Bitcoin Knots {}", node::version_string(version)))
        }
        Ok(version) => Outcome::Problem {
            found: format!("Bitcoin Knots {} is too old", node::version_string(version)),
            fix: format!("install {} or newer", node::version_string(node::MIN_KNOTS)),
        },
        Err(error) => Outcome::Problem { found: error, fix: "install Bitcoin Knots".into() },
    });

    let client = node::client(settings)
        .ok()
        .filter(|client| client.get_blockchain_info().is_ok());

    report("node", match &client {
        Some(client) => match node::running_version(client) {
            Some(version) if version < node::MIN_KNOTS => Outcome::Problem {
                found: format!("running Knots {}, too old to mine with", node::version_string(version)),
                fix: "update it, then `bip110-miner stop` and start again".into(),
            },
            _ => Outcome::Ok(format!("running, RPC on port {}", settings.rpc_port)),
        },
        None => Outcome::Problem {
            found: "not running".into(),
            fix: "`bip110-miner` starts it".into(),
        },
    });

    let info = client.as_ref().and_then(|client| client.get_blockchain_info().ok());
    report("chain", match &info {
        None => Outcome::Skipped("needs the node".into()),
        Some(info) if info.chain != settings.network.as_str() => Outcome::Problem {
            found: format!("the node is on {}, not {}", info.chain, network_key(settings.network)),
            fix: "check the node's data directory".into(),
        },
        Some(info) if info.initial_block_download || info.headers > info.blocks + 2 => Outcome::Problem {
            found: format!(
                "catching up: block {} of {}",
                grouped(u64::from(info.blocks)),
                grouped(u64::from(info.headers))
            ),
            fix: "leave it running; mining waits for it".into(),
        },
        Some(info) => Outcome::Ok(format!("in sync at block {}", grouped(u64::from(info.blocks)))),
    });

    let peers: Option<Vec<Value>> = client.as_ref().and_then(|client| client.call("getpeerinfo", json!([])).ok());
    report("peers", match &peers {
        None => Outcome::Skipped("needs the node".into()),
        // A regtest chain exists only on this machine, so there is nobody to meet.
        Some(peers) if peers.is_empty() && settings.network == Network::Regtest => {
            Outcome::Ok("none, as expected on regtest".into())
        }
        Some(peers) if peers.is_empty() => Outcome::Problem {
            found: "none".into(),
            fix: "check the internet connection".into(),
        },
        Some(peers) => {
            let fork = peers.iter().filter(|peer| advertises_blake2b(peer)).count();
            if fork == 0 {
                Outcome::Problem {
                    found: format!("{} peers, none on BIP-110", peers.len()),
                    fix: "wait a few minutes; the node looks for them".into(),
                }
            } else {
                Outcome::Ok(format!("{} connected, {fork} on BIP-110", peers.len()))
            }
        }
    });

    report("reward address", match (&settings.address, &client) {
        (None, _) if settings.network == Network::Regtest => {
            Outcome::Ok("not needed: regtest coins are worthless, so a throwaway address is used".into())
        }
        (None, _) => Outcome::Problem {
            found: "not set".into(),
            fix: "`bip110-miner setup --address <your address>`".into(),
        },
        (Some(address), None) => Outcome::Skipped(format!("{} (needs the node to check)", short_address(address))),
        (Some(address), Some(client)) => match client.validate_address(address) {
            Ok(info) if info.is_valid => Outcome::Ok(short_address(address)),
            Ok(_) => Outcome::Problem {
                found: format!("{} is not a valid {} address", short_address(address), network_key(settings.network)),
                fix: "set the right one with `bip110-miner setup --address`".into(),
            },
            Err(error) => Outcome::Problem { found: error.to_string(), fix: "check the node".into() },
        },
    });

    report("wallets", match &client {
        None => Outcome::Skipped("needs the node".into()),
        Some(client) => {
            let stuck = super::wallet::open_wallets(client);
            match stuck.first() {
                None => Outcome::Ok("every wallet on this node opens".into()),
                Some((name, error)) if error.contains("pruned") => Outcome::Problem {
                    found: format!("\"{name}\" fell behind the pruned chain and won't open"),
                    fix: "if it never received anything, create a new wallet that stays open".into(),
                },
                Some((name, error)) => Outcome::Problem {
                    found: format!("\"{name}\" won't open: {error}"),
                    fix: "see `bip110-miner wallet`".into(),
                },
            }
        }
    });

    report("mining port", match TcpListener::bind(settings.listen) {
        Ok(_) => Outcome::Ok(format!("{} is free", settings.listen)),
        Err(_) => Outcome::Problem {
            found: format!("{} is in use", settings.listen),
            fix: "another miner may already be running — quit it first".into(),
        },
    });

    if problems == 0 {
        println!("\nall good — run `bip110-miner` to mine");
        Ok(())
    } else {
        println!("\n{problems} thing{} to fix", if problems == 1 { "" } else { "s" });
        // Already said; the empty error only sets the exit status.
        Err(String::new())
    }
}
