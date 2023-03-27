use gcrs::gcroot::GCRoots;

fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    println!("{:#?}", GCRoots::from_nix_store_command()?);

    Ok(())
}
