use anyhow::Result;
use clap::Parser;
use xshell::{cmd, Shell};

#[derive(Parser)]
#[command(author, version, about)]
enum Cli {
    /// Generate code coverage.
    Coverage {
        #[clap(raw = true)]
        args: Vec<String>,
    },
}

fn main() -> Result<()> {
    let sh = Shell::new()?;

    match Cli::parse() {
        Cli::Coverage { args } => {
            sh.set_var("LLVM_PROFILE_FILE", "target/coverage/profile-%p.profraw");
            sh.set_var("RUSTFLAGS", "-C instrument-coverage");

            cmd!(sh, "cargo test").run()?;

            let grcov_args = [
                "target/coverage",
                "--binary-path",
                "target/debug",
                "--source-dir",
                ".",
                "--excl-start",
                "mod tests",
                "--excl-line",
                "#\\[",
                "--ignore",
                "/*",
                "--ignore",
                "examples/*",
            ];
            cmd!(sh, "grcov {grcov_args...} {args...}").run()?;
        }
    }

    Ok(())
}
