//! Shell completion generation command

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::Shell;

/// Generate shell completions for the given shell and write to stdout.
///
/// # Errors
/// Returns an error if `shell` is not a recognised shell variant — in practice
/// `clap_complete::Shell` is an exhaustive enum so the only failure path is I/O.
pub fn run<A>(shell: Shell) -> Result<()>
where
    A: CommandFactory,
{
    let mut cmd = A::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, &bin_name, &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify bash completions include every top-level subcommand name.
    #[test]
    fn completions_bash_contains_all_subcommands() {
        use crate::CliApp;

        let mut buf: Vec<u8> = Vec::new();
        let mut cmd = CliApp::command();
        let bin_name = cmd.get_name().to_string();
        clap_complete::generate(Shell::Bash, &mut cmd, &bin_name, &mut buf);

        // Flush is implicit for Vec<u8>, but ensure we have output.
        let output = String::from_utf8(buf).expect("utf-8");
        assert!(
            !output.is_empty(),
            "bash completion output should be non-empty"
        );

        // Top-level subcommand names that must appear in the completion script.
        let required = [
            "status", "migrate", "user", "mailbox", "queue", "backup", "restore", "man",
        ];
        for name in required {
            assert!(
                output.contains(name),
                "bash completions should mention subcommand '{name}' — got:\n{output}"
            );
        }
    }
}
