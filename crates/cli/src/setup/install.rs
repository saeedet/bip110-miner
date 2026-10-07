//! Installing the node software: Bitcoin Knots, downloaded and checked.
//!
//! # What "checked" means
//!
//! Knots publishes a `SHA256SUMS` file signed by its maintainer. The hash of
//! the macOS build below was taken from that file after verifying the
//! signature against Luke Dashjr's signing key (fingerprint
//! 1A3E 761F 19D2 CC77 85C5 502E A291 A2C4 5D0C 504A). Building the hash in
//! means a newcomer needs no GnuPG: the download is accepted only if it is
//! byte for byte the file that was signed. A newer Knots is supported by a new
//! release of this program, which is the review a consensus-critical update
//! deserves anyway.

use std::path::{Path, PathBuf};

use super::download::{self, Fetched};
use super::screen::{Answer, Part, Wizard};
use crate::config::Settings;
use crate::node;
use crate::platform;

/// A Knots release this program knows how to install.
struct Release {
    /// The version, as people say it.
    version: &'static str,
    /// The file to download.
    file: &'static str,
    /// The folder the archive unpacks into.
    folder: &'static str,
    /// Where the file sits under the download site.
    path: &'static str,
    /// Its size.
    bytes: u64,
    /// Its SHA-256, from the signed `SHA256SUMS`.
    sha256: &'static str,
}

/// The release installed: the one with the 973,440 soft fork.
const RELEASE: Release = Release {
    version: "29.4.2",
    file: "bitcoin-29.4.2.knots20260508-arm64-apple-darwin.tar.gz",
    folder: "bitcoin-29.4.2.knots20260508",
    path: "29.x/29.4.2.knots20260508",
    bytes: 37_285_825,
    sha256: "230eda151c877752beffed294897aaee7c1ab8ed9f510b3ca5ad5de1292d3258",
};

/// Where Knots publishes its releases.
const OFFICIAL: &str = "https://bitcoinknots.org/files";

/// The download address: the official site, or a mirror named in
/// `BIP110_KNOTS_MIRROR`. A mirror changes only where the bytes come from;
/// they are held to the same built-in hash.
fn url() -> String {
    match std::env::var("BIP110_KNOTS_MIRROR").ok().filter(|mirror| !mirror.is_empty()) {
        Some(mirror) => format!("{}/{}", mirror.trim_end_matches('/'), RELEASE.file),
        None => format!("{OFFICIAL}/{}/{}", RELEASE.path, RELEASE.file),
    }
}

/// Where this program keeps the Knots it installs.
fn knots_dir() -> PathBuf {
    platform::state_dir().join("knots")
}

/// Offers to install Knots, or to update the one found (`found`).
///
/// On success, the node software at `settings.binaries` is ready to start.
pub fn offer(wizard: &mut Wizard, settings: &mut Settings, found: Option<(u32, u32, u32)>) -> Result<Answer<()>, String> {
    let mut intro = match found {
        None => vec![
            Part::Text(
                "The node is the program that talks to the BIP-110 network and checks every block \
                 for itself. It's called Bitcoin Knots, and it's free."
                    .into(),
            ),
        ],
        Some(version) => vec![
            Part::Text(format!(
                "This computer has Knots {}. The network changed a rule at block 973,440 that this \
                 version doesn't know about, so a block mined with it could be rejected.",
                node::version_string(version)
            )),
            Part::Gap,
            Part::Text(
                "What changed: newly mined coins now wait about 45 days (6,480 blocks) before they \
                 can be spent, instead of about 16 hours."
                    .into(),
            ),
        ],
    };
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        intro.push(Part::Gap);
        intro.push(Part::Text(
            "This program can only install Knots on Apple Silicon Macs for now. Install it \
             yourself from bitcoinknots.org, then run this again."
                .into(),
        ));
        wizard.notice(&intro, "enter quit")?;
        return Ok(Answer::Quit);
    }

    let action = if found.is_some() { "Update to" } else { "Download" };
    let choices = vec![
        format!("{action} Knots {} and check it's the official file    ({})", RELEASE.version, download::size(RELEASE.bytes)),
        "I'll install it myself — show me how".to_owned(),
    ];
    match wizard.choose(&intro, &choices)? {
        Answer::Given(0) => install(wizard, settings),
        Answer::Given(_) => {
            let how = vec![
                Part::Text(format!(
                    "Download Knots {} for macOS from bitcoinknots.org and unpack it. Then either put \
                     its programs in {}, or set BIP110_KNOTS to the folder holding bitcoind.",
                    RELEASE.version,
                    settings.binaries.display()
                )),
                Part::Gap,
                Part::Text("Run bip110-miner again afterwards and it carries on from here.".into()),
            ];
            wizard.notice(&how, "enter quit")?;
            Ok(Answer::Quit)
        }
        Answer::Back | Answer::Quit => Ok(Answer::Quit),
    }
}

/// Downloads, checks, unpacks and links the release.
fn install(wizard: &mut Wizard, settings: &mut Settings) -> Result<Answer<()>, String> {
    let archive = knots_dir().join("downloads").join(RELEASE.version).join(RELEASE.file);
    let intro = vec![Part::Text(format!("Downloading Bitcoin Knots {} from {}…", RELEASE.version, host(&url())))];
    if let Fetched::Stopped = download::fetch(wizard, &intro, &url(), &archive, RELEASE.bytes)? {
        return Ok(Answer::Quit);
    }

    wizard.show(&[Part::Text("Checking it's the official file…".into())], "")?;
    if let Err(problem) = check(&archive) {
        // Not the signed file: never keep it, never run it.
        let _ = std::fs::remove_file(&archive);
        return Err(problem);
    }

    wizard.show(&[Part::Text("Unpacking…".into())], "")?;
    let installed = unpack(&archive)?;
    settings.binaries = installed.join("bin");
    let version = node::installed_version(settings)?;
    if version < node::MIN_KNOTS {
        return Err(format!("the installed Knots reports version {}", node::version_string(version)));
    }
    Ok(Answer::Given(()))
}

/// The host part of a URL, for saying where a download comes from.
fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or(url)
}

/// Whether `archive` is exactly the signed release.
fn check(archive: &Path) -> Result<(), String> {
    let bytes = std::fs::read(archive).map_err(|error| format!("cannot read the download: {error}"))?;
    let found = hex(&sha256::sha256(&bytes));
    if found == RELEASE.sha256 {
        Ok(())
    } else {
        Err(format!(
            "the download is not the official Knots {} file (its SHA-256 is {found}, not {}). \
             It has been deleted and nothing was installed.",
            RELEASE.version, RELEASE.sha256
        ))
    }
}

/// Lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Unpacks the release to `knots/<version>` and points `knots/current` at it.
///
/// Unpacked beside the final place first, so a half-finished unpack never
/// looks like an install.
fn unpack(archive: &Path) -> Result<PathBuf, String> {
    let dir = knots_dir();
    let scratch = dir.join(format!("{}.unpacking", RELEASE.version));
    let _ = std::fs::remove_dir_all(&scratch);
    platform::unpack(archive, &scratch)?;

    let target = dir.join(RELEASE.version);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(scratch.join(RELEASE.folder), &target)
        .map_err(|error| format!("cannot move Knots into place: {error}"))?;
    let _ = std::fs::remove_dir_all(&scratch);

    // Replace the link in one step: make the new one beside it, then rename.
    let current = dir.join("current");
    let link = dir.join("current.new");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(RELEASE.version, &link).map_err(|error| format!("cannot link Knots: {error}"))?;
    std::fs::rename(&link, &current).map_err(|error| format!("cannot link Knots: {error}"))?;
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_hash_is_well_formed() {
        assert_eq!(RELEASE.sha256.len(), 64);
        assert!(RELEASE.sha256.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hashes_compare_as_lowercase_hex() {
        assert_eq!(hex(&[0x0a, 0xff]), "0aff");
        assert_eq!(
            hex(&sha256::sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_mirror_changes_only_the_host() {
        assert!(url().ends_with(RELEASE.file));
        assert_eq!(host("https://bitcoinknots.org/files/x"), "bitcoinknots.org");
    }
}
