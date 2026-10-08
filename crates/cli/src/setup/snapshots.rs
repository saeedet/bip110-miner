//! Whole-screen snapshots of setup, as in `docs/cli-design.md`.
//!
//! Like the dashboard's: a changed layout shows up as a readable diff, and
//! `UPDATE_SNAPSHOTS=1` accepts a deliberate change.

use ratatui::text::Line;

use super::screen::{Item, Mark, Part, options, render};

fn items(marks: [(Mark, &str); 4]) -> Vec<Item> {
    ["This computer", "Node software", "Reward address", "Blockchain"]
        .into_iter()
        .zip(marks)
        .map(|(name, (mark, detail))| Item { name, mark, detail: detail.into() })
        .collect()
}

fn check(name: &str, lines: &[Line<'_>], width: usize, height: usize) {
    let text: Vec<String> =
        lines.iter().map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect()).collect();
    assert_eq!(text.len(), height, "{name}: every row is drawn");
    for (row, line) in text.iter().enumerate() {
        assert_eq!(line.chars().count(), width, "{name}: row {row} is not {width} wide: {line:?}");
    }
    let actual = text.join("\n");
    let path = format!("{}/src/setup/snapshots/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(std::path::Path::new(&path).parent().expect("has a parent")).expect("mkdir");
        std::fs::write(&path, format!("{actual}\n")).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected.trim_end_matches('\n') == actual,
        "{name} changed. Rerun with UPDATE_SNAPSHOTS=1 to accept.\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

const COMPUTER: (Mark, &str) = (Mark::Done, "Apple M3 · 8 cores · 146 GB free");

#[test]
fn first_run_asks_about_the_node() {
    let mut body = vec![
        Part::Text(
            "The node is the program that talks to the BIP-110 network and checks every block for \
             itself. It's called Bitcoin Knots, and it's free."
                .into(),
        ),
        Part::Gap,
    ];
    body.extend(options(
        &[
            "Download Knots 29.4.2 and check it's the official file    (37 MB)".into(),
            "I'll install it myself — show me how".into(),
        ],
        0,
    ));
    let screen = render(
        "getting set up",
        &items([COMPUTER, (Mark::Now, "not installed"), (Mark::Waiting, ""), (Mark::Waiting, "")]),
        &body,
        "↑↓ choose · enter confirm · q quit",
        80,
        24,
    );
    check("first_run_80x24", &screen, 80, 24);
}

#[test]
fn reward_address_choice() {
    let mut body = vec![
        Part::Text("If your computer ever finds a block, the reward goes to an address you own.".into()),
        Part::Gap,
        Part::Text(
            "For BIP-110 coins, the safest home is a wallet on this node. A normal Bitcoin wallet \
             app will accept the address, but can't spend the coins."
                .into(),
        ),
        Part::Gap,
    ];
    body.extend(options(
        &[
            "Create a wallet here, protected by a passphrase   (recommended)".into(),
            "I already have a BIP-110 address — let me paste it".into(),
        ],
        0,
    ));
    let screen = render(
        "getting set up",
        &items([
            COMPUTER,
            (Mark::Done, "Knots 29.4.2"),
            (Mark::Now, "not set"),
            (Mark::Waiting, "starting the node…"),
        ]),
        &body,
        "↑↓ choose · enter confirm · q quit",
        80,
        24,
    );
    check("address_80x24", &screen, 80, 24);
}

#[test]
fn passphrase_is_never_shown() {
    let body = vec![
        Part::Text("Choose a passphrase. It encrypts the wallet stored on this computer.".into()),
        Part::Gap,
        Part::Line(Line::raw("      Passphrase     ••••••••••••••")),
        Part::Line(Line::raw("      Once more      ••••••••••••••   ✔ match")),
        Part::Gap,
        Part::Text("• You only need it to SPEND rewards, never to mine.".into()),
    ];
    let screen = render(
        "getting set up",
        &items([
            COMPUTER,
            (Mark::Done, "Knots 29.4.2"),
            (Mark::Now, "creating a wallet"),
            (Mark::Waiting, ""),
        ]),
        &body,
        "enter continue · esc back",
        80,
        24,
    );
    check("passphrase_80x24", &screen, 80, 24);
}

#[test]
fn catching_up() {
    let body = vec![
        Part::Text(
            "Before mining, your computer needs its own copy of the blockchain, so it can check \
             every block itself instead of trusting anyone else."
                .into(),
        ),
        Part::Gap,
        Part::Line(Line::raw("    Catching up      ██████████████████░░░░░░░░░░   64%   about 2 hours left")),
        Part::Line(Line::raw("                     block 950,112 of 976,034")),
        Part::Gap,
        Part::Text("⏵ Mining starts on its own when this reaches 100%.".into()),
    ];
    let screen = render(
        "getting set up",
        &items([
            COMPUTER,
            (Mark::Done, "Knots 29.4.2"),
            (Mark::Done, "bc1qw508…f3t4"),
            (Mark::Now, "catching up"),
        ]),
        &body,
        "q quit (progress is kept) · mining starts on its own",
        80,
        24,
    );
    check("catching_up_80x24", &screen, 80, 24);
}
