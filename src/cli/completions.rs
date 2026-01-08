use clap::CommandFactory;
use clap_complete::generate;
use std::io;

use super::commands::{Cli, Shell};

pub fn generate_completions(shell: Shell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let shell: clap_complete::Shell = shell.into();
    generate(shell, &mut cmd, name, &mut io::stdout());
}
