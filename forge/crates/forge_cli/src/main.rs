// Pedantic lints we've audited and accept as style preferences for this crate.
#![allow(
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::unreadable_literal,
    clippy::doc_markdown,
    clippy::wildcard_imports,
    clippy::needless_pass_by_value,
    clippy::manual_let_else
)]

//! Forge — C17 compiler CLI entry point.
//!
//! Parses command-line arguments with `clap` and delegates to [`forge_driver`]
//! for all compilation work.
//!
//! # Usage shapes
//!
//! ```text
//! forge [flags] FILE                # gcc-style: default mode is a full build
//! forge -E   [flags] FILE           # preprocess only, write to stdout
//! forge check [flags] FILE          # lex + preprocess, dump token stream
//! forge build [flags] FILE -o OUT   # explicit build subcommand
//! ```
//!
//! All preprocessor flags (`-I`, `-D`, `-U`) are `global = true`, so they
//! may appear either before the subcommand word or inside it — matching
//! the ergonomics of `gcc`.
//!
//! # Exit codes
//!
//! `0` if the pipeline produced no error-severity diagnostics; `1`
//! otherwise.  Warnings do not cause a non-zero exit.

use std::path::{Path, PathBuf};
use std::process;

use clap::{ArgAction, Parser, Subcommand};
use forge_diagnostics::render_diagnostics;
use forge_driver::{
    compile, format_token, parse_cli_define, tokens_to_source, CliDefine, CompileOptions,
    CompileOutput, TargetArch, TokenKind,
};

/// The Forge C17 compiler.
#[derive(Debug, Parser)]
#[command(
    name = "forge",
    version,
    about = "A C17 compiler with e-graph optimization, verified passes, and energy-aware codegen.",
    long_about = None,
)]
struct Cli {
    /// Source file to operate on — required when no subcommand is given.
    #[arg(value_name = "FILE", global = false)]
    file: Option<PathBuf>,

    /// Preprocess only: emit the post-preprocessor C source to stdout.
    ///
    /// With `-E` the driver stops after the preprocessor and prints a
    /// reconstructed C source text (similar to `gcc -E` / `clang -E`,
    /// minus `# <line>` linemarker comments).
    #[arg(short = 'E', long = "preprocess-only", global = true)]
    preprocess_only: bool,

    /// Prepend DIR to the `#include` search path (repeatable, searched
    /// in command-line order, before auto-detected system paths).
    #[arg(short = 'I', value_name = "DIR", global = true, action = ArgAction::Append)]
    include_paths: Vec<PathBuf>,

    /// Define a macro.  Accepts `NAME`, `NAME=value`, or
    /// `'NAME(params)=body'`.  A bare name is defined as `1`.  Processed
    /// before `-U` flags.
    #[arg(short = 'D', value_name = "MACRO", global = true, action = ArgAction::Append)]
    defines: Vec<String>,

    /// Undefine a macro (processed after every `-D`).
    #[arg(short = 'U', value_name = "MACRO", global = true, action = ArgAction::Append)]
    undefines: Vec<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Sub-command shapes — kept for the cargo-style `forge check / build`
/// invocation form.  The `gcc`-style form (`forge [-E] FILE`) uses the
/// top-level positional `file` field instead.
#[derive(Debug, Subcommand)]
enum Command {
    /// Compile a C source file to a native executable.
    ///
    /// The backend is not yet wired up, so `build` today runs exactly
    /// the same pipeline as `check` (lex + preprocess) and writes
    /// nothing to the output path.  The `-o` argument is accepted for
    /// forward compatibility.
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
    /// `check` runs the full lex + preprocess pipeline, prints every
    /// post-preprocessor token to stdout in `KIND span=START..END 'text'`
    /// form, and finishes with a `preprocessing successful, N tokens`
    /// summary line so test harnesses can assert on the token count.
    Check {
        /// The C source file to check.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

/// Which pipeline operation the user asked for, once flags and any
/// subcommand have been merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// `-E`: write reconstructed preprocessed source to stdout.
    Preprocess,
    /// `check`: lex + preprocess, emit the token stream.
    Check,
    /// `build` (or default when a file is given with no `-E`): full
    /// pipeline — currently no-op past the preprocessor.
    Build,
}

fn main() {
    let cli = Cli::parse();

    let (file, mode, build_output) = match resolve_invocation(&cli) {
        Ok(tuple) => tuple,
        Err(msg) => {
            eprintln!("forge: error: {msg}");
            process::exit(1);
        }
    };

    let mut defines: Vec<CliDefine> = Vec::with_capacity(cli.defines.len());
    for raw in &cli.defines {
        match parse_cli_define(raw) {
            Ok(d) => defines.push(d),
            Err(e) => {
                eprintln!("forge: error: invalid -D argument: {e}");
                process::exit(1);
            }
        }
    }

    let options = CompileOptions {
        include_paths: cli.include_paths.clone(),
        defines,
        undefines: cli.undefines.clone(),
        target_arch: TargetArch::default(),
    };

    run_compile(&file, mode, &options, build_output.as_deref());
}

/// Merge the top-level positional file and any subcommand into a single
/// `(file, mode, build_output)` triple.
///
/// * `forge FILE`                → `(FILE, Build, None)`
/// * `forge -E FILE`             → `(FILE, Preprocess, None)`
/// * `forge check FILE`          → `(FILE, Check, None)`
/// * `forge build FILE -o OUT`   → `(FILE, Build, Some(OUT))`
///
/// Conflicts (e.g., both a top-level file and a subcommand) produce a
/// textual error the caller prints and exits on.
fn resolve_invocation(cli: &Cli) -> Result<(PathBuf, Mode, Option<PathBuf>), String> {
    match (&cli.command, &cli.file, cli.preprocess_only) {
        (Some(Command::Build { file, output }), None, false) => {
            Ok((file.clone(), Mode::Build, Some(output.clone())))
        }
        (Some(Command::Build { file, .. } | Command::Check { file }), None, true)
        | (None, Some(file), true) => Ok((file.clone(), Mode::Preprocess, None)),
        (Some(Command::Check { file }), None, false) => Ok((file.clone(), Mode::Check, None)),
        (None, Some(file), false) => Ok((file.clone(), Mode::Build, None)),
        (Some(_), Some(_), _) => Err("cannot combine a positional FILE with a subcommand; \
             put FILE inside the subcommand or drop the subcommand"
            .to_string()),
        (None, None, _) => Err(
            "no input file\n\nUsage: forge [-E] [-I DIR]... [-D MACRO]... [-U MACRO]... FILE\n   \
             or: forge check FILE\n   or: forge build FILE [-o OUT]"
                .to_string(),
        ),
    }
}

/// Read `file`, invoke the compiler pipeline, and dispatch on `mode`.
///
/// * Every diagnostic (error **and** warning) is rendered to stderr
///   against the driver's `effective_source` (which may include a
///   CLI-synthesised prelude in addition to the user's source).
/// * `Preprocess` mode writes the reconstructed preprocessed source to
///   stdout via [`tokens_to_source`].
/// * `Check` mode prints every token to stdout via [`format_token`] and
///   finishes with a summary line the lit test harness can match on.
/// * `Build` mode currently only runs lex + preprocess — the later
///   pipeline stages are not yet wired in — and writes nothing to the
///   output path.
/// * The process exits with code `1` iff the pipeline produced at least
///   one error-severity diagnostic; warnings alone do not fail the build.
fn run_compile(file: &Path, mode: Mode, options: &CompileOptions, _build_output: Option<&Path>) {
    let filename = file.to_string_lossy();

    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("forge: error: cannot read '{filename}': {e}");
            process::exit(1);
        }
    };

    let output: CompileOutput = compile(&filename, &source, options);

    if !output.diagnostics.is_empty() {
        render_diagnostics(&output.effective_source, &filename, &output.diagnostics);
    }

    match mode {
        Mode::Preprocess => {
            // `print!` rather than `println!` — tokens_to_source already
            // appends a trailing newline when the stream is non-empty.
            print!("{}", tokens_to_source(&output.tokens));
        }
        Mode::Check => {
            for tok in &output.tokens {
                println!("{}", format_token(&output.effective_source, tok));
            }
            let token_count = output
                .tokens
                .iter()
                .filter(|t| !matches!(t.kind, TokenKind::Eof))
                .count();
            println!("preprocessing successful, {token_count} tokens");
        }
        Mode::Build => {
            // Backend not yet implemented — produce no artifact, just
            // surface diagnostics and let the exit code speak.
        }
    }

    if output.has_errors() {
        process::exit(1);
    }
}
