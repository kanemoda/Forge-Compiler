//! Forge — C17 compiler CLI entry point.
//!
//! Parses command-line arguments with `clap` and delegates to [`forge_driver`]
//! for all compilation work.

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use forge_diagnostics::render_diagnostics;
use forge_driver::compile;

/// The Forge C17 compiler.
#[derive(Debug, Parser)]
#[command(
    name = "forge",
    version,
    about = "A C17 compiler with e-graph optimization, verified passes, and energy-aware codegen.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile a C source file to a native executable.
    Build {
        /// The C source file to compile.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Write the output executable to this path.
        #[arg(short, long, value_name = "FILE", default_value = "a.out")]
        output: PathBuf,
    },

    /// Check a C source file for errors without producing an executable.
    Check {
        /// The C source file to check.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Build { file, output: _ } => {
            run_compile(&file);
        }
        Command::Check { file } => {
            run_compile(&file);
        }
    }
}

/// Read `file`, invoke the compiler pipeline, and report any diagnostics.
///
/// Exits with code 1 on error, 0 on success.
fn run_compile(file: &PathBuf) {
    let filename = file.to_string_lossy();

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("forge: error: cannot read '{}': {}", filename, e);
            process::exit(1);
        }
    };

    match compile(&filename, &source) {
        Ok(()) => {}
        Err(diagnostics) => {
            render_diagnostics(&source, &filename, &diagnostics);
            process::exit(1);
        }
    }
}
