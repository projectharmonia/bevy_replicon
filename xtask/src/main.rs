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
            cmd!(sh, "cargo test")
                .env("LLVM_PROFILE_FILE", "target/coverage/profile-%p.profraw")
                .env("RUSTFLAGS", "-C instrument-coverage")
                .run()?;

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
