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

//! Compilation pipeline orchestration for the Forge compiler.
//!
//! The driver is the glue between the CLI and each individual compiler
//! phase.  It takes a source file and a [`CompileOptions`] bundle, runs
//! every phase in sequence, and returns a [`CompileOutput`] that carries
//! the artifacts produced by each completed phase alongside every
//! [`Diagnostic`] collected along the way.
//!
//! # Current state
//!
//! The lexer and the preprocessor are both wired in.  Subsequent phases
//! (parser, sema, IR, codegen, …) will be added here as they are
//! implemented; each phase appends its diagnostics to
//! [`CompileOutput::diagnostics`] and contributes its own artifact field
//! when appropriate.
//!
//! # Token-stream output
//!
//! For the `check` subcommand the CLI prints every token on its own line
//! in the format produced by [`format_token`]:
//!
//! ```text
//! Int span=0..3 'int'
//! Identifier("main") span=4..8 'main'
//! ```
//!
//! This shape is considered a **public contract** — it is the format
//! consumed by the lit-style test suite.

use std::fmt::Write as _;
use std::path::PathBuf;

pub use forge_diagnostics::{Diagnostic, Severity};
pub use forge_lexer::{Lexer, Token, TokenKind};
pub use forge_parser::printer::print_ast;
pub use forge_parser::{Parser, TranslationUnit};
pub use forge_preprocess::{
    detect_system_include_paths, spelling_of, PreprocessConfig, Preprocessor, TargetArch,
};

/// How far the driver should run the compilation pipeline before
/// returning control to the caller.
///
/// The CLI maps `-E` to [`CompileStage::Preprocess`] and every other
/// subcommand (`check`, `parse`, `build`) to [`CompileStage::Parse`].
/// The default is [`CompileStage::Parse`] — a library consumer that
/// takes the defaults gets the full front-end pipeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompileStage {
    /// Stop after the preprocessor.  Matches `forge -E`: callers that
    /// want the post-preprocessor token stream but deliberately *not*
    /// the parser's verdict on it (e.g. a raw token-dump probe on a
    /// source fragment that is not a complete translation unit).
    Preprocess,
    /// Lex + preprocess + parse.  The default — produces an AST plus
    /// the combined diagnostics of all three phases.
    #[default]
    Parse,
}

/// How the driver should drive the compilation pipeline.
///
/// Matches the CLI flag set: include search paths, command-line
/// `-D` / `-U` macro operations, the target architecture, and a
/// [`CompileStage`] cap that controls how far past the preprocessor
/// [`compile`] runs.  The CLI builds one of these from its parsed
/// arguments and hands it to [`compile`].
#[derive(Clone, Debug, Default)]
pub struct CompileOptions {
    /// Directories searched for `#include <...>` and (after the current
    /// file's directory) for `#include "..."`, in CLI order.  Prepended
    /// to the auto-detected system paths inside [`compile`].
    pub include_paths: Vec<PathBuf>,
    /// Every `-D` definition from the command line, in user order.
    pub defines: Vec<CliDefine>,
    /// Every `-U` undefine from the command line, in user order.
    pub undefines: Vec<String>,
    /// Target architecture the preprocessor should configure itself for.
    pub target_arch: TargetArch,
    /// Last pipeline phase to run.  Defaults to [`CompileStage::Parse`]
    /// (the full front-end); `-E` sets this to [`CompileStage::Preprocess`]
    /// so parser diagnostics do not fire on a preprocessor-only probe.
    pub stage: CompileStage,
}

/// One `-D NAME[=VALUE]` definition supplied on the command line.
///
/// `-D NAME` alone is represented as `ObjectLike { name, body: "1" }`,
/// matching the conventional compiler CLI behaviour.  A name containing
/// `(` is parsed as a function-like macro.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliDefine {
    /// `-D NAME`, `-D NAME=body`, or `-D NAME=` (empty body).
    ObjectLike {
        /// Macro name.
        name: String,
        /// Replacement body as a raw string — lexed by the preprocessor.
        body: String,
    },
    /// `-D 'NAME(p1, p2)=body'` or `-D 'NAME(p1, ...)=body'`.
    FunctionLike {
        /// Macro name.
        name: String,
        /// Named parameters, in declaration order.
        params: Vec<String>,
        /// `true` if the parameter list ended with `...`.
        is_variadic: bool,
        /// Replacement body as a raw string — lexed by the preprocessor.
        body: String,
    },
}

impl CliDefine {
    /// The macro name, regardless of variant.
    pub fn name(&self) -> &str {
        match self {
            CliDefine::ObjectLike { name, .. } | CliDefine::FunctionLike { name, .. } => name,
        }
    }
}

/// Parse one `-D` argument string into a [`CliDefine`].
///
/// Shapes accepted:
/// * `NAME` — object-like, body is `"1"`.
/// * `NAME=body` — object-like, body is the raw text after the first `=`.
///   An empty body (`NAME=`) is accepted and stored as-is.
/// * `NAME(p1, p2, ...)=body` — function-like.  An empty parameter list
///   `NAME()=body` is accepted.  The trailing `...` marks the macro as
///   variadic (the extra arguments are exposed as `__VA_ARGS__`).
///
/// Returns the textual error reason on a malformed input so the CLI can
/// surface it to the user.
pub fn parse_cli_define(input: &str) -> Result<CliDefine, String> {
    let (name_part, body) = match input.split_once('=') {
        Some((n, b)) => (n.to_string(), b.to_string()),
        None => (input.to_string(), "1".to_string()),
    };

    if let Some(paren_idx) = name_part.find('(') {
        let name = name_part[..paren_idx].trim().to_string();
        if !is_identifier(&name) {
            return Err(format!("invalid macro name: {name:?}"));
        }
        let param_list = &name_part[paren_idx..];
        if !param_list.ends_with(')') {
            return Err(format!(
                "function-like macro definition is missing a closing `)`: {input:?}"
            ));
        }
        let inside = &param_list[1..param_list.len() - 1];
        let mut params: Vec<String> = Vec::new();
        let mut is_variadic = false;
        if !inside.trim().is_empty() {
            for (i, raw) in inside.split(',').enumerate() {
                let p = raw.trim();
                if p == "..." {
                    is_variadic = true;
                    // `...` must be last — anything after is a parse error.
                    if i + 1 != inside.split(',').count() {
                        return Err(format!("`...` must be the last parameter in {input:?}"));
                    }
                } else if is_identifier(p) {
                    params.push(p.to_string());
                } else {
                    return Err(format!("invalid parameter {p:?} in {input:?}"));
                }
            }
        }
        Ok(CliDefine::FunctionLike {
            name,
            params,
            is_variadic,
            body,
        })
    } else {
        let name = name_part.trim().to_string();
        if !is_identifier(&name) {
            return Err(format!("invalid macro name: {name:?}"));
        }
        Ok(CliDefine::ObjectLike { name, body })
    }
}

/// Whether `s` is a syntactically valid C identifier.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The aggregate result of running the compilation pipeline on a source file.
///
/// Contains both the artifacts produced by every completed phase and every
/// [`Diagnostic`] (error, warning, note) emitted along the way.  The CLI
/// renders diagnostics unconditionally and exits non-zero iff
/// [`CompileOutput::has_errors`] returns `true`.
#[derive(Debug, Clone)]
pub struct CompileOutput {
    /// The full post-preprocessor token stream, terminated by
    /// [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// The parsed AST, if the parser phase ran.  Always present after a
    /// successful [`compile`] call (parsing is unconditional and always
    /// yields a possibly-partial tree even on error).
    pub ast: Option<TranslationUnit>,
    /// Diagnostics collected from every pipeline phase, in emission order.
    pub diagnostics: Vec<Diagnostic>,
    /// The source text actually fed to the lexer — the user's original
    /// source with any CLI-synthesised prelude prepended.  Diagnostic
    /// rendering uses this so spans stay valid.
    pub effective_source: String,
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

/// Run the compilation pipeline on the given source text.
///
/// The pipeline currently covers lexing, preprocessing, and — when
/// [`CompileOptions::stage`] is [`CompileStage::Parse`] (the default) —
/// parsing.  `options` carries every CLI-driven knob: include paths,
/// `-D`/`-U` macro operations, the target architecture, and the stage
/// cap.
///
/// Callers that want only the post-preprocessor token stream (e.g.
/// `forge -E`) pass [`CompileStage::Preprocess`] and get back an
/// [`CompileOutput`] with `ast = None` and no parser diagnostics.  All
/// other callers receive a (possibly partial) AST — the parser yields
/// one on every input, valid or not.
///
/// `filename` is used for `__FILE__` expansions, for `#include "..."`
/// relative resolution, and for the `#line` reset that makes user
/// diagnostics report against the original source even when a synthetic
/// CLI prelude was prepended.
pub fn compile(filename: &str, source: &str, options: &CompileOptions) -> CompileOutput {
    // ---- Build include search path: user -I first, then auto-detected.
    let mut include_paths = options.include_paths.clone();
    include_paths.extend(detect_system_include_paths());

    // ---- Synthesise the CLI prelude.
    //
    // Every -D and -U operation is threaded through the source as synthetic
    // `#define` / `#undef` lines.  Going through the ordinary directive
    // dispatch keeps the semantics uniform: a function-like `-D` goes through
    // the same parser as an in-source `#define`, and the per-name `#undef`
    // that precedes each `#define` suppresses the "redefinition with
    // different replacement" warning when a CLI macro overrides a built-in.
    //
    // A trailing `#line 1 "<filename>"` resets the preprocessor's line
    // counter so diagnostics on the user's source report against the
    // user's original line numbers.
    let prelude = build_cli_prelude(filename, &options.defines, &options.undefines);
    let effective_source = if prelude.is_empty() {
        source.to_string()
    } else {
        format!("{prelude}{source}")
    };

    // ---- Lex.
    let mut lexer = Lexer::new(&effective_source);
    let tokens = lexer.tokenize();
    let mut diagnostics = lexer.take_diagnostics();

    // ---- Preprocess.
    let config = PreprocessConfig {
        include_paths,
        target_arch: options.target_arch,
        predefined_macros: Vec::new(),
        ..PreprocessConfig::default()
    };
    let mut pp = Preprocessor::new(config);
    // Stamp the root include frame with the on-disk file path whenever
    // one is available, so `#include "..."` inside the source can
    // resolve relative to the including file's directory.  An in-memory
    // `filename` that does not correspond to a real file (e.g.
    // `"<stdin>"`) falls back to the path-less `run_with_source`.
    let root_path = PathBuf::from(filename);
    let pp_tokens = if root_path.is_file() {
        let canonical = std::fs::canonicalize(&root_path).unwrap_or(root_path);
        pp.run_with_source_at(tokens, &effective_source, filename, canonical)
    } else {
        pp.run_with_source(tokens, &effective_source, filename)
    };
    diagnostics.extend(pp.take_diagnostics());

    // ---- Parse (optional — skipped for `forge -E`).
    //
    // The parser consumes its token vector, so we clone the post-pp
    // stream before handing it off.  The clone is paid for only when
    // the caller asked for a parse; `-E` and other preprocess-only
    // consumers keep the zero-copy path.
    let ast = match options.stage {
        CompileStage::Preprocess => None,
        CompileStage::Parse => {
            let (tu, parser_diagnostics) = Parser::parse(pp_tokens.clone());
            diagnostics.extend(parser_diagnostics);
            Some(tu)
        }
    };

    CompileOutput {
        tokens: pp_tokens,
        ast,
        diagnostics,
        effective_source,
    }
}

/// Run the parser on a post-preprocessor token stream.
///
/// Returns `(translation_unit, parser_diagnostics)` — the parser always
/// yields a (possibly partial) AST plus its own diagnostics, even on
/// syntactic errors.
///
/// [`compile`] invokes this internally when
/// [`CompileOptions::stage`] is [`CompileStage::Parse`].  The free
/// function is kept exported for callers that already have a token
/// stream in hand (e.g. tests that build one manually) and want to run
/// the parser without re-lexing.
pub fn parse_tokens(tokens: Vec<Token>) -> (TranslationUnit, Vec<Diagnostic>) {
    Parser::parse(tokens)
}

/// Build the synthetic preamble the driver prepends to the user's source.
///
/// Exposed for test inspection and separated from [`compile`] for
/// readability.  The format is a sequence of `#undef` / `#define` lines
/// in CLI order (all `-D`s first, then all `-U`s — matching the order
/// the phase specification prescribes), finished with a `#line 1 "<file>"`
/// reset so user-source line numbers are preserved.
///
/// Returns an empty string when there is nothing to do, so the common
/// zero-flag path allocates nothing extra.
pub fn build_cli_prelude(filename: &str, defines: &[CliDefine], undefines: &[String]) -> String {
    if defines.is_empty() && undefines.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for def in defines {
        match def {
            CliDefine::ObjectLike { name, body } => {
                let _ = writeln!(out, "#undef {name}");
                let body = if body.is_empty() { "1" } else { body.as_str() };
                let _ = writeln!(out, "#define {name} {body}");
            }
            CliDefine::FunctionLike {
                name,
                params,
                is_variadic,
                body,
            } => {
                let _ = writeln!(out, "#undef {name}");
                let mut param_list = params.join(", ");
                if *is_variadic {
                    if !param_list.is_empty() {
                        param_list.push_str(", ");
                    }
                    param_list.push_str("...");
                }
                let body = if body.is_empty() { "" } else { body.as_str() };
                let _ = writeln!(out, "#define {name}({param_list}) {body}");
            }
        }
    }
    for name in undefines {
        let _ = writeln!(out, "#undef {name}");
    }
    // Escape backslashes and quotes for the `#line` filename literal so
    // Windows paths (and the rare `"` in a filename) stay well-formed.
    let escaped = filename.replace('\\', "\\\\").replace('"', "\\\"");
    let _ = writeln!(out, "#line 1 \"{escaped}\"");
    out
}

/// Reconstruct a readable C source text from a post-preprocessor token
/// stream.  This is what `forge -E` writes to stdout — similar in intent
/// (and usually in shape) to what `gcc -E` / `clang -E` produce, minus
/// the `# <line> "<file>"` linemarker comments the Forge preprocessor
/// does not yet emit.
///
/// # Rules
///
/// * Every non-[`TokenKind::Eof`] token is printed via [`spelling_of`].
/// * Between two tokens on the same line, a single space is emitted iff
///   the later token has [`Token::has_leading_space`] set.
/// * A newline is emitted before any token with
///   [`Token::at_start_of_line`] set (except the very first token).
/// * A single trailing newline is appended so terminals don't render the
///   final prompt on the last line of output.
///
/// The output is syntactically valid C: every pair of tokens the
/// preprocessor left adjacent without a space in the source was already
/// lex-unambiguous (the lexer would have merged them otherwise), so
/// preserving adjacency cannot accidentally re-merge distinct tokens.
pub fn tokens_to_source(tokens: &[Token]) -> String {
    let mut out = String::new();
    let mut first = true;
    for tok in tokens {
        if matches!(tok.kind, TokenKind::Eof) {
            continue;
        }
        if first {
            first = false;
        } else if tok.at_start_of_line {
            out.push('\n');
        } else if tok.has_leading_space {
            out.push(' ');
        }
        out.push_str(&spelling_of(&tok.kind));
    }
    if !first {
        out.push('\n');
    }
    out
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

    fn opts() -> CompileOptions {
        CompileOptions::default()
    }

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
        let out = compile("empty.c", "", &opts());
        assert!(out.diagnostics.is_empty());
        assert!(!out.has_errors());
        // An empty input still yields a single Eof token.
        assert_eq!(out.tokens.len(), 1);
        assert!(matches!(out.tokens[0].kind, TokenKind::Eof));
    }

    #[test]
    fn compile_simple_source_has_no_errors() {
        let src = "int main(void) { return 0; }";
        let out = compile("main.c", src, &opts());
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
        let out = compile("bad.c", "0x", &opts());
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
        // The literal is wrapped in a well-formed declaration so the
        // parser phase does not add spurious error-severity diagnostics.
        let out = compile("warn.c", "int x = 99999999999999999999999999;\n", &opts());
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

    #[test]
    fn compile_expands_object_like_macro_from_source() {
        let src = "#define N 42\nint x = N;\n";
        let out = compile("x.c", src, &opts());
        assert!(!out.has_errors());
        let kinds = kind_strings(&out);
        // After the preprocessor runs the `#define` is gone and `N` has
        // been replaced by `42`.
        assert!(!kinds.iter().any(|k| k.contains("Hash")));
        assert!(
            kinds.iter().any(|k| k.contains("value: 42")),
            "expected IntegerLiteral(42) in {kinds:?}"
        );
    }

    // ---------- CliDefine parsing ----------

    #[test]
    fn parse_cli_define_bare_name_defaults_to_one() {
        let d = parse_cli_define("FOO").unwrap();
        assert_eq!(
            d,
            CliDefine::ObjectLike {
                name: "FOO".into(),
                body: "1".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_explicit_body() {
        let d = parse_cli_define("FOO=bar").unwrap();
        assert_eq!(
            d,
            CliDefine::ObjectLike {
                name: "FOO".into(),
                body: "bar".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_empty_body_is_preserved_verbatim() {
        let d = parse_cli_define("FOO=").unwrap();
        assert_eq!(
            d,
            CliDefine::ObjectLike {
                name: "FOO".into(),
                body: String::new(),
            }
        );
    }

    #[test]
    fn parse_cli_define_body_may_contain_equals() {
        // Only the *first* `=` separates name from body, so complex bodies
        // like `FOO=x==y` survive verbatim.
        let d = parse_cli_define("FOO=x==y").unwrap();
        assert_eq!(
            d,
            CliDefine::ObjectLike {
                name: "FOO".into(),
                body: "x==y".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_function_like_with_two_params() {
        let d = parse_cli_define("ADD(a, b)=((a)+(b))").unwrap();
        assert_eq!(
            d,
            CliDefine::FunctionLike {
                name: "ADD".into(),
                params: vec!["a".into(), "b".into()],
                is_variadic: false,
                body: "((a)+(b))".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_function_like_variadic() {
        let d = parse_cli_define("LOG(fmt, ...)=printf(fmt, __VA_ARGS__)").unwrap();
        assert_eq!(
            d,
            CliDefine::FunctionLike {
                name: "LOG".into(),
                params: vec!["fmt".into()],
                is_variadic: true,
                body: "printf(fmt, __VA_ARGS__)".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_function_like_empty_parameter_list() {
        let d = parse_cli_define("GREET()=puts(\"hi\")").unwrap();
        assert_eq!(
            d,
            CliDefine::FunctionLike {
                name: "GREET".into(),
                params: Vec::new(),
                is_variadic: false,
                body: "puts(\"hi\")".into(),
            }
        );
    }

    #[test]
    fn parse_cli_define_rejects_unclosed_paren() {
        assert!(parse_cli_define("BAD(x=1").is_err());
    }

    #[test]
    fn parse_cli_define_rejects_bad_name() {
        assert!(parse_cli_define("1FOO=1").is_err());
        assert!(parse_cli_define("=body").is_err());
    }

    // ---------- -D / -U through the pipeline ----------

    #[test]
    fn cli_define_overrides_object_like_macro() {
        let mut options = opts();
        options
            .defines
            .push(parse_cli_define("CUSTOM=777").expect("parses"));
        let out = compile("x.c", "int x = CUSTOM;\n", &options);
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        let kinds = kind_strings(&out);
        assert!(
            kinds.iter().any(|k| k.contains("value: 777")),
            "expected IntegerLiteral(777) in {kinds:?}"
        );
    }

    #[test]
    fn cli_define_function_like_expands_at_invocation() {
        let mut options = opts();
        options
            .defines
            .push(parse_cli_define("SQR(x)=((x)*(x))").expect("parses"));
        let out = compile("x.c", "int y = SQR(7);\n", &options);
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        let src = tokens_to_source(&out.tokens);
        assert!(
            src.contains("((7)*(7))"),
            "expected expansion of SQR(7) in {src:?}"
        );
    }

    #[test]
    fn cli_undefine_removes_a_previous_cli_define() {
        let mut options = opts();
        options
            .defines
            .push(parse_cli_define("FOO=1").expect("parses"));
        options.undefines.push("FOO".into());
        let out = compile(
            "x.c",
            "#ifdef FOO\nint defined;\n#else\nint undefined;\n#endif\n",
            &options,
        );
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        let kinds = kind_strings(&out);
        assert!(
            kinds.iter().any(|k| k.contains("undefined")),
            "FOO should be undefined after -U: {kinds:?}"
        );
        assert!(
            !kinds.iter().any(|k| k.contains("\"defined\"")),
            "FOO should not be defined after -U: {kinds:?}"
        );
    }

    #[test]
    fn cli_define_preserves_user_line_numbers() {
        // With a synthetic prelude prepended, `__LINE__` at the top of the
        // user file must still report as line 1 of the user's source.
        let mut options = opts();
        options
            .defines
            .push(parse_cli_define("VAL=1").expect("parses"));
        let out = compile("x.c", "int line = __LINE__;\n", &options);
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        let kinds = kind_strings(&out);
        assert!(
            kinds.iter().any(|k| k.contains("value: 1")),
            "__LINE__ should be 1 after prelude reset: {kinds:?}"
        );
    }

    // ---------- Include paths ----------

    #[test]
    fn include_paths_from_options_are_searched_first() {
        // Create a temp directory with a header, pass its path via
        // `include_paths`, and `#include <header>` should succeed.
        let dir = std::env::temp_dir().join(format!("forge_driver_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let header = dir.join("forge_test_header.h");
        std::fs::write(&header, "#define FROM_HEADER 314\n").unwrap();

        let mut options = opts();
        options.include_paths.push(dir.clone());

        let out = compile(
            "x.c",
            "#include <forge_test_header.h>\nint v = FROM_HEADER;\n",
            &options,
        );
        let kinds = kind_strings(&out);
        let _ = std::fs::remove_file(&header);
        let _ = std::fs::remove_dir(&dir);
        assert!(!out.has_errors(), "diagnostics: {:?}", out.diagnostics);
        assert!(
            kinds.iter().any(|k| k.contains("value: 314")),
            "header macro should have been found in {kinds:?}"
        );
    }

    // ---------- tokens_to_source ----------

    #[test]
    fn tokens_to_source_roundtrips_simple_decl() {
        let src = "int x = 42;\n";
        let out = compile("x.c", src, &opts());
        let rebuilt = tokens_to_source(&out.tokens);
        assert!(rebuilt.contains("int x = 42"), "got: {rebuilt:?}");
        assert!(rebuilt.ends_with('\n'));
    }

    #[test]
    fn tokens_to_source_preserves_line_breaks_between_statements() {
        let src = "int a;\nint b;\n";
        let out = compile("x.c", src, &opts());
        let rebuilt = tokens_to_source(&out.tokens);
        // Two statements on separate lines — a newline must appear
        // between them in the rebuilt text.
        let mut lines = rebuilt.lines();
        assert_eq!(lines.next(), Some("int a;"));
        assert_eq!(lines.next(), Some("int b;"));
    }

    #[test]
    fn tokens_to_source_skips_eof() {
        let out = compile("x.c", "", &opts());
        let rebuilt = tokens_to_source(&out.tokens);
        assert!(rebuilt.is_empty(), "expected empty output, got {rebuilt:?}");
    }

    #[test]
    fn tokens_to_source_expands_object_macro() {
        let src = "#define N 99\nint v = N;\n";
        let out = compile("x.c", src, &opts());
        let rebuilt = tokens_to_source(&out.tokens);
        assert!(rebuilt.contains("99"), "got: {rebuilt:?}");
        assert!(!rebuilt.contains('#'), "directives must not leak");
        assert!(!rebuilt.contains(" N "), "macro name must not leak");
    }

    #[test]
    fn tokens_to_source_expands_function_macro() {
        let src = "#define MAX(a, b) ((a) > (b) ? (a) : (b))\nint x = MAX(3, 5);\n";
        let out = compile("x.c", src, &opts());
        let rebuilt = tokens_to_source(&out.tokens);
        assert!(
            rebuilt.contains("((3) > (5) ? (3) : (5))"),
            "got: {rebuilt:?}"
        );
    }

    // ---------- format_token() shape ----------

    #[test]
    fn format_token_keyword() {
        let src = "int";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert_eq!(line, "Int span=0..3 'int'");
    }

    #[test]
    fn format_token_punctuator() {
        let src = "+=";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert_eq!(line, "PlusEqual span=0..2 '+='");
    }

    #[test]
    fn format_token_identifier_includes_name_and_source_slice() {
        let src = "foo";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert!(line.starts_with("Identifier(\"foo\")"), "{line}");
        assert!(line.ends_with("span=0..3 'foo'"), "{line}");
    }

    #[test]
    fn format_token_integer_literal_shows_value_and_suffix() {
        let src = "42u";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert!(line.contains("IntegerLiteral"), "{line}");
        assert!(line.contains("value: 42"), "{line}");
        assert!(line.contains("suffix: U"), "{line}");
        assert!(line.ends_with("'42u'"), "{line}");
    }

    #[test]
    fn format_token_float_literal_shows_value_and_suffix() {
        let src = "1.5f";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert!(line.contains("FloatLiteral"), "{line}");
        assert!(line.contains("value: 1.5"), "{line}");
        assert!(line.contains("suffix: F"), "{line}");
    }

    #[test]
    fn format_token_char_literal_shows_value_and_prefix() {
        let src = "'A'";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert!(line.contains("CharLiteral"), "{line}");
        assert!(line.contains("value: 65"), "{line}");
        assert!(line.contains("prefix: None"), "{line}");
    }

    #[test]
    fn format_token_string_literal_shows_decoded_value() {
        let src = "\"hello\"";
        let out = compile("x.c", src, &opts());
        let line = format_token(&out.effective_source, &out.tokens[0]);
        assert!(line.contains("StringLiteral"), "{line}");
        assert!(line.contains("value: \"hello\""), "{line}");
        assert!(line.contains("prefix: None"), "{line}");
    }

    #[test]
    fn format_token_eof_has_empty_text_slice() {
        let out = compile("x.c", "", &opts());
        let eof = out.tokens.last().expect("tokenize always yields Eof");
        let line = format_token(&out.effective_source, eof);
        assert_eq!(line, "Eof span=0..0 ''");
    }

    // ---------- build_cli_prelude ----------

    #[test]
    fn build_cli_prelude_empty_for_no_flags() {
        let p = build_cli_prelude("x.c", &[], &[]);
        assert!(p.is_empty());
    }

    #[test]
    fn build_cli_prelude_emits_line_reset_at_end() {
        let defs = vec![CliDefine::ObjectLike {
            name: "A".into(),
            body: "1".into(),
        }];
        let p = build_cli_prelude("x.c", &defs, &[]);
        assert!(
            p.ends_with("#line 1 \"x.c\"\n"),
            "expected trailing #line reset, got {p:?}"
        );
    }

    #[test]
    fn build_cli_prelude_escapes_quotes_and_backslashes_in_filename() {
        let defs = vec![CliDefine::ObjectLike {
            name: "A".into(),
            body: "1".into(),
        }];
        let p = build_cli_prelude(r#"C:\tmp\a "b".c"#, &defs, &[]);
        assert!(p.contains(r#"#line 1 "C:\\tmp\\a \"b\".c""#), "got: {p:?}");
    }

    #[test]
    fn build_cli_prelude_renders_function_like_macros() {
        let defs = vec![CliDefine::FunctionLike {
            name: "F".into(),
            params: vec!["x".into()],
            is_variadic: true,
            body: "(x)".into(),
        }];
        let p = build_cli_prelude("x.c", &defs, &[]);
        assert!(
            p.contains("#undef F\n#define F(x, ...) (x)\n"),
            "got: {p:?}"
        );
    }

    #[test]
    fn build_cli_prelude_renders_undefines_after_defines() {
        let defs = vec![CliDefine::ObjectLike {
            name: "A".into(),
            body: "1".into(),
        }];
        let unds = vec!["B".to_string()];
        let p = build_cli_prelude("x.c", &defs, &unds);
        let def_pos = p.find("#define A").expect("define rendered");
        let und_pos = p.find("#undef B\n#line").expect("undef rendered");
        assert!(def_pos < und_pos, "defines must precede undefines: {p:?}");
    }
}
