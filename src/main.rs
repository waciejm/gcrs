use crate::gcroot::GCRoots;

use eyre::Result;

pub mod gcroot;

fn main() -> Result<()> {
    color_eyre::install()?;

    println!("{:#?}", GCRoots::from_nix_store_command().unwrap());

    Ok(())
}
