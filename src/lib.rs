use args::Command;
use clap::Parser;

use gcroot::GCRoots;

mod args;
pub mod gcroot;

pub fn run() -> eyre::Result<()> {
    let args = args::Args::parse();
    match args.command {
        Some(Command::Print) => println!("{}", GCRoots::from_nix_store_command()?),
        None => todo!(),
    }
    Ok(())
}
