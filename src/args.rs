use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Print,
}
