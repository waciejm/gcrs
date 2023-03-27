use clap::Parser;

use gcroot::GCRoots;

mod args;
pub mod gcroot;

pub fn run() -> eyre::Result<()> {
    args::Args::parse();
    println!("{:#?}", GCRoots::from_nix_store_command()?);
    Ok(())
}
