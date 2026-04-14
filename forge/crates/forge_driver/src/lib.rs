//! Compilation pipeline orchestration for the Forge compiler.
//!
//! The driver is the glue between the CLI and each individual compiler phase.
//! It takes a source file, runs each phase in sequence, and returns either
//! a successful result or a list of [`Diagnostic`]s describing every error
//! encountered during compilation.
//!
//! # Current state
//!
//! Phase 0 stub: the pipeline accepts a file and returns `Ok(())` immediately.
//! Subsequent phases (lexer, preprocessor, parser, sema, IR, …) will be wired
//! in here as they are implemented.

pub use forge_diagnostics::Diagnostic;

/// Run the full compilation pipeline on the given source text.
///
/// # Arguments
///
/// * `filename` - The name of the source file (used in diagnostics).
/// * `source`   - The raw source text to compile.
///
/// # Errors
///
/// Returns `Err(diagnostics)` if any errors are produced during compilation.
/// All errors encountered are collected before returning so the user sees every
/// problem in a single compiler invocation.
pub fn compile(filename: &str, source: &str) -> Result<(), Vec<Diagnostic>> {
    // Phase 0 stub — all subsequent pipeline stages are wired in here.
    let _ = (filename, source);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_empty_source_succeeds() {
        let result = compile("empty.c", "");
        assert!(
            result.is_ok(),
            "compiling empty source should succeed in stub phase"
        );
    }

    #[test]
    fn test_compile_simple_source_succeeds() {
        let source = "int main(void) { return 0; }";
        let result = compile("main.c", source);
        assert!(
            result.is_ok(),
            "compiling a simple C snippet should succeed in stub phase"
        );
    }
}
