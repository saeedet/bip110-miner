//! Where rewards go: an address pasted in, or a new wallet on the node.

use node_rpc::RpcClient;

use super::screen::{Answer, Part, Wizard};
use super::wallet;
use crate::chain;
use crate::config::{self, Settings, network_key};

/// The two ways to get a reward address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// Make a wallet on the node, protected by a passphrase.
    NewWallet,
    /// Paste an address from a wallet held elsewhere.
    Paste,
}

/// Asks where rewards should go, saves the answer, and returns it.
pub fn ask(wizard: &mut Wizard, settings: &mut Settings, client: &RpcClient) -> Result<Answer<String>, String> {
    let intro: Vec<Part> = chain::ADDRESS_INTRO.iter().flat_map(|text| [Part::Text((*text).into()), Part::Gap]).collect();
    let intro = &intro[..intro.len() - 1];
    let choices: Vec<String> = chain::ADDRESS_WAYS.iter().map(|(_, label)| (*label).to_owned()).collect();

    loop {
        let way = match wizard.choose(intro, &choices)? {
            Answer::Given(index) => chain::ADDRESS_WAYS[index].0,
            Answer::Back | Answer::Quit => return Ok(Answer::Quit),
        };
        let answer = match way {
            Way::Paste => paste(wizard, settings, client)?,
            Way::NewWallet => wallet::create(wizard, client)?,
        };
        match answer {
            Answer::Given(address) => {
                save(settings, &address)?;
                return Ok(Answer::Given(address));
            }
            Answer::Back => continue,
            Answer::Quit => return Ok(Answer::Quit),
        }
    }
}

/// Asks for an address and has the node check it.
fn paste(wizard: &mut Wizard, settings: &Settings, client: &RpcClient) -> Result<Answer<String>, String> {
    let network = network_key(settings.network);
    let intro = [Part::Text(format!(
        "Paste a {network} address from your wallet. It is checked here before anything is saved: a \
         mistyped or wrong-network address would mine perfectly good blocks that pay nobody."
    ))];
    wizard.text(&intro, "Address", |typed| {
        if typed.is_empty() {
            return Err("paste an address first".into());
        }
        match client.validate_address(typed) {
            Ok(info) if info.is_valid => Ok(()),
            Ok(_) => Err(format!("that is not a valid {network} address")),
            Err(error) => Err(format!("the node could not check it: {error}")),
        }
    })
}

/// Remembers `address` for this network.
fn save(settings: &mut Settings, address: &str) -> Result<(), String> {
    let path = config::path();
    let mut file = config::File::load(&path)?;
    file.address.insert(network_key(settings.network).to_owned(), address.to_owned());
    file.save(&path)?;
    settings.address = Some(address.to_owned());
    Ok(())
}
