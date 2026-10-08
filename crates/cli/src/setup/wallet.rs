//! A new wallet on the node, protected by a passphrase, with a backup.
//!
//! # The two things that make this safe
//!
//! The passphrase is typed into this program, shown only as dots, and sent
//! straight to the node over its local RPC connection. It is never displayed,
//! written to a file, logged, or put on a command line where other programs
//! or the shell's history could see it.
//!
//! The wallet loads every time the node starts. A wallet left closed stops
//! following the chain, and on a pruned node it can fall so far behind that
//! the blocks it would need to catch up are gone — after which the node will
//! not open it at all without re-downloading the whole chain.

use std::path::PathBuf;

use node_rpc::RpcClient;
use serde_json::{Value, json};

use super::screen::{Answer, Part, Wizard};
use crate::chain;
use crate::platform;

/// Makes the wallet and returns its first address.
pub fn create(wizard: &mut Wizard, client: &RpcClient) -> Result<Answer<String>, String> {
    let intro = [Part::Text("Choose a passphrase. It encrypts the wallet stored on this computer.".into())];
    let after = [
        Part::Gap,
        Part::Text("• It's never shown, saved, logged, or sent anywhere but your own node.".into()),
        Part::Text("• You only need it to SPEND rewards, never to mine.".into()),
        Part::Text(
            "• If you lose it, coins in this wallet are gone for good. Write it down somewhere safe now."
                .into(),
        ),
    ];
    let passphrase = match wizard.passphrase(&intro, &after)? {
        Answer::Given(passphrase) => passphrase,
        Answer::Back => return Ok(Answer::Back),
        Answer::Quit => return Ok(Answer::Quit),
    };

    wizard.show(&[Part::Text("Creating the wallet…".into())], "")?;
    let name = free_name(client);
    // createwallet name, disable_private_keys, blank, passphrase, avoid_reuse,
    // descriptors, load_on_startup.
    let created: Result<Value, _> =
        client.call("createwallet", json!([name, false, false, passphrase, false, true, true]));
    drop(passphrase);
    created.map_err(|error| format!("the node could not create the wallet: {error}"))?;

    let address: String = client
        .call_wallet(&name, "getnewaddress", json!(["mining rewards", "bech32"]))
        .map_err(|error| format!("the wallet was created but would not give an address: {error}"))?;

    let backup = backup_path(&name);
    let backed_up = client.call_wallet::<Value>(&name, "backupwallet", json!([backup.display().to_string()]));

    let mut done = vec![
        Part::Text(format!("Your wallet \"{name}\" is ready. Rewards will go to:")),
        Part::Gap,
        Part::Text(format!("    {address}")),
        Part::Gap,
    ];
    if backed_up.is_ok() {
        done.push(Part::Text(format!(
            "A backup copy is at {}. Keep a copy somewhere other than this computer — a USB stick, \
             or your password manager. It is encrypted with your passphrase, so it needs both to spend.",
            backup.display()
        )));
    } else if let Err(error) = backed_up {
        done.push(Part::Text(format!(
            "Saving a backup to {} did not work ({error}). Make one before relying on this wallet.",
            backup.display()
        )));
    }
    if wizard.notice(&done, "enter continue · q quit")? {
        Ok(Answer::Given(address))
    } else {
        Ok(Answer::Quit)
    }
}

/// The program's wallet name, or that name with a number if it is taken.
fn free_name(client: &RpcClient) -> String {
    let on_disk: Value = client.call("listwalletdir", json!([])).unwrap_or(Value::Null);
    let taken: Vec<&str> = on_disk
        .get("wallets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|wallet| wallet.get("name").and_then(Value::as_str))
        .collect();
    std::iter::once(chain::WALLET_NAME.to_owned())
        .chain((2..).map(|n| format!("{}-{n}", chain::WALLET_NAME)))
        .find(|name| !taken.contains(&name.as_str()))
        .expect("an unbounded search always finds a free name")
}

/// Where the backup goes: the home folder, under a name that isn't taken.
fn backup_path(name: &str) -> PathBuf {
    let home = platform::home();
    std::iter::once(home.join(format!("{name}.backup")))
        .chain((2..).map(|n| home.join(format!("{name}.{n}.backup"))))
        .find(|path| !path.exists())
        .expect("an unbounded search always finds a free name")
}
