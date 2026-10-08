//! Setup: making sure everything mining needs is in place, and asking about
//! whatever isn't.
//!
//! Runs before mining whenever the program is in a terminal. Each step checks
//! first and asks only if something is missing, so a machine that is already
//! set up goes straight to the dashboard, and one where something changed —
//! the node needs updating, say — is asked about that alone.
//!
//! The steps, in order:
//!
//! 1. **This computer** — what it is, and whether the disk has room.
//! 2. **Node software** — installed and new enough, or offered.
//! 3. **Reward address** — set and valid, or asked for. Asked before the
//!    blockchain on purpose: the sync can take hours, and a newcomer should
//!    be able to answer everything and then walk away.
//! 4. **Blockchain** — caught up, or followed until it is, with a quick start
//!    offered where one exists.

pub mod address;
mod download;
mod install;
mod screen;
mod sync;
mod wallet;

#[cfg(test)]
mod snapshots;

use std::time::Duration;

use node_rpc::Network;

use crate::commands::short_address;
use crate::config::{self, Settings, network_key};
use crate::{chain, node, platform};
use screen::{Answer, Item, Mark, Part, Wizard};

/// How setup ended.
pub enum Outcome {
    /// Everything is in place: mine.
    Ready,
    /// The user stopped, or there is something they have to do first.
    Quit,
}

const COMPUTER: &str = "This computer";
const SOFTWARE: &str = "Node software";
const ADDRESS: &str = "Reward address";
const BLOCKCHAIN: &str = "Blockchain";

/// Checks everything, asking about what's missing. `ask_address` asks where
/// rewards go even when an address is already set, for `setup` itself.
pub fn run(settings: &mut Settings, ask_address: bool) -> Result<Outcome, String> {
    if !ask_address && already_ready(settings) {
        return Ok(Outcome::Ready);
    }
    let title = if config::path().exists() { "getting ready" } else { "getting set up" };
    let items = [COMPUTER, SOFTWARE, ADDRESS, BLOCKCHAIN]
        .map(|name| Item { name, mark: Mark::Waiting, detail: String::new() })
        .to_vec();
    let mut wizard = Wizard::open(title, items);
    let result = steps(&mut wizard, settings, ask_address);
    wizard.close();
    result
}

/// Whether there is nothing to ask, so setup need not show at all.
fn already_ready(settings: &Settings) -> bool {
    let software = node::installed_version(settings).is_ok_and(|version| version >= node::MIN_VERSION);
    let Ok(client) = node::client(settings) else { return false };
    let Ok(info) = client.get_blockchain_info() else { return false };
    let current = node::running_version(&client).is_none_or(|version| version >= node::MIN_VERSION);
    let address = settings.network == Network::Regtest || settings.address.is_some();
    let synced = settings.network == Network::Regtest
        || (!info.initial_block_download && info.headers > 0 && info.headers <= info.blocks + 2);
    software && current && address && synced
}

fn steps(wizard: &mut Wizard, settings: &mut Settings, ask_address: bool) -> Result<Outcome, String> {
    // 1. This computer.
    let free = platform::free_bytes(&settings.datadir);
    let free_text = free.map_or_else(String::new, |bytes| format!(" · {} free", download::size(bytes)));
    wizard.set(COMPUTER, Mark::Done, format!("{}{free_text}", platform::machine()));
    let needed = chain::disk_needed(settings.network);
    if free.is_some_and(|free| free < needed) {
        wizard.set(COMPUTER, Mark::Now, format!("{}{free_text}", platform::machine()));
        let intro = [Part::Text(format!(
            "The blockchain needs about {} of disk on {}, and there is less free than that. It may \
             fill the disk before it finishes.",
            download::size(needed),
            network_key(settings.network)
        ))];
        let choices = ["Carry on anyway".to_owned(), "Stop here, so I can make room".to_owned()];
        if !matches!(wizard.choose(&intro, &choices)?, Answer::Given(0)) {
            return Ok(Outcome::Quit);
        }
        wizard.set(COMPUTER, Mark::Done, format!("{}{free_text} (tight)", platform::machine()));
    }

    // 2. Node software.
    wizard.set(SOFTWARE, Mark::Now, "checking…");
    let found = node::installed_version(settings).ok();
    if found.is_none_or(|version| version < node::MIN_VERSION) {
        wizard.set(
            SOFTWARE,
            Mark::Now,
            found.map_or("not installed".to_owned(), |v| {
                format!("{} {} — out of date", chain::NODE_NAME, node::version_string(v))
            }),
        );
        if let Answer::Quit | Answer::Back = install::offer(wizard, settings, found)? {
            return Ok(Outcome::Quit);
        }
    }
    let version = node::installed_version(settings)?;
    wizard.set(SOFTWARE, Mark::Done, format!("{} {}", chain::NODE_NAME, node::version_string(version)));

    // The node itself, which the remaining steps talk to.
    if let Answer::Quit = start_node(wizard, settings)? {
        return Ok(Outcome::Quit);
    }
    let client = node::client(settings)?;
    wizard.set(BLOCKCHAIN, Mark::Waiting, "");

    // 3. Reward address.
    let still_valid = settings
        .address
        .as_deref()
        .is_some_and(|address| client.validate_address(address).is_ok_and(|info| info.is_valid));
    if settings.network == Network::Regtest && !ask_address {
        wizard.set(ADDRESS, Mark::Done, "not needed: regtest pays a throwaway address");
    } else if still_valid && !ask_address {
        wizard.set(ADDRESS, Mark::Done, short_address(settings.address.as_deref().unwrap_or_default()));
    } else {
        let current = settings.address.as_deref().map_or("not set".to_owned(), short_address);
        wizard.set(ADDRESS, Mark::Now, current);
        match address::ask(wizard, settings, &client)? {
            Answer::Given(address) => wizard.set(ADDRESS, Mark::Done, short_address(&address)),
            Answer::Back | Answer::Quit => return Ok(Outcome::Quit),
        }
    }

    // 4. Blockchain.
    if settings.network == Network::Regtest {
        wizard.set(BLOCKCHAIN, Mark::Done, "regtest makes its own");
        return Ok(Outcome::Ready);
    }
    wizard.set(BLOCKCHAIN, Mark::Now, "catching up");
    match sync::wait(wizard, settings, &client)? {
        Answer::Given(()) => {
            wizard.set(BLOCKCHAIN, Mark::Done, "in sync");
            Ok(Outcome::Ready)
        }
        Answer::Back | Answer::Quit => Ok(Outcome::Quit),
    }
}

/// Starts the node if it isn't running, restarting one too old to mine with.
fn start_node(wizard: &mut Wizard, settings: &mut Settings) -> Result<Answer<()>, String> {
    wizard.set(BLOCKCHAIN, Mark::Waiting, "starting the node…");

    if node::is_running(settings) {
        let client = node::client(settings)?;
        match node::running_version(&client) {
            Some(version) if version < node::MIN_VERSION => {
                let body = [Part::Text(format!(
                    "The node running now is {} {}, started before the update. Restarting it on the new version…",
                    chain::NODE_NAME,
                    node::version_string(version)
                ))];
                wizard.show(&body, "")?;
                node::stop(settings)?;
                while node::is_running(settings) {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            _ => return Ok(Answer::Given(())),
        }
    } else if platform::port_in_use(settings.rpc_port) {
        // Something else answers on the node's port: another node, most
        // likely, with its own data and cookie. Move ours rather than fight.
        let taken = settings.rpc_port;
        let free = platform::free_port_from(taken + 1).ok_or("no free port for the node")?;
        let path = config::path();
        let mut file = config::File::load(&path)?;
        file.node.rpc_port = Some(free);
        file.save(&path)?;
        settings.rpc_port = free;
        let body = [Part::Text(format!(
            "Another program is using port {taken}, which this node normally uses. It will use port {free} instead, from now on."
        ))];
        if !wizard.notice(&body, "enter continue · q quit")? {
            return Ok(Answer::Quit);
        }
    }

    let mut drawn = Ok(());
    node::start(settings, |elapsed| {
        let body = [
            Part::Text("Starting the node…".into()),
            Part::Text(format!(
                "{} so far. Loading its database can take a few minutes on a big chain.",
                crate::dashboard::format::duration(elapsed.as_secs_f64())
            )),
        ];
        if drawn.is_ok() {
            drawn = wizard.show(&body, "");
        }
    })?;
    drawn?;
    Ok(Answer::Given(()))
}
