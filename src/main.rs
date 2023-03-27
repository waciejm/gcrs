fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    gcrs::run()
}
