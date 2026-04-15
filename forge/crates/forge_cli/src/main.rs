//! Forge — C17 compiler CLI entry point.
//!
//! Parses command-line arguments with `clap` and delegates to [`forge_driver`]
//! for all compilation work.

use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};
use forge_diagnostics::render_diagnostics;
use forge_driver::{compile, format_token, CompileOutput};

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
    ///
    /// While the pipeline only has a lexer, `check` prints the token stream
    /// to stdout, one token per line, in the format
    /// `KIND span=START..END 'text'`.  Diagnostics (if any) are rendered
    /// to stderr.
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
            run_compile(&file, /* print_tokens = */ false);
        }
        Command::Check { file } => {
            run_compile(&file, /* print_tokens = */ true);
        }
    }
}

/// Read `file`, invoke the compiler pipeline, and report results.
///
/// * Every diagnostic (error **or** warning) is rendered to stderr.
/// * When `print_tokens` is `true` (the `check` subcommand), every token
///   is also printed to stdout in `KIND span=START..END 'text'` form.
/// * The process exits with code `1` iff the pipeline produced at least one
///   error-severity diagnostic; warnings alone do not fail the build.
fn run_compile(file: &Path, print_tokens: bool) {
    let filename = file.to_string_lossy();

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("forge: error: cannot read '{filename}': {e}");
            process::exit(1);
        }
    };

    let output: CompileOutput = compile(&filename, &source);

    if !output.diagnostics.is_empty() {
        render_diagnostics(&source, &filename, &output.diagnostics);
    }

    if print_tokens {
        for tok in &output.tokens {
            println!("{}", format_token(&source, tok));
        }
    }

    if output.has_errors() {
        process::exit(1);
    }
}
