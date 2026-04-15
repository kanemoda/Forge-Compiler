//! Compilation pipeline orchestration for the Forge compiler.
//!
//! The driver is the glue between the CLI and each individual compiler phase.
//! It takes a source file, runs each phase in sequence, and returns a
//! [`CompileOutput`] that bundles every artifact produced (currently just the
//! lexer's token stream) with every [`Diagnostic`] collected along the way.
//!
//! # Current state
//!
//! Only the lexer is wired in.  Subsequent phases (preprocessor, parser,
//! sema, IR, …) will be added here as they are implemented; each phase
//! appends its diagnostics to [`CompileOutput::diagnostics`] and contributes
//! its own artifact field when appropriate.
//!
//! # Token-stream output
//!
//! For the `check` subcommand the CLI prints every token on its own line in
//! the format produced by [`format_token`]:
//!
//! ```text
//! Int span=0..3 'int'
//! Identifier("main") span=4..8 'main'
//! ```
//!
//! This shape is considered a **public contract** — it is the format
//! consumed by the lit-style test suite.

pub use forge_diagnostics::{Diagnostic, Severity};
pub use forge_lexer::{Lexer, Token, TokenKind};

/// The aggregate result of running the compilation pipeline on a source file.
///
/// Contains both the artifacts produced by every completed phase and every
/// [`Diagnostic`] (error, warning, note) emitted along the way.  The CLI
/// renders diagnostics unconditionally and exits non-zero iff
/// [`CompileOutput::has_errors`] returns `true`.
#[derive(Debug, Clone)]
pub struct CompileOutput {
    /// The full token stream produced by the lexer, terminated by
    /// [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Diagnostics collected from every pipeline phase, in emission order.
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileOutput {
    /// `true` if at least one diagnostic has severity [`Severity::Error`].
    ///
    /// The CLI uses this to decide the process exit code.  Warnings do
    /// **not** cause a non-zero exit; they are informational only.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error))
    }
}

/// Run the full compilation pipeline on the given source text.
///
/// # Arguments
///
/// * `filename` — the display name of the source file (retained for future
///   phases that resolve include paths relative to it).
/// * `source`   — the raw source text to compile.
///
/// # Returns
///
/// A [`CompileOutput`] holding every token produced by the lexer and every
/// diagnostic (error or warning) encountered during compilation.  The caller
/// is responsible for rendering the diagnostics and deciding whether to fail
/// based on [`CompileOutput::has_errors`].
pub fn compile(filename: &str, source: &str) -> CompileOutput {
    // Retained for future phases — preprocessor include resolution, etc.
    let _ = filename;

    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize();
    let diagnostics = lexer.take_diagnostics();

    CompileOutput {
        tokens,
        diagnostics,
    }
}

/// Render a single token as a one-line `KIND span=START..END 'text'` string.
///
/// This is the format consumed by `forge check` and by the lit-style test
/// suite, so the shape is considered a public contract.
///
/// * `KIND` is the [`Debug`](std::fmt::Debug) representation of the token
///   kind — keyword variants render as their name (`Int`, `Return`, …),
///   punctuators as their name (`PlusEqual`, `LessLessEqual`, …), and
///   literal variants expand their inner fields so the numeric or textual
///   value is visible at a glance.
/// * `span=START..END` are the byte-offset bounds of the token in the source.
/// * `'text'` is the raw source slice the token covers.  For character and
///   string literals this naturally includes the surrounding quotes.
pub fn format_token(source: &str, tok: &Token) -> String {
    let start = tok.span.start as usize;
    let end = tok.span.end as usize;
    let text = source.get(start..end).unwrap_or("");
    format!("{:?} span={start}..{end} '{text}'", tok.kind)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_strings(output: &CompileOutput) -> Vec<String> {
        output
            .tokens
            .iter()
            .map(|t| format!("{:?}", t.kind))
            .collect()
    }

    // ---------- compile() basics ----------

    #[test]
    fn compile_empty_source_has_no_diagnostics() {
        let out = compile("empty.c", "");
        assert!(out.diagnostics.is_empty());
        assert!(!out.has_errors());
        // An empty input still yields a single Eof token.
        assert_eq!(out.tokens.len(), 1);
        assert!(matches!(out.tokens[0].kind, TokenKind::Eof));
    }

    #[test]
    fn compile_simple_source_has_no_errors() {
        let src = "int main(void) { return 0; }";
        let out = compile("main.c", src);
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        let kinds = kind_strings(&out);
        assert!(
            kinds.iter().any(|k| k == "Int"),
            "expected an Int keyword in {kinds:?}"
        );
        assert!(
            kinds.iter().any(|k| k == "Return"),
            "expected a Return keyword in {kinds:?}"
        );
    }

    #[test]
    fn compile_surfaces_lexer_errors() {
        // `0x` alone is a hex literal with no digits — the lexer emits an
        // error-severity diagnostic, which must surface on the driver output.
        let out = compile("bad.c", "0x");
        assert!(
            out.has_errors(),
            "expected errors, got diagnostics: {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn compile_surfaces_lexer_warnings_without_error_flag() {
        // Integer overflow produces a warning-severity diagnostic; it must
        // be visible on the output even though `has_errors` stays false.
        let out = compile("warn.c", "99999999999999999999999999");
        assert!(
            !out.has_errors(),
            "overflow is a warning, not an error: {:?}",
            out.diagnostics
        );
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d.severity, Severity::Warning)),
            "expected warning in {:?}",
            out.diagnostics
        );
    }

    // ---------- format_token() shape ----------

    #[test]
    fn format_token_keyword() {
        let src = "int";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert_eq!(line, "Int span=0..3 'int'");
    }

    #[test]
    fn format_token_punctuator() {
        let src = "+=";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert_eq!(line, "PlusEqual span=0..2 '+='");
    }

    #[test]
    fn format_token_identifier_includes_name_and_source_slice() {
        let src = "foo";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert!(line.starts_with("Identifier(\"foo\")"), "{line}");
        assert!(line.ends_with("span=0..3 'foo'"), "{line}");
    }

    #[test]
    fn format_token_integer_literal_shows_value_and_suffix() {
        let src = "42u";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert!(line.contains("IntegerLiteral"), "{line}");
        assert!(line.contains("value: 42"), "{line}");
        assert!(line.contains("suffix: U"), "{line}");
        assert!(line.ends_with("'42u'"), "{line}");
    }

    #[test]
    fn format_token_float_literal_shows_value_and_suffix() {
        let src = "1.5f";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert!(line.contains("FloatLiteral"), "{line}");
        assert!(line.contains("value: 1.5"), "{line}");
        assert!(line.contains("suffix: F"), "{line}");
    }

    #[test]
    fn format_token_char_literal_shows_value_and_prefix() {
        let src = "'A'";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert!(line.contains("CharLiteral"), "{line}");
        assert!(line.contains("value: 65"), "{line}");
        assert!(line.contains("prefix: None"), "{line}");
    }

    #[test]
    fn format_token_string_literal_shows_decoded_value() {
        let src = "\"hello\"";
        let out = compile("x.c", src);
        let line = format_token(src, &out.tokens[0]);
        assert!(line.contains("StringLiteral"), "{line}");
        assert!(line.contains("value: \"hello\""), "{line}");
        assert!(line.contains("prefix: None"), "{line}");
    }

    #[test]
    fn format_token_eof_has_empty_text_slice() {
        let out = compile("x.c", "");
        let eof = out.tokens.last().expect("tokenize always yields Eof");
        let line = format_token("", eof);
        assert_eq!(line, "Eof span=0..0 ''");
    }
}
