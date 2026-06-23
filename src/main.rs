//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod tui;

fn usage() {
    println!("a3s {} — A3S coding agent CLI\n", env!("CARGO_PKG_VERSION"));
    println!("usage:");
    println!("  a3s code         launch the interactive coding agent (TUI)");
    println!("  a3s --version    show version");
    println!("  a3s --help       show this help");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("code") => tui::run().await,
        Some("-V") | Some("--version") => {
            println!("a3s {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None | Some("-h") | Some("--help") | Some("help") => {
            usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("a3s: unknown command '{other}' — try 'a3s --help'");
            std::process::exit(2);
        }
    }
}
