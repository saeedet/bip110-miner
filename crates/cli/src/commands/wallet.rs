//! `bip110-miner wallet` — where rewards go, and whether you can spend them.
//!
//! The question that matters most for this chain is the last one. Its address
//! format is Bitcoin's, so any Bitcoin wallet produces addresses the node
//! accepts — but spending what arrives needs a wallet that knows this chain.
//! A wallet on this node does.

use serde_json::{Value, json};

use crate::config::{Settings, network_key};
use crate::node;

/// Runs the command.
pub fn run(settings: &Settings) -> Result<(), String> {
    let Some(address) = &settings.address else {
        println!("no reward address for {} yet. Save one with:", network_key(settings.network));
        println!("\n    bip110-miner setup --address <your address>");
        return Ok(());
    };
    println!("address   {address}");

    let Ok(client) = node::client(settings) else {
        println!("checks    start the node to check it: `bip110-miner`");
        return Ok(());
    };

    let valid = client.validate_address(address).map(|info| info.is_valid).unwrap_or(false);
    println!(
        "network   {}",
        if valid { format!("valid for {}", network_key(settings.network)) } else { format!("NOT valid for {}", network_key(settings.network)) }
    );

    let unopenable = open_wallets(&client);

    let wallets: Vec<String> = client.call("listwallets", json!([])).unwrap_or_default();
    let owner = wallets.iter().find(|wallet| {
        client
            .call_wallet::<Value>(wallet, "getaddressinfo", json!([address]))
            .ok()
            .and_then(|info| info.get("ismine").and_then(Value::as_bool))
            .unwrap_or(false)
    });

    match owner {
        Some(wallet) => {
            let info: Value = client.call_wallet(wallet, "getwalletinfo", json!([])).unwrap_or(Value::Null);
            let encrypted = info.get("unlocked_until").is_some();
            println!(
                "spending  wallet \"{wallet}\" on this node can spend it{}",
                if encrypted { " (passphrase-protected)" } else { " — NOT passphrase-protected" }
            );
        }
        None if !unopenable.is_empty() => {
            println!("spending  can't tell: this node has a wallet it cannot open");
            for (name, error) in &unopenable {
                println!("\n  wallet \"{name}\"");
                if error.contains("pruned") {
                    // The one case worth explaining in full, because the node's
                    // own advice — re-download the whole chain — is far more
                    // than it usually takes, and the cause is easy to avoid.
                    println!("    fell behind the chain while it wasn't open, and the blocks it");
                    println!("    would need to catch up have since been pruned. Its keys are safe");
                    println!("    in the file, but the node won't open it without re-downloading");
                    println!("    the chain. If it never received anything, a new wallet that");
                    println!("    stays open is the simple fix.");
                } else {
                    println!("    {error}");
                }
            }
        }
        None => println!(
            "spending  not in any wallet on this node. That's fine if you hold its keys in a \
             wallet that knows this chain; a Bitcoin-only wallet will see the address but not the coins."
        ),
    }
    Ok(())
}

/// Opens every wallet on disk that isn't open yet, and returns the ones that
/// would not open, with the node's reason.
///
/// A wallet on disk is not necessarily loaded: the node forgets which were
/// open when it restarts. Opening them here, for the check only, keeps a
/// wallet that is merely closed from being reported as missing — which a
/// newcomer would reasonably read as lost coins. Opening a wallet this way
/// does not change what the node opens next time.
pub fn open_wallets(client: &node_rpc::RpcClient) -> Vec<(String, String)> {
    let on_disk: Value = client.call("listwalletdir", json!([])).unwrap_or(Value::Null);
    let loaded: Vec<String> = client.call("listwallets", json!([])).unwrap_or_default();

    on_disk
        .get("wallets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|wallet| wallet.get("name").and_then(Value::as_str))
        .filter(|name| !loaded.iter().any(|loaded| loaded == name))
        .filter_map(|name| {
            client
                .call::<Value>("loadwallet", json!([name, false]))
                .err()
                .map(|error| (name.to_owned(), error.to_string()))
        })
        .collect()
}
