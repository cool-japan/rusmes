//! Man page generation command

use anyhow::Result;
use clap::CommandFactory;
use clap_mangen::Man;
use std::io::Write;

/// Write roff-formatted man page for the given CLI application to stdout.
///
/// The output can be piped directly into `man -l -` for immediate viewing, or
/// stored in a system man directory (e.g. `/usr/local/share/man/man1/`).
///
/// # Errors
/// Returns an error if writing to stdout fails.
pub fn run<A>() -> Result<()>
where
    A: CommandFactory,
{
    let cmd = A::command();
    let man = Man::new(cmd);
    let mut stdout = std::io::stdout();
    man.render(&mut stdout)?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify man page output contains the `.TH` roff macro, which is the
    /// mandatory title header for a well-formed man page.
    #[test]
    fn man_page_produces_valid_roff() {
        use crate::CliApp;
        use std::io::Write;

        let cmd = CliApp::command();
        let man = Man::new(cmd);
        let mut buf: Vec<u8> = Vec::new();
        man.render(&mut buf).expect("man page render");
        buf.flush().ok();

        let output = String::from_utf8(buf).expect("utf-8");
        assert!(
            output.contains(".TH"),
            "man page output should contain the .TH roff macro — got:\n{output}"
        );
    }
}
