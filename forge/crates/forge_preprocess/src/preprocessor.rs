//! The core preprocessor driver: main loop, directive dispatch, and the
//! `#define` / `#undef` handlers, plus object-like macro expansion.
//!
//! This module does four things:
//!
//! 1. Walk the incoming token stream and classify each token as either a
//!    directive, a macro invocation, or an emittable token (see
//!    [`Preprocessor::run`]).
//! 2. Dispatch directives by name to a handler.  `#define` and `#undef`
//!    are fully implemented; the rest are recognised and their argument
//!    lines are consumed, but they record an informational
//!    [`Diagnostic`] saying the handler is not yet wired up.
//! 3. Expand **object-like** macros using the C17 §6.10.3.4 hide-set
//!    algorithm: when a macro `M` is expanded, every token of its
//!    replacement list is tagged with `invocation.hide_set ∪ { M }`, and
//!    that replacement is spliced back into the cursor for rescanning.
//!    A later rescan that would re-enter `M` short-circuits because `M`
//!    is in the token's hide set.
//! 4. Emit a diagnostic when a `#define` redefines an existing name with
//!    a different replacement list, per C17 §6.10.3/2.
//!
//! Function-like macro expansion, conditional compilation, `#include`,
//! and the remaining directives will land in subsequent prompts.  The
//! types they will need — [`IfState`], [`IncludeFrame`],
//! [`TokenCursor::push_front`] — are already in place so those additions
//! do not need to reshape this module.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use forge_diagnostics::{Diagnostic, Severity};
use forge_lexer::{lex_fragment, IntSuffix, Lexer, Span, StringPrefix, Token, TokenKind};

use crate::cond_expr::{self, PPValue};
use crate::cursor::TokenCursor;
use crate::expand::{paste_spelling, spelling_of, stringify};
use crate::pp_token::{unwrap_tokens, wrap_tokens, PPToken};
use crate::state::{
    macros_equivalent, IfState, IncludeFrame, MacroDef, PreprocessConfig, TargetArch,
};

/// Display name used for `__FILE__` when the preprocessor is handed raw
/// tokens via [`Preprocessor::run`] without a filename.
const DEFAULT_INPUT_NAME: &str = "<input>";

/// Identifier the preprocessor recognises inside a variadic macro as the
/// placeholder for the `...` argument list.
const VA_ARGS: &str = "__VA_ARGS__";

/// The main preprocessor state.
///
/// Build one via [`Preprocessor::new`], hand it the lexer's token stream
/// through [`Preprocessor::run`], and then collect any accumulated
/// [`Diagnostic`]s via [`Preprocessor::take_diagnostics`].
///
/// For most callers the stand-alone [`preprocess`] function is more
/// convenient — use [`Preprocessor`] directly when you need fine-grained
/// control, for example to drive multiple translation units that share a
/// warning counter.
pub struct Preprocessor {
    /// The macro table, keyed by macro name.
    macros: HashMap<String, MacroDef>,
    /// Directories searched for `#include <...>` headers, in order.  For
    /// quote includes the *current file's* directory is searched first
    /// and these paths act as the fallback.
    include_paths: Vec<PathBuf>,
    /// Stack of currently-open include frames.  The top of the stack is
    /// the file the preprocessor is currently emitting tokens from; the
    /// bottom is the translation unit passed to [`Preprocessor::run`] /
    /// [`Preprocessor::run_file`].
    include_stack: Vec<IncludeFrame>,
    /// Set of canonical paths that have been seen with `#pragma once`.
    /// Any future `#include` of one of these is silently skipped.
    pragma_once_files: HashSet<PathBuf>,
    /// Maximum allowed include depth before the preprocessor gives up.
    max_include_depth: u32,
    /// Stack of open `#if`/`#ifdef`/`#ifndef` blocks.  A block is
    /// emitting tokens iff every frame has `current_branch_active`.
    if_stack: Vec<IfState>,
    /// Target architecture — drives arch-specific predefined macros.
    target_arch: TargetArch,
    /// Display filename for the *currently active* source — what
    /// `__FILE__` expands to at this moment.  Changes across
    /// `#include` boundaries and is restored on the way out.
    current_file: String,
    /// Byte offsets of every newline in the currently active source,
    /// plus an implicit `0` at the start.  Used to turn the byte
    /// position of a `__LINE__` invocation into a 1-based line number
    /// via a single binary search.
    line_starts: Vec<u32>,
    /// Effective line-number translation for the currently active
    /// source, installed by `#line N`.  `Some((anchor_actual,
    /// anchor_reported))` means: any token whose actual 1-based line
    /// number `L` is `>= anchor_actual` reports line `anchor_reported
    /// + (L - anchor_actual)`.  `None` means no translation is in
    /// force — `__LINE__` expands to the actual physical line.
    ///
    /// The field is saved / restored across `#include` boundaries so a
    /// `#line` inside a header does not leak into the including file.
    line_offset: Option<(u32, u32)>,
    /// Effective filename override for the currently active source,
    /// installed by `#line N "filename"`.  `Some(name)` forces
    /// `__FILE__` to expand to `name` until either a new `#line`
    /// changes it or the enclosing include frame is popped.
    file_override: Option<String>,
    /// Accumulating output buffer.  Top-level [`Preprocessor::run`]
    /// resets it at the start of each run; `#include` handling
    /// appends into it across recursive `drive` calls.
    output: Vec<PPToken>,
    /// Diagnostics collected during preprocessing.
    diagnostics: Vec<Diagnostic>,
    /// `true` once a `#error` directive has fired.  Unlike the diagnostic
    /// vector, this flag survives a [`Self::take_diagnostics`] call so
    /// the stand-alone [`preprocess`] entry point can report failure
    /// even if the caller drains warnings mid-run.
    has_errors: bool,
}

impl Preprocessor {
    /// Build a fresh preprocessor from a [`PreprocessConfig`].
    ///
    /// The C17 standard predefined macros (`__STDC__`, `__STDC_VERSION__`,
    /// `__STDC_HOSTED__`), the architecture- and platform-specific
    /// macros, the GCC-compatibility macros (`__GNUC__`, …), the build
    /// date / time (`__DATE__`, `__TIME__`), and every user-supplied
    /// `-D` definition from the config are installed up-front.  The
    /// magic macros `__FILE__` and `__LINE__` are registered as well so
    /// `#undef __LINE__` behaves like a real C compiler; they are
    /// **expanded specially** rather than from their (empty) replacement
    /// list.
    pub fn new(config: PreprocessConfig) -> Self {
        let PreprocessConfig {
            include_paths,
            target_arch,
            predefined_macros,
            max_include_depth,
        } = config;
        let mut pp = Self {
            macros: HashMap::new(),
            include_paths,
            include_stack: Vec::new(),
            pragma_once_files: HashSet::new(),
            max_include_depth,
            if_stack: Vec::new(),
            target_arch,
            current_file: DEFAULT_INPUT_NAME.to_string(),
            line_starts: vec![0],
            line_offset: None,
            file_override: None,
            output: Vec::new(),
            diagnostics: Vec::new(),
            has_errors: false,
        };
        pp.install_predefined_macros(&predefined_macros);
        pp
    }

    /// Immutable view of the macro table.  Primarily useful in tests and
    /// for debug dumps.
    pub fn macros(&self) -> &HashMap<String, MacroDef> {
        &self.macros
    }

    /// Immutable view of the configured include search paths.
    pub fn include_paths(&self) -> &[PathBuf] {
        &self.include_paths
    }

    /// The current include stack (empty until `#include` lands).
    pub fn include_stack(&self) -> &[IncludeFrame] {
        &self.include_stack
    }

    /// The currently-open conditional blocks, outermost first.
    pub fn if_stack(&self) -> &[IfState] {
        &self.if_stack
    }

    /// The configured [`TargetArch`].
    pub fn target_arch(&self) -> TargetArch {
        self.target_arch
    }

    /// Drain and return every diagnostic recorded so far, leaving the
    /// internal buffer empty.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// `true` iff a `#error` directive has fired at any point during
    /// this preprocessor's lifetime.  Unlike the diagnostics vector,
    /// this flag is *not* reset by [`Self::take_diagnostics`] — it is
    /// the durable record of "preprocessing failed" that the high-level
    /// [`preprocess`] entry point consults when deciding whether to
    /// return `Err`.
    pub fn has_errors(&self) -> bool {
        self.has_errors
    }

    /// Run the preprocessor over `tokens` and return the output token
    /// stream.  The input is consumed by value because the preprocessor
    /// pushes tokens around as part of macro-expansion rescans.
    ///
    /// The returned stream preserves the lexer's trailing
    /// [`TokenKind::Eof`] so downstream phases can rely on that
    /// invariant.
    ///
    /// `__FILE__` expands to `"<input>"` and `__LINE__` to `1` for every
    /// token, because this overload does not carry source text.  Callers
    /// that want meaningful line/file information should use
    /// [`Preprocessor::run_with_source`] or [`Preprocessor::run_file`].
    pub fn run(&mut self, tokens: Vec<Token>) -> Vec<Token> {
        self.run_impl(tokens, "", DEFAULT_INPUT_NAME, None)
    }

    /// Preprocess `tokens` that were lexed from `source`.
    ///
    /// `filename` is used for `__FILE__` expansions and for diagnostic
    /// labels; `source` is consulted to compute line numbers for
    /// `__LINE__`.  Both are purely informational — the tokens
    /// themselves carry every byte offset the preprocessor needs.
    pub fn run_with_source(
        &mut self,
        tokens: Vec<Token>,
        source: &str,
        filename: &str,
    ) -> Vec<Token> {
        self.run_impl(tokens, source, filename, None)
    }

    /// Preprocess `tokens` as if they had been lexed from the file at
    /// `root_path`.
    ///
    /// Identical to [`run_with_source`](Self::run_with_source) except
    /// that the root include frame is stamped with `root_path`, so
    /// `#include "..."` directives inside `tokens` resolve relative to
    /// its parent directory — matching what `run_file` does but for
    /// callers that have already read (and possibly transformed) the
    /// source text.
    pub fn run_with_source_at(
        &mut self,
        tokens: Vec<Token>,
        source: &str,
        filename: &str,
        root_path: PathBuf,
    ) -> Vec<Token> {
        self.run_impl(tokens, source, filename, Some(root_path))
    }

    /// Read `path` from disk, lex it, and preprocess the result.
    ///
    /// The returned stream includes the file's contents with every
    /// directive consumed and every macro fully expanded.  On an I/O
    /// failure the error is returned immediately — no partial output
    /// token list is produced, and the diagnostics buffer is left
    /// untouched so the caller can call again after fixing the issue.
    pub fn run_file(&mut self, path: &Path) -> std::io::Result<Vec<Token>> {
        let source = std::fs::read_to_string(path)?;
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.display().to_string();
        let mut lexer = Lexer::new(&source);
        let tokens = lexer.tokenize();
        for d in lexer.take_diagnostics() {
            self.diagnostics.push(d);
        }
        Ok(self.run_impl(tokens, &source, &filename, Some(canonical)))
    }

    /// Private implementation shared by [`run`](Self::run),
    /// [`run_with_source`](Self::run_with_source), and
    /// [`run_file`](Self::run_file).
    ///
    /// `root_path`, when present, is pushed onto the include stack as
    /// the bottom frame so the first `#include "..."` can be resolved
    /// relative to the source file's directory.
    fn run_impl(
        &mut self,
        tokens: Vec<Token>,
        source: &str,
        filename: &str,
        root_path: Option<PathBuf>,
    ) -> Vec<Token> {
        // Pull the trailing EOF off the end so we can re-emit it after
        // `drive()` finishes — `drive()` itself swallows EOF silently
        // because nested include streams do not terminate the output.
        let mut tokens = tokens;
        let eof = match tokens.last() {
            Some(t) if matches!(t.kind, TokenKind::Eof) => tokens.pop(),
            _ => None,
        };

        self.output.clear();
        self.current_file = filename.to_string();
        self.line_starts = compute_line_starts(source);
        let root_frame = match root_path {
            Some(p) => IncludeFrame::file(p, 0),
            None => IncludeFrame::new(filename, 0),
        };
        self.include_stack.push(root_frame);

        let mut cursor = TokenCursor::new(wrap_tokens(tokens));
        self.drive(&mut cursor);
        self.flush_unterminated_ifs();

        self.include_stack.pop();

        // Re-emit the EOF sentinel so downstream phases still see a
        // terminated stream.
        let eof = eof.unwrap_or(Token {
            kind: TokenKind::Eof,
            span: Span::new(0, 0),
            at_start_of_line: true,
            has_leading_space: false,
        });
        self.output.push(PPToken::new(eof));

        unwrap_tokens(std::mem::take(&mut self.output))
    }

    /// The inner drive loop — consume `cursor` until it is exhausted
    /// (or hits a sentinel `Eof`), appending every emitted token to
    /// [`Preprocessor::output`].  Used both at the top level and
    /// recursively by `#include`.
    fn drive(&mut self, cursor: &mut TokenCursor) {
        while let Some(tok_ref) = cursor.peek() {
            let is_hash_directive =
                matches!(tok_ref.kind(), TokenKind::Hash) && tok_ref.at_start_of_line();
            let is_eof = matches!(tok_ref.kind(), TokenKind::Eof);

            if is_eof {
                // Swallow the stream's EOF — the top-level `run_impl`
                // is responsible for emitting a single EOF on the way
                // out.
                cursor.advance();
                break;
            }

            if is_hash_directive {
                let Some(hash) = cursor.advance() else { break };
                self.handle_directive(&hash.token, cursor);
            } else {
                if self.is_active() && self.try_expand(cursor) {
                    continue;
                }
                let Some(tok) = cursor.advance() else { break };
                if self.is_active() {
                    self.output.push(tok);
                }
            }
        }
    }

    // -----------------------------------------------------------------
    // Macro expansion
    // -----------------------------------------------------------------

    /// If the next token is an identifier that names a macro and is not
    /// blocked by its own hide set, consume it and splice the expansion
    /// back into the cursor for rescanning.  Returns `true` iff an
    /// expansion was performed; on `false` the cursor is left untouched.
    ///
    /// Hide-set discipline (C17 §6.10.3.4):
    ///
    /// * The invocation is only expanded when its token's `hide_set` does
    ///   **not** contain the macro name.
    /// * Every replacement token carries
    ///   `invocation.hide_set ∪ { macro_name }`, which protects it from
    ///   being re-entered on a subsequent rescan.
    /// * The first replacement token inherits the invocation's
    ///   `at_start_of_line` / `has_leading_space` flags so a macro used
    ///   at the start of a line still acts like a start-of-line token in
    ///   its new context.
    ///
    /// Function-like macros additionally require a `(` to immediately
    /// follow the name token (whitespace between the two is allowed).
    /// A bare function-like name with no following `(` passes through as
    /// an ordinary identifier.
    fn try_expand(&mut self, cursor: &mut TokenCursor) -> bool {
        // Decide whether the next token is an expandable macro name.
        // Extract an owned `name` + a flag so the immutable borrow of
        // `cursor` / `self.macros` can end before we mutate either.
        let (name, is_fn_like) = {
            let tok = match cursor.peek() {
                Some(t) => t,
                None => return false,
            };
            let name = match &tok.token.kind {
                TokenKind::Identifier(s) => s,
                _ => return false,
            };
            // `_Pragma` is an operator (C17 §6.10.9), not a macro.
            // Intercept it here so an expansion-produced `_Pragma(...)`
            // is processed identically to a source-written one.  The
            // hide set is not consulted — `_Pragma` is a keyword-like
            // construct, not a macro.
            if name == "_Pragma" {
                if let Some(next) = cursor.peek_nth(1) {
                    if matches!(next.kind(), TokenKind::LeftParen) {
                        return self.try_handle_pragma_operator(cursor);
                    }
                }
            }
            if tok.hide_set.contains(name) {
                return false;
            }
            // Magic macros (`__FILE__`, `__LINE__`) are intercepted
            // before the macro table: their "replacement" is computed
            // on the fly from the preprocessor's current state.  Only
            // fire when the magic name is still defined — `#undef
            // __FILE__` removes it and the name then falls through to
            // the usual identifier path.
            if is_magic_name(name) && self.macros.contains_key(name) {
                return self.expand_magic_macro(cursor);
            }
            match self.macros.get(name) {
                Some(MacroDef::ObjectLike { .. }) => (name.clone(), false),
                Some(MacroDef::FunctionLike { .. }) => {
                    // Must be immediately followed by `(` to count as an
                    // invocation.  The `(` may carry a leading space —
                    // C17 allows whitespace between the macro name and
                    // its opening paren — but must not start a new line
                    // inside a preprocessing directive.  We treat any
                    // next token kind of `LeftParen` as an invocation.
                    match cursor.peek_nth(1) {
                        Some(t) if matches!(t.kind(), TokenKind::LeftParen) => (name.clone(), true),
                        _ => return false,
                    }
                }
                _ => return false,
            }
        };

        // Consume the invocation now that we have committed to expanding.
        let invocation = cursor.advance().expect("peek succeeded");

        if is_fn_like {
            self.expand_function_like(cursor, name, invocation)
        } else {
            self.expand_object_like(cursor, name, invocation)
        }
    }

    /// Splice an object-like macro's replacement list back into `cursor`
    /// for rescanning.  Returns `true` (expansion always succeeds once we
    /// commit to it).
    fn expand_object_like(
        &mut self,
        cursor: &mut TokenCursor,
        name: String,
        invocation: PPToken,
    ) -> bool {
        let replacement = match self.macros.get(&name) {
            Some(MacroDef::ObjectLike { replacement, .. }) => replacement.clone(),
            _ => unreachable!("caller resolved `{name}` to an object-like macro"),
        };

        let mut new_hide_set = invocation.hide_set.clone();
        new_hide_set.insert(name);

        let expansion: Vec<PPToken> = replacement
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let mut pp = PPToken::with_hide_set(t, new_hide_set.clone());
                if i == 0 {
                    // The first expansion token takes the invocation's
                    // position flags so its role in the surrounding
                    // stream is preserved.
                    pp.token.at_start_of_line = invocation.token.at_start_of_line;
                    pp.token.has_leading_space = invocation.token.has_leading_space;
                }
                pp
            })
            .collect();

        cursor.push_front(expansion);
        true
    }

    /// Expand a function-like macro invocation.  `invocation` is the
    /// macro-name token already consumed from `cursor`; the cursor's
    /// next token is the opening `(`.
    ///
    /// The four-phase §6.10.3 algorithm:
    ///
    /// 1. **Collect** — gather the comma-separated arguments (respecting
    ///    nested parens) up to the matching `)`.
    /// 2. **Substitute** — walk the replacement list; each parameter use
    ///    is replaced by either the raw argument (if adjacent to `#` or
    ///    `##`) or the fully-expanded argument (otherwise).  `#` turns
    ///    into a StringLiteral per C17 §6.10.3.2.
    /// 3. **Paste** — for each `##`, concatenate the spellings of its
    ///    two operands and re-lex the result with [`lex_fragment`].
    /// 4. **Hide-set** — add the macro name to every resulting token's
    ///    hide set, union the invocation's own hide set, and splice back
    ///    into the cursor for rescanning.
    fn expand_function_like(
        &mut self,
        cursor: &mut TokenCursor,
        name: String,
        invocation: PPToken,
    ) -> bool {
        // Consume the `(` — we verified it's there in `try_expand`.
        let lparen = cursor
            .advance()
            .expect("try_expand peeked `(` before commit");
        let lparen_span = lparen.token.span;

        let (params, is_variadic, replacement) = match self.macros.get(&name) {
            Some(MacroDef::FunctionLike {
                params,
                is_variadic,
                replacement,
                ..
            }) => (params.clone(), *is_variadic, replacement.clone()),
            _ => unreachable!("caller resolved `{name}` to a function-like macro"),
        };

        let args = match self.collect_macro_arguments(
            cursor,
            &name,
            lparen_span,
            params.len(),
            is_variadic,
        ) {
            Some(a) => a,
            None => return true, // error already reported; treat the invocation as consumed
        };

        // Arity check: named params must be satisfied; variadic macros
        // want at least `params.len()` arguments; non-variadic want
        // exactly `params.len()` (with the usual "F() == one empty arg
        // when the macro has a single parameter" rule baked into
        // `collect_macro_arguments`).
        let expected_min = params.len();
        let too_few = if is_variadic {
            args.len() < expected_min
        } else {
            args.len() != expected_min
        };
        if too_few {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "macro `{name}` expects {} argument{}, got {}",
                    expected_min,
                    if expected_min == 1 { "" } else { "s" },
                    args.len()
                ))
                .span(lparen_span.range()),
            );
            return true;
        }

        // 2 + 3: substitute parameters, then process `##`.
        let substituted = self.substitute_args(&replacement, &params, is_variadic, &args);
        let pasted = self.process_paste(substituted);

        // 4: hide-set.
        let mut extra_hide: HashSet<String> = invocation.hide_set.clone();
        extra_hide.insert(name);
        let mut final_tokens: Vec<PPToken> = pasted
            .into_iter()
            .map(|mut pp| {
                pp.hide_set.extend(extra_hide.iter().cloned());
                pp
            })
            .collect();
        if let Some(first) = final_tokens.first_mut() {
            first.token.at_start_of_line = invocation.token.at_start_of_line;
            first.token.has_leading_space = invocation.token.has_leading_space;
        }

        cursor.push_front(final_tokens);
        true
    }

    // -----------------------------------------------------------------
    // Function-like helpers: argument collection, substitution, pasting
    // -----------------------------------------------------------------

    /// Consume the argument list of a function-like macro invocation.
    /// The opening `(` has already been consumed.
    ///
    /// Returns one [`Vec<PPToken>`] per argument, in order.  Nested
    /// parentheses are tracked so `F((a, b), c)` produces two
    /// arguments: `(a, b)` and `c`.
    ///
    /// Variadic macros suppress the comma-split once `params.len()`
    /// commas have been consumed: every further comma goes into the
    /// `__VA_ARGS__` argument.
    ///
    /// Special shapes:
    ///
    /// * `F()` with a zero-parameter macro — zero arguments.
    /// * `F()` with a one-parameter macro — one empty argument.
    /// * `F(,)` — two empty arguments.
    ///
    /// On an unterminated argument list an error diagnostic is recorded
    /// and `None` is returned; the cursor is left at the offending
    /// position (end-of-line or EOF).
    fn collect_macro_arguments(
        &mut self,
        cursor: &mut TokenCursor,
        macro_name: &str,
        lparen_span: Span,
        param_count: usize,
        is_variadic: bool,
    ) -> Option<Vec<Vec<PPToken>>> {
        let mut args: Vec<Vec<PPToken>> = Vec::new();
        let mut current: Vec<PPToken> = Vec::new();
        let mut paren_depth: usize = 1;
        let mut saw_comma = false;

        loop {
            let tok = match cursor.advance() {
                Some(t) => t,
                None => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "unterminated argument list invoking macro `{macro_name}`"
                        ))
                        .span(lparen_span.range()),
                    );
                    return None;
                }
            };

            if matches!(tok.kind(), TokenKind::Eof) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "unterminated argument list invoking macro `{macro_name}`"
                    ))
                    .span(lparen_span.range()),
                );
                cursor.push_front(vec![tok]);
                return None;
            }

            match tok.kind() {
                TokenKind::LeftParen => {
                    paren_depth += 1;
                    current.push(tok);
                }
                TokenKind::RightParen => {
                    paren_depth -= 1;
                    if paren_depth == 0 {
                        // Commit the argument in flight, handling the
                        // zero- and single-parameter corner cases.
                        if !saw_comma && current.is_empty() {
                            if param_count > 0 || is_variadic {
                                args.push(Vec::new());
                            }
                        } else {
                            args.push(current);
                        }
                        break;
                    }
                    current.push(tok);
                }
                TokenKind::Comma => {
                    // Top-level commas split arguments.  Once the
                    // variadic threshold is reached, further commas are
                    // literal content of `__VA_ARGS__`.
                    let at_variadic_tail = is_variadic && args.len() >= param_count;
                    if paren_depth == 1 && !at_variadic_tail {
                        saw_comma = true;
                        args.push(std::mem::take(&mut current));
                    } else {
                        current.push(tok);
                    }
                }
                _ => {
                    current.push(tok);
                }
            }
        }

        // If the macro is variadic and the caller did not pass an explicit
        // variadic portion, synthesise one empty argument so downstream
        // substitution always finds `__VA_ARGS__` at index `param_count`.
        if is_variadic && args.len() == param_count {
            args.push(Vec::new());
        }

        Some(args)
    }

    /// Walk the replacement list and substitute parameter references per
    /// C17 §6.10.3.1.
    ///
    /// The rules:
    ///
    /// * `#`-prefixed parameter → StringLiteral built from the **raw**
    ///   argument tokens via [`stringify`].
    /// * Parameter adjacent (left or right) to `##` → **raw** argument.
    /// * Any other parameter → argument **fully expanded** once,
    ///   memoised so repeated uses share the expansion.
    /// * `__VA_ARGS__` in a variadic macro → the final argument, raw or
    ///   expanded using the same rules as an ordinary parameter.
    ///
    /// Non-parameter tokens pass through unchanged.
    fn substitute_args(
        &mut self,
        replacement: &[Token],
        params: &[String],
        is_variadic: bool,
        args: &[Vec<PPToken>],
    ) -> Vec<PPToken> {
        let mut out: Vec<PPToken> = Vec::new();
        // `expanded[i] == Some(_)` once argument `i` has been expanded
        // at least once; the cached expansion is reused for subsequent
        // uses of the same parameter.
        let mut expanded: Vec<Option<Vec<PPToken>>> = vec![None; args.len()];

        let param_index = |name: &str| -> Option<usize> {
            if let Some(i) = params.iter().position(|p| p == name) {
                Some(i)
            } else if is_variadic && name == VA_ARGS {
                Some(params.len())
            } else {
                None
            }
        };

        let mut i = 0;
        while i < replacement.len() {
            let tok = &replacement[i];

            // `#` + parameter → stringify.  Only fires if the next token
            // is an identifier naming a parameter (or `__VA_ARGS__` in a
            // variadic macro).
            if matches!(tok.kind, TokenKind::Hash) {
                if let Some(next) = replacement.get(i + 1) {
                    if let TokenKind::Identifier(n) = &next.kind {
                        if let Some(idx) = param_index(n) {
                            let body = stringify(&args[idx]);
                            let str_tok = Token {
                                kind: TokenKind::StringLiteral {
                                    value: body,
                                    prefix: StringPrefix::None,
                                },
                                span: tok.span,
                                at_start_of_line: tok.at_start_of_line,
                                has_leading_space: tok.has_leading_space,
                            };
                            out.push(PPToken::new(str_tok));
                            i += 2;
                            continue;
                        }
                    }
                }
                // Not a stringification site — emit `#` verbatim.
                out.push(PPToken::new(tok.clone()));
                i += 1;
                continue;
            }

            // Parameter identifier (or `__VA_ARGS__`).
            if let TokenKind::Identifier(n) = &tok.kind {
                if let Some(idx) = param_index(n) {
                    let prev_is_paste =
                        i > 0 && matches!(replacement[i - 1].kind, TokenKind::HashHash);
                    let next_is_paste = i + 1 < replacement.len()
                        && matches!(replacement[i + 1].kind, TokenKind::HashHash);
                    let use_raw = prev_is_paste || next_is_paste;

                    let to_splice: Vec<PPToken> = if use_raw {
                        args[idx].clone()
                    } else {
                        if expanded[idx].is_none() {
                            expanded[idx] = Some(self.expand_tokens(args[idx].clone()));
                        }
                        expanded[idx].clone().unwrap_or_default()
                    };

                    if !to_splice.is_empty() {
                        // Preserve the parameter-use's spacing flags on
                        // the first substituted token so surrounding
                        // whitespace is not lost.
                        let mut iter = to_splice.into_iter();
                        if let Some(mut first) = iter.next() {
                            first.token.at_start_of_line = tok.at_start_of_line;
                            first.token.has_leading_space = tok.has_leading_space;
                            out.push(first);
                            out.extend(iter);
                        }
                    }
                    i += 1;
                    continue;
                }
            }

            // Any other token passes through.
            out.push(PPToken::new(tok.clone()));
            i += 1;
        }

        out
    }

    /// Process every `##` in `tokens`, left to right.  For each, pop the
    /// left operand from the output built so far, take the right operand
    /// from the input, concatenate their spellings via
    /// [`paste_spelling`], and feed the result back through
    /// [`lex_fragment`].
    ///
    /// Per C17 §6.10.3.3:
    ///
    /// * The result's hide set is the **intersection** of its two
    ///   operands' hide sets.
    /// * Pasting a placeholder (empty side) with a token reproduces the
    ///   other token.
    /// * If re-lexing produces more than one token, the pasted text is
    ///   not a valid preprocessing token — a warning is emitted but the
    ///   multiple tokens are kept (matches GCC / Clang behaviour).
    fn process_paste(&mut self, tokens: Vec<PPToken>) -> Vec<PPToken> {
        let mut out: Vec<PPToken> = Vec::new();
        let mut it = tokens.into_iter().peekable();

        while let Some(tok) = it.next() {
            if matches!(tok.kind(), TokenKind::HashHash) {
                let left = out.pop();
                let right = it.next();
                let merged = self.paste_two(left, right, tok.span());
                out.extend(merged);
                continue;
            }
            out.push(tok);
        }

        out
    }

    /// Paste two tokens together and return the re-lexed result.
    ///
    /// Implements the paste rules from C17 §6.10.3.3: empty sides
    /// degrade to the non-empty side; two empty sides produce no
    /// tokens; a non-single-token result emits a warning.
    fn paste_two(
        &mut self,
        left: Option<PPToken>,
        right: Option<PPToken>,
        hh_span: Span,
    ) -> Vec<PPToken> {
        let left_tok = left.as_ref().map(|p| &p.token);
        let right_tok = right.as_ref().map(|p| &p.token);
        let combined = paste_spelling(left_tok, right_tok);

        if combined.is_empty() {
            return Vec::new();
        }

        let new_tokens = lex_fragment(&combined);
        if new_tokens.is_empty() {
            return Vec::new();
        }
        if new_tokens.len() > 1 {
            let left_text = left_tok.map(|t| spelling_of(&t.kind)).unwrap_or_default();
            let right_text = right_tok.map(|t| spelling_of(&t.kind)).unwrap_or_default();
            self.diagnostics.push(
                Diagnostic::warning(format!(
                    "pasting `{left_text}` and `{right_text}` does not give a valid preprocessing token"
                ))
                .span(hh_span.range())
                .note("the result will be kept as multiple tokens"),
            );
        }

        let hide_set: HashSet<String> = match (left.as_ref(), right.as_ref()) {
            (Some(l), Some(r)) => l.hide_set.intersection(&r.hide_set).cloned().collect(),
            (Some(l), None) => l.hide_set.clone(),
            (None, Some(r)) => r.hide_set.clone(),
            (None, None) => HashSet::new(),
        };

        // Preserve the left operand's position flags on the first
        // pasted token so spacing around the original macro use is not
        // erased.  If there is no left operand, fall back to the right.
        let (sol, ls) = match (&left, &right) {
            (Some(l), _) => (l.token.at_start_of_line, l.token.has_leading_space),
            (None, Some(r)) => (r.token.at_start_of_line, r.token.has_leading_space),
            (None, None) => (false, false),
        };

        new_tokens
            .into_iter()
            .enumerate()
            .map(|(idx, t)| {
                let mut pp = PPToken::with_hide_set(t, hide_set.clone());
                if idx == 0 {
                    pp.token.at_start_of_line = sol;
                    pp.token.has_leading_space = ls;
                }
                pp
            })
            .collect()
    }

    /// Run the macro-expansion engine over a standalone token list.
    ///
    /// Used to pre-expand function-like macro arguments before they are
    /// spliced into the replacement list (C17 §6.10.3.1/1).  This does
    /// **not** process directives — the tokens are assumed to be
    /// expression-shaped (no `#` at start-of-line will appear inside a
    /// macro argument).
    fn expand_tokens(&mut self, tokens: Vec<PPToken>) -> Vec<PPToken> {
        let mut cursor = TokenCursor::new(tokens);
        let mut out: Vec<PPToken> = Vec::new();
        while cursor.peek().is_some() {
            if self.try_expand(&mut cursor) {
                continue;
            }
            if let Some(tok) = cursor.advance() {
                if matches!(tok.kind(), TokenKind::Eof) {
                    break;
                }
                out.push(tok);
            }
        }
        out
    }

    // -----------------------------------------------------------------
    // Directive dispatch
    // -----------------------------------------------------------------

    /// Handle a preprocessing directive whose opening `#` has already
    /// been consumed and is passed in as `hash` for diagnostic spans.
    fn handle_directive(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        // The directive name lives on the same line as the `#`.
        let Some(name_tok) = cursor.peek() else {
            return;
        };

        // A bare `#` followed by end-of-line (or EOF) is a **null
        // directive** — valid C17, and a no-op.
        if name_tok.at_start_of_line() || matches!(name_tok.kind(), TokenKind::Eof) {
            return;
        }

        // Consume the directive name now that we know it is on the
        // current line.
        let name_tok = match cursor.advance() {
            Some(t) => t,
            None => return,
        };

        let directive_name = directive_name_of(&name_tok.token);
        let is_conditional = matches!(
            directive_name.as_deref(),
            Some("if" | "ifdef" | "ifndef" | "elif" | "else" | "endif")
        );

        // Inside a skipped conditional block every non-conditional
        // directive is discarded silently — C17 guarantees that the
        // skipped group is only examined for conditional structure, so
        // `#define`/`#include`/etc. arguments that may be malformed do
        // not produce noise here.
        if !self.is_active() && !is_conditional {
            cursor.skip_to_end_of_line();
            return;
        }

        match directive_name.as_deref() {
            Some("define") => self.handle_define(hash, cursor),
            Some("undef") => self.handle_undef(hash, cursor),
            Some("if") => self.handle_if(&name_tok.token, cursor),
            Some("ifdef") => self.handle_ifdef(&name_tok.token, cursor),
            Some("ifndef") => self.handle_ifndef(&name_tok.token, cursor),
            Some("elif") => self.handle_elif(&name_tok.token, cursor),
            Some("else") => self.handle_else(&name_tok.token, cursor),
            Some("endif") => self.handle_endif(&name_tok.token, cursor),
            Some("include") => self.handle_include(&name_tok.token, cursor),
            Some("error") => self.handle_error(hash, cursor),
            Some("warning") => self.handle_warning(hash, cursor),
            Some("line") => self.handle_line(hash, cursor),
            Some("pragma") => self.handle_pragma(hash, cursor),
            Some(other) => {
                self.diagnostics.push(
                    Diagnostic::error(format!("unknown preprocessing directive `#{other}`"))
                        .span(name_tok.token.span.range())
                        .label("no directive with this name"),
                );
                cursor.skip_to_end_of_line();
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("expected preprocessing directive name after `#`")
                        .span(name_tok.token.span.range()),
                );
                cursor.skip_to_end_of_line();
            }
        }
    }

    /// Emit an "unterminated #if" error for every still-open
    /// conditional.  Called once the input runs out.
    fn flush_unterminated_ifs(&mut self) {
        let frames = std::mem::take(&mut self.if_stack);
        for frame in frames {
            self.diagnostics.push(
                Diagnostic::error("unterminated `#if` block — missing `#endif`")
                    .span(frame.if_location.range())
                    .label("conditional opened here"),
            );
        }
    }

    /// `true` iff every frame on the `#if` stack is currently active.
    /// An empty stack counts as active, so translation-unit-level code
    /// is always emitted.
    fn is_active(&self) -> bool {
        self.if_stack.iter().all(|s| s.current_branch_active)
    }

    // -----------------------------------------------------------------
    // Conditional compilation — §6.10.1
    // -----------------------------------------------------------------

    /// Handle `#if EXPR`.
    ///
    /// The expression is evaluated only when the enclosing conditional
    /// is itself active: inside a skipped group we push an **inert**
    /// [`IfState`] that never becomes active no matter what `#elif` or
    /// `#else` follows.  This matches C17 §6.10.1/6: directives in a
    /// skipped group are examined to identify the matching `#endif`,
    /// but their expressions are not evaluated.
    fn handle_if(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let if_span = name_tok.span;
        let enclosing_active = self.is_active();
        let line_tokens = cursor.collect_to_end_of_line();

        let active = if enclosing_active {
            let value = self.evaluate_if_expression(line_tokens, if_span);
            !value.is_zero()
        } else {
            false
        };
        self.push_if_frame(active, enclosing_active, if_span);
    }

    /// Handle `#ifdef NAME`.
    fn handle_ifdef(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let ifdef_span = name_tok.span;
        let enclosing_active = self.is_active();
        let active = if enclosing_active {
            match self.read_conditional_identifier(cursor, name_tok, "ifdef") {
                Some(ident) => self.macros.contains_key(&ident),
                None => false,
            }
        } else {
            cursor.skip_to_end_of_line();
            false
        };
        self.push_if_frame(active, enclosing_active, ifdef_span);
    }

    /// Handle `#ifndef NAME`.
    fn handle_ifndef(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let ifndef_span = name_tok.span;
        let enclosing_active = self.is_active();
        let active = if enclosing_active {
            match self.read_conditional_identifier(cursor, name_tok, "ifndef") {
                Some(ident) => !self.macros.contains_key(&ident),
                None => false,
            }
        } else {
            cursor.skip_to_end_of_line();
            false
        };
        self.push_if_frame(active, enclosing_active, ifndef_span);
    }

    /// Handle `#elif EXPR`.
    ///
    /// If the matching frame already took an earlier branch, or if the
    /// enclosing conditional is inactive, the expression is not
    /// evaluated — the frame stays inactive.
    fn handle_elif(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let elif_span = name_tok.span;
        let line_tokens = cursor.collect_to_end_of_line();

        if self.if_stack.is_empty() {
            self.diagnostics
                .push(Diagnostic::error("`#elif` without matching `#if`").span(elif_span.range()));
            return;
        }

        let top = self.if_stack.len() - 1;
        if self.if_stack[top].else_seen {
            self.diagnostics
                .push(Diagnostic::error("`#elif` after `#else`").span(elif_span.range()));
            self.if_stack[top].current_branch_active = false;
            return;
        }

        let enclosing_active = self.if_stack[..top].iter().all(|f| f.current_branch_active);
        let any_branch_taken = self.if_stack[top].any_branch_taken;

        if !enclosing_active || any_branch_taken {
            self.if_stack[top].current_branch_active = false;
            return;
        }

        let value = self.evaluate_if_expression(line_tokens, elif_span);
        let condition = !value.is_zero();
        self.if_stack[top].current_branch_active = condition;
        if condition {
            self.if_stack[top].any_branch_taken = true;
        }
    }

    /// Handle `#else`.
    fn handle_else(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let else_span = name_tok.span;
        cursor.skip_to_end_of_line();

        if self.if_stack.is_empty() {
            self.diagnostics
                .push(Diagnostic::error("`#else` without matching `#if`").span(else_span.range()));
            return;
        }

        let top = self.if_stack.len() - 1;
        if self.if_stack[top].else_seen {
            self.diagnostics.push(
                Diagnostic::error("duplicate `#else` in the same `#if` block")
                    .span(else_span.range()),
            );
            return;
        }
        self.if_stack[top].else_seen = true;

        let enclosing_active = self.if_stack[..top].iter().all(|f| f.current_branch_active);
        if !enclosing_active {
            self.if_stack[top].current_branch_active = false;
        } else {
            let any = self.if_stack[top].any_branch_taken;
            let now_active = !any;
            self.if_stack[top].current_branch_active = now_active;
            if now_active {
                self.if_stack[top].any_branch_taken = true;
            }
        }
    }

    /// Handle `#endif` — pop the current frame.
    fn handle_endif(&mut self, name_tok: &Token, cursor: &mut TokenCursor) {
        let endif_span = name_tok.span;
        cursor.skip_to_end_of_line();
        if self.if_stack.pop().is_none() {
            self.diagnostics.push(
                Diagnostic::error("`#endif` without matching `#if`").span(endif_span.range()),
            );
        }
    }

    /// Push a fresh [`IfState`] for a just-opened `#if` / `#ifdef` /
    /// `#ifndef`.  When `enclosing_active` is false the frame is **inert**
    /// — it can never become active regardless of `#elif` or `#else`.
    fn push_if_frame(&mut self, active: bool, enclosing_active: bool, span: Span) {
        if enclosing_active {
            self.if_stack.push(IfState::new(active, span));
        } else {
            self.if_stack.push(IfState {
                any_branch_taken: true,
                current_branch_active: false,
                else_seen: false,
                if_location: span,
            });
        }
    }

    /// Read the single-identifier operand of `#ifdef` / `#ifndef`.  On
    /// any shape mismatch an error is recorded and `None` is returned;
    /// the rest of the line is consumed so the main loop keeps moving.
    fn read_conditional_identifier(
        &mut self,
        cursor: &mut TokenCursor,
        name_tok: &Token,
        directive_name: &str,
    ) -> Option<String> {
        let tok = self.expect_identifier_on_line(
            cursor,
            name_tok,
            &format!("`#{directive_name}` requires an identifier"),
        )?;
        cursor.skip_to_end_of_line();
        match tok.kind {
            TokenKind::Identifier(s) => Some(s),
            _ => unreachable!("expect_identifier_on_line guarantees an Identifier"),
        }
    }

    /// Evaluate the expression portion of `#if` / `#elif`: substitute
    /// `defined`, macro-expand, rewrite remaining identifiers to `0`,
    /// then hand the cleaned token stream to [`cond_expr::evaluate`].
    fn evaluate_if_expression(&mut self, line_tokens: Vec<PPToken>, if_location: Span) -> PPValue {
        let after_defined = self.substitute_defined_operator(line_tokens);
        let after_has_include = self.substitute_has_include_operator(after_defined);
        let expanded = self.expand_tokens(after_has_include);
        let after_zero = zero_remaining_identifiers(expanded);
        let raw: Vec<Token> = after_zero.into_iter().map(|pp| pp.token).collect();
        let (value, diags) = cond_expr::evaluate(&raw, if_location);
        self.diagnostics.extend(diags);
        value
    }

    /// Replace every `defined IDENT` and `defined ( IDENT )` in `tokens`
    /// with `1` / `0` per C17 §6.10.1/1.
    ///
    /// This runs **before** macro expansion so that `defined FOO` is
    /// answered by the macro table rather than by expanding `FOO`.
    fn substitute_defined_operator(&mut self, tokens: Vec<PPToken>) -> Vec<PPToken> {
        let mut out: Vec<PPToken> = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let tok = &tokens[i];
            let is_defined = matches!(&tok.token.kind, TokenKind::Identifier(s) if s == "defined");
            if !is_defined {
                out.push(tok.clone());
                i += 1;
                continue;
            }

            // Found a `defined` operator.  It may be `defined IDENT` or
            // `defined ( IDENT )`.
            let mut j = i + 1;
            let with_paren = matches!(
                tokens.get(j).map(|t| &t.token.kind),
                Some(TokenKind::LeftParen)
            );
            if with_paren {
                j += 1;
            }
            let ident_name = match tokens.get(j) {
                Some(t) => match &t.token.kind {
                    TokenKind::Identifier(s) => Some(s.clone()),
                    _ => None,
                },
                None => None,
            };

            match ident_name {
                Some(name) => {
                    j += 1;
                    if with_paren {
                        match tokens.get(j).map(|t| &t.token.kind) {
                            Some(TokenKind::RightParen) => {
                                j += 1;
                            }
                            _ => {
                                self.diagnostics.push(
                                    Diagnostic::error(
                                        "expected `)` after identifier in `defined(...)`",
                                    )
                                    .span(tok.token.span.range()),
                                );
                            }
                        }
                    }
                    let value: u64 = if self.macros.contains_key(&name) {
                        1
                    } else {
                        0
                    };
                    out.push(PPToken::new(replacement_int_literal(&tok.token, value)));
                    i = j;
                }
                None => {
                    self.diagnostics.push(
                        Diagnostic::error("`defined` requires an identifier operand")
                            .span(tok.token.span.range()),
                    );
                    out.push(PPToken::new(replacement_int_literal(&tok.token, 0)));
                    i += 1;
                }
            }
        }
        out
    }

    /// Replace every `__has_include ( <header> )` and
    /// `__has_include ( "header" )` in `tokens` with `1` or `0` depending
    /// on whether [`Preprocessor::resolve_include`] can find a file.
    ///
    /// Must run **before** macro expansion: the `<stdio.h>` argument does
    /// not lex as a single token and would be mangled by expansion's
    /// identifier-scan logic.  Unlike `defined`, `__has_include` is a
    /// conditional-expression-only construct — this rewrite is only
    /// called from [`Preprocessor::evaluate_if_expression`].
    fn substitute_has_include_operator(&mut self, tokens: Vec<PPToken>) -> Vec<PPToken> {
        let mut out: Vec<PPToken> = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let tok = &tokens[i];
            let is_has_include =
                matches!(&tok.token.kind, TokenKind::Identifier(s) if s == "__has_include");
            if !is_has_include {
                out.push(tok.clone());
                i += 1;
                continue;
            }

            // Expect a `(`.
            let mut j = i + 1;
            if !matches!(tokens.get(j).map(|t| t.kind()), Some(TokenKind::LeftParen)) {
                self.diagnostics.push(
                    Diagnostic::error("`__has_include` requires a parenthesised argument")
                        .span(tok.token.span.range()),
                );
                out.push(PPToken::new(replacement_int_literal(&tok.token, 0)));
                i += 1;
                continue;
            }
            j += 1;

            // Collect tokens until matching `)`, tracking nesting just in
            // case the argument contained stray parens.
            let mut inner: Vec<PPToken> = Vec::new();
            let mut depth = 1;
            while let Some(inner_tok) = tokens.get(j) {
                match inner_tok.kind() {
                    TokenKind::LeftParen => {
                        depth += 1;
                        inner.push(inner_tok.clone());
                    }
                    TokenKind::RightParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        inner.push(inner_tok.clone());
                    }
                    _ => inner.push(inner_tok.clone()),
                }
                j += 1;
            }
            if depth != 0 {
                self.diagnostics.push(
                    Diagnostic::error("unterminated `__has_include(...)` argument list")
                        .span(tok.token.span.range()),
                );
                out.push(PPToken::new(replacement_int_literal(&tok.token, 0)));
                i = j;
                continue;
            }
            // Step past the `)`.
            j += 1;

            let value = match header_name_from_tokens(&inner) {
                Some((header, is_system)) => {
                    if self.resolve_include(&header, is_system).is_some() {
                        1
                    } else {
                        0
                    }
                }
                None => {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "`__has_include` argument must be `<header>` or `\"header\"`",
                        )
                        .span(tok.token.span.range()),
                    );
                    0
                }
            };
            out.push(PPToken::new(replacement_int_literal(&tok.token, value)));
            i = j;
        }
        out
    }

    // -----------------------------------------------------------------
    // #define / #undef
    // -----------------------------------------------------------------

    /// Handle `#define NAME ...` (both object- and function-like).
    fn handle_define(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        // Consume the macro name.
        let name_tok =
            match self.expect_identifier_on_line(cursor, hash, "`#define` requires a macro name") {
                Some(t) => t,
                None => return,
            };

        let name = match &name_tok.kind {
            TokenKind::Identifier(s) => s.clone(),
            _ => unreachable!("expect_identifier_on_line returned a non-identifier"),
        };

        // Function-like iff the next token is `(` with NO leading space
        // (and on the same line).  With a leading space the `(` is part
        // of the replacement list, so this is an object-like macro.
        let function_like = matches!(
            cursor.peek(),
            Some(t)
                if matches!(t.kind(), TokenKind::LeftParen)
                    && !t.has_leading_space()
                    && !t.at_start_of_line()
        );

        let new_def = if function_like {
            // Consume the `(`.
            cursor.advance();
            match self.parse_parameter_list(&name, &name_tok, cursor) {
                Some((params, is_variadic)) => {
                    // Strip hide sets on storage — the replacement list
                    // has not been expanded yet, and expansion will
                    // compute fresh hide sets at invocation time.
                    let replacement = unwrap_tokens(cursor.collect_to_end_of_line());
                    MacroDef::FunctionLike {
                        name: name.clone(),
                        params,
                        is_variadic,
                        replacement,
                    }
                }
                None => return,
            }
        } else {
            let replacement = unwrap_tokens(cursor.collect_to_end_of_line());
            MacroDef::ObjectLike {
                name: name.clone(),
                replacement,
                is_predefined: false,
            }
        };

        if let Some(existing) = self.macros.get(&name) {
            if !macros_equivalent(existing, &new_def) {
                self.diagnostics.push(
                    Diagnostic::warning(format!("`{name}` redefined"))
                        .span(name_tok.span.range())
                        .label("redefinition differs from the previous definition"),
                );
            }
        }

        self.macros.insert(name, new_def);
    }

    /// Parse the parameter list of a function-like macro.  The opening
    /// `(` has already been consumed.
    ///
    /// Returns `Some((params, is_variadic))` on success, or `None` on a
    /// syntax error (the diagnostic has already been pushed and the rest
    /// of the line consumed).
    fn parse_parameter_list(
        &mut self,
        macro_name: &str,
        name_tok: &Token,
        cursor: &mut TokenCursor,
    ) -> Option<(Vec<String>, bool)> {
        let mut params: Vec<String> = Vec::new();
        let mut is_variadic = false;

        // Special case: `#define F() body` — empty parameter list.
        if matches!(cursor.peek(), Some(t) if matches!(t.kind(), TokenKind::RightParen)) {
            cursor.advance();
            return Some((params, is_variadic));
        }

        loop {
            let Some(tok) = cursor.advance() else {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "unterminated parameter list in `#define {macro_name}`"
                    ))
                    .span(name_tok.span.range())
                    .label("macro name declared here"),
                );
                return None;
            };

            if tok.at_start_of_line() || matches!(tok.kind(), TokenKind::Eof) {
                self.diagnostics.push(
                    Diagnostic::error(format!(
                        "unterminated parameter list in `#define {macro_name}`"
                    ))
                    .span(name_tok.span.range())
                    .label("macro name declared here"),
                );
                // Put the token back so the main loop re-enters on the
                // new line / EOF cleanly.
                cursor.push_front(vec![tok]);
                return None;
            }

            let tok_span = tok.token.span;
            match tok.token.kind {
                TokenKind::Ellipsis => {
                    is_variadic = true;
                    match cursor.advance() {
                        Some(t) if matches!(t.kind(), TokenKind::RightParen) => {
                            return Some((params, is_variadic));
                        }
                        other => {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    "`...` must be the last element of a macro parameter list",
                                )
                                .span(tok_span.range()),
                            );
                            if let Some(t) = other {
                                cursor.push_front(vec![t]);
                            }
                            cursor.skip_to_end_of_line();
                            return None;
                        }
                    }
                }
                TokenKind::Identifier(param_name) => {
                    if params.contains(&param_name) {
                        self.diagnostics.push(
                            Diagnostic::warning(format!(
                                "duplicate macro parameter `{param_name}`"
                            ))
                            .span(tok_span.range()),
                        );
                    }
                    params.push(param_name);
                    match cursor.advance() {
                        Some(t) if matches!(t.kind(), TokenKind::Comma) => {
                            // Another parameter follows.
                        }
                        Some(t) if matches!(t.kind(), TokenKind::RightParen) => {
                            return Some((params, is_variadic));
                        }
                        other => {
                            self.diagnostics.push(
                                Diagnostic::error(format!(
                                    "expected `,` or `)` in parameter list of `#define {macro_name}`"
                                ))
                                .span(tok_span.range()),
                            );
                            if let Some(t) = other {
                                cursor.push_front(vec![t]);
                            }
                            cursor.skip_to_end_of_line();
                            return None;
                        }
                    }
                }
                _ => {
                    self.diagnostics.push(
                        Diagnostic::error(format!(
                            "expected a parameter name in `#define {macro_name}`"
                        ))
                        .span(tok_span.range()),
                    );
                    cursor.skip_to_end_of_line();
                    return None;
                }
            }
        }
    }

    /// Handle `#undef NAME`.  Removing a name that is not defined is
    /// **not** an error in C17.
    fn handle_undef(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let name_tok =
            match self.expect_identifier_on_line(cursor, hash, "`#undef` requires a macro name") {
                Some(t) => t,
                None => return,
            };

        if let TokenKind::Identifier(name) = &name_tok.kind {
            self.macros.remove(name);
        }
        cursor.skip_to_end_of_line();
    }

    /// Consume the next token and require it to be an [`TokenKind::Identifier`]
    /// on the current line.  On any mismatch, push a diagnostic anchored
    /// at `anchor` (typically the directive's `#` token) with `missing_msg`,
    /// then skip to end-of-line and return `None`.
    fn expect_identifier_on_line(
        &mut self,
        cursor: &mut TokenCursor,
        anchor: &Token,
        missing_msg: &str,
    ) -> Option<Token> {
        let Some(tok) = cursor.advance() else {
            self.diagnostics
                .push(Diagnostic::error(missing_msg.to_string()).span(anchor.span.range()));
            return None;
        };

        if tok.at_start_of_line() || matches!(tok.kind(), TokenKind::Eof) {
            self.diagnostics
                .push(Diagnostic::error(missing_msg.to_string()).span(anchor.span.range()));
            cursor.push_front(vec![tok]);
            return None;
        }

        if !matches!(tok.kind(), TokenKind::Identifier(_)) {
            self.diagnostics.push(
                Diagnostic::error(format!("{missing_msg} (found a non-identifier token)"))
                    .span(tok.token.span.range()),
            );
            cursor.skip_to_end_of_line();
            return None;
        }

        // Shed the (empty) hide set — identifiers read here are always
        // directive-body tokens, so the hide set is moot.
        Some(tok.into_token())
    }

    // -----------------------------------------------------------------
    // Predefined macros
    // -----------------------------------------------------------------

    /// Install the preprocessor's baseline macro table: every C17-mandated
    /// predefined macro (`__STDC__`, `__STDC_VERSION__`, `__STDC_HOSTED__`,
    /// `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`), the
    /// architecture-, platform-, and GCC-compatibility macros, a pile of
    /// `__SIZEOF_*__` / `__*_MAX__` / `__*_TYPE__` macros that system
    /// headers probe for, the `__has_*` query family (all but
    /// `__has_include` return 0 — `__has_include` is special-cased in the
    /// conditional-expression evaluator), and finally each
    /// `-D NAME=VALUE` the caller supplied.
    fn install_predefined_macros(&mut self, user_macros: &[(String, String)]) {
        // C17 / standard pedigree.
        for (name, body) in STANDARD_INT_MACROS {
            self.define_predefined_int_macro(name, body);
        }

        // Type macros — expand to the token sequence that names the type.
        for (name, body) in STANDARD_TYPE_MACROS {
            self.define_predefined_int_macro(name, body);
        }

        // Platform macros, driven by the host the compiler is running on.
        // A future cross-compilation config can override these.
        if cfg!(target_os = "linux") {
            for name in ["__linux__", "__linux", "linux", "__ELF__"] {
                self.define_predefined_int_macro(name, "1");
            }
            for name in ["__unix__", "__unix", "unix"] {
                self.define_predefined_int_macro(name, "1");
            }
        }
        if cfg!(target_os = "macos") {
            for name in ["__APPLE__", "__MACH__"] {
                self.define_predefined_int_macro(name, "1");
            }
            for name in ["__unix__", "__unix"] {
                self.define_predefined_int_macro(name, "1");
            }
        }

        // Target architecture macros.
        match self.target_arch {
            TargetArch::X86_64 => {
                for name in ["__x86_64__", "__x86_64", "__amd64__", "__amd64"] {
                    self.define_predefined_int_macro(name, "1");
                }
            }
            TargetArch::AArch64 => {
                for name in ["__aarch64__", "__ARM_ARCH_8A__"] {
                    self.define_predefined_int_macro(name, "1");
                }
            }
        }

        // GCC/Clang compatibility shims — these let typical system
        // headers parse even though we do not yet implement the
        // attributes they request.
        for (name, body) in GCC_COMPAT_OBJECT_MACROS {
            self.define_predefined_int_macro(name, body);
        }
        // __extension__ evaporates.
        self.define_predefined_int_macro("__extension__", "");

        // Function-like compat macros — `__attribute__(x)`, `__asm__(x)`
        // discard their argument; `__builtin_va_list` maps to `void *`.
        self.define_function_like_predefined("__attribute__", &["x"], false, "");
        self.define_function_like_predefined("__asm__", &["x"], false, "");
        self.define_function_like_predefined("__builtin_va_list", &[], false, "void *");

        // __has_* query family.  `__has_include` is not installed as a
        // real macro: it is intercepted before macro expansion in
        // `substitute_has_include_operator`, because its argument does
        // not lex as a single token.  `__has_include_next` is similarly
        // special-cased (it takes the same `<header>` argument shape).
        // The rest all evaluate to 0 until we grow real support for the
        // features they name.
        for name in [
            "__has_builtin",
            "__has_attribute",
            "__has_feature",
            "__has_extension",
            "__has_warning",
            "__has_c_attribute",
            "__has_cpp_attribute",
            "__has_declspec_attribute",
            "__has_constexpr_builtin",
            // `__has_include_next` is a Clang/GCC extension; we cannot
            // implement its "search after the current directory" rule
            // without tracking include-path ordinals, so treat it as
            // always-0 (matches the common fallback path in headers).
            "__has_include_next",
            // Clang target-feature probes.
            "__is_target_arch",
            "__is_target_vendor",
            "__is_target_os",
            "__is_target_environment",
            "__is_target_variant_os",
            "__is_target_variant_environment",
        ] {
            self.define_function_like_predefined(name, &["x"], false, "0");
        }

        // Dynamic magic macros — `__FILE__` and `__LINE__` are intercepted
        // during expansion; they are recorded here so `#undef __LINE__`
        // has the same visible effect as in GCC.
        self.define_magic_marker("__FILE__");
        self.define_magic_marker("__LINE__");

        // __DATE__ / __TIME__ — captured once at startup.
        let (date, time) = date_time_strings();
        self.define_predefined_string_macro("__DATE__", &date);
        self.define_predefined_string_macro("__TIME__", &time);

        // User -D definitions.  An empty value means "defined as 1",
        // matching the conventional compiler CLI.
        for (name, value) in user_macros {
            let body = if value.is_empty() {
                "1"
            } else {
                value.as_str()
            };
            self.define_predefined_int_macro(name, body);
        }
    }

    /// Define a predefined object-like macro whose body is given as a
    /// plain string — it is lexed in-place and stored.
    fn define_predefined_int_macro(&mut self, name: &str, body: &str) {
        let mut tokens = lex_fragment(body);
        if matches!(tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
            tokens.pop();
        }
        self.macros.insert(
            name.to_string(),
            MacroDef::ObjectLike {
                name: name.to_string(),
                replacement: tokens,
                is_predefined: true,
            },
        );
    }

    /// Define a predefined function-like macro whose body is given as a
    /// plain string.
    fn define_function_like_predefined(
        &mut self,
        name: &str,
        params: &[&str],
        is_variadic: bool,
        body: &str,
    ) {
        let mut replacement = lex_fragment(body);
        if matches!(replacement.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
            replacement.pop();
        }
        self.macros.insert(
            name.to_string(),
            MacroDef::FunctionLike {
                name: name.to_string(),
                params: params.iter().map(|s| s.to_string()).collect(),
                is_variadic,
                replacement,
            },
        );
    }

    /// Install a placeholder entry for a magic macro so that its name is
    /// "defined" for `#ifdef` checks.  The stored replacement list is
    /// empty — the real value is computed at expansion time.
    fn define_magic_marker(&mut self, name: &str) {
        self.macros.insert(
            name.to_string(),
            MacroDef::ObjectLike {
                name: name.to_string(),
                replacement: Vec::new(),
                is_predefined: true,
            },
        );
    }

    /// Define a predefined macro whose single-token replacement is a
    /// string literal with the given value.
    fn define_predefined_string_macro(&mut self, name: &str, value: &str) {
        let tok = Token {
            kind: TokenKind::StringLiteral {
                value: value.to_string(),
                prefix: StringPrefix::None,
            },
            span: Span::new(0, 0),
            at_start_of_line: false,
            has_leading_space: false,
        };
        self.macros.insert(
            name.to_string(),
            MacroDef::ObjectLike {
                name: name.to_string(),
                replacement: vec![tok],
                is_predefined: true,
            },
        );
    }

    /// Handle an in-stream `_Pragma ( "text" )` operator (C17 §6.10.9).
    /// The `_Pragma` identifier is at the cursor's current position and
    /// its following `(` has already been verified.  On success the
    /// full `_Pragma(...)` form is consumed and the destringised body
    /// is processed with the same rules as `#pragma`; the operator
    /// itself contributes no tokens to the output stream.
    fn try_handle_pragma_operator(&mut self, cursor: &mut TokenCursor) -> bool {
        let pragma_tok = cursor.advance().expect("caller verified identifier");
        let anchor_span = pragma_tok.token.span;
        // Consume `(`.
        let _lparen = cursor
            .advance()
            .expect("caller verified left-paren follows `_Pragma`");

        // Expect a single StringLiteral (optionally prefixed).  C17
        // allows only a string literal here, though real compilers also
        // accept implicit-concat runs — a single literal covers the
        // common case.
        let literal = match cursor.advance() {
            Some(t) => t,
            None => {
                self.diagnostics.push(
                    Diagnostic::error("`_Pragma` requires a string-literal argument")
                        .span(anchor_span.range()),
                );
                return true;
            }
        };
        let raw_string = match &literal.token.kind {
            TokenKind::StringLiteral { value, .. } => value.clone(),
            _ => {
                self.diagnostics.push(
                    Diagnostic::error("`_Pragma` requires a string-literal argument")
                        .span(literal.token.span.range()),
                );
                return true;
            }
        };

        // Expect `)`.
        match cursor.advance() {
            Some(t) if matches!(t.kind(), TokenKind::RightParen) => {}
            Some(other) => {
                self.diagnostics.push(
                    Diagnostic::error("expected `)` to close `_Pragma` operator")
                        .span(other.token.span.range()),
                );
                // Put the stray token back so the main loop can
                // continue; we have already consumed the bulk of the
                // operator.
                cursor.push_front(vec![other]);
                return true;
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error("unterminated `_Pragma` operator — missing `)`")
                        .span(anchor_span.range()),
                );
                return true;
            }
        }

        // Destringise and re-lex, then feed through the shared pragma
        // dispatcher.
        let body_text = destringise(&raw_string);
        let body_tokens = lex_fragment(&body_text);
        let body_pp: Vec<PPToken> = body_tokens.into_iter().map(PPToken::new).collect();
        self.process_pragma_body(anchor_span, &body_pp);
        true
    }

    /// Compute the reported 1-based line number for a token whose byte
    /// offset into the current source is `byte_offset`.  Applies the
    /// `#line` translation that is currently in force, if any.
    fn effective_line_number(&self, byte_offset: u32) -> u64 {
        let physical = line_number_at(&self.line_starts, byte_offset) as u32;
        match self.line_offset {
            Some((anchor_actual, anchor_reported)) if physical >= anchor_actual => {
                let delta = physical - anchor_actual;
                (anchor_reported as u64).saturating_add(delta as u64)
            }
            _ => physical as u64,
        }
    }

    /// Dynamic expansion for `__FILE__` and `__LINE__`.  The cursor's
    /// next token is the magic-macro identifier; it is consumed and
    /// replaced by a single freshly-built token.
    fn expand_magic_macro(&mut self, cursor: &mut TokenCursor) -> bool {
        let invocation = match cursor.advance() {
            Some(t) => t,
            None => return false,
        };
        let name = match &invocation.token.kind {
            TokenKind::Identifier(s) => s.clone(),
            _ => unreachable!("expand_magic_macro: caller already verified identifier"),
        };

        let replacement = match name.as_str() {
            "__FILE__" => {
                let name = self
                    .file_override
                    .clone()
                    .unwrap_or_else(|| self.current_file.clone());
                Token {
                    kind: TokenKind::StringLiteral {
                        value: name,
                        prefix: StringPrefix::None,
                    },
                    span: invocation.token.span,
                    at_start_of_line: invocation.token.at_start_of_line,
                    has_leading_space: invocation.token.has_leading_space,
                }
            }
            "__LINE__" => {
                let line = self.effective_line_number(invocation.token.span.start);
                Token {
                    kind: TokenKind::IntegerLiteral {
                        value: line,
                        suffix: IntSuffix::None,
                    },
                    span: invocation.token.span,
                    at_start_of_line: invocation.token.at_start_of_line,
                    has_leading_space: invocation.token.has_leading_space,
                }
            }
            _ => {
                // Not a magic name we recognise — put the token back so
                // the caller re-examines it as an ordinary identifier.
                cursor.push_front(vec![invocation]);
                return false;
            }
        };

        // Hide set gets the name so `__LINE__` inside an argument that
        // re-introduces it cannot recur.
        let mut hide = invocation.hide_set.clone();
        hide.insert(name);
        cursor.push_front(vec![PPToken::with_hide_set(replacement, hide)]);
        true
    }

    // -----------------------------------------------------------------
    // #include
    // -----------------------------------------------------------------

    /// Handle `#include "foo.h"`, `#include <foo.h>`, or a computed
    /// include whose tokens macro-expand into one of those shapes.
    fn handle_include(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let line_tokens = cursor.collect_to_end_of_line();

        let parsed = match self.parse_include_argument(&line_tokens, hash) {
            Some(v) => v,
            None => return, // error already recorded
        };
        let (header, is_system) = parsed;

        // Depth check — outermost frame counts as depth 0, so the first
        // `#include` makes depth 1.
        let current_depth = self.include_stack.last().map(|f| f.depth).unwrap_or(0);
        if current_depth + 1 > self.max_include_depth {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "`#include` nesting too deep (limit: {})",
                    self.max_include_depth
                ))
                .span(hash.span.range())
                .label("including this file would exceed the include-depth limit"),
            );
            return;
        }

        let Some(resolved) = self.resolve_include(&header, is_system) else {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "cannot find header `{}` ({} include)",
                    header,
                    if is_system { "system" } else { "quote" }
                ))
                .span(hash.span.range())
                .label("no file matched any configured search path"),
            );
            return;
        };

        // Skip silently if `#pragma once` has retired this file.
        if self.pragma_once_files.contains(&resolved) {
            return;
        }

        // Circular-inclusion check: a file already being preprocessed
        // must not be entered again.
        if self
            .include_stack
            .iter()
            .any(|f| f.path.as_deref() == Some(resolved.as_path()))
        {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "circular `#include` detected while including `{}`",
                    resolved.display()
                ))
                .span(hash.span.range()),
            );
            return;
        }

        // Read and lex.
        let source = match std::fs::read_to_string(&resolved) {
            Ok(s) => s,
            Err(err) => {
                self.diagnostics.push(
                    Diagnostic::error(format!("failed to read `{}`: {err}", resolved.display()))
                        .span(hash.span.range()),
                );
                return;
            }
        };
        let mut lexer = Lexer::new(&source);
        let inner_tokens = lexer.tokenize();
        for d in lexer.take_diagnostics() {
            self.diagnostics.push(d);
        }

        // Include-guard detection: if the file is wrapped in a canonical
        // `#ifndef X / #define X / ... / #endif`, skip re-processing on
        // subsequent includes.  Only activates after the first pass.
        // (We do the parse-time snapshot here; re-inclusion is gated in
        // `pragma_once_files` or via `if_stack` behaviour downstream.)
        // The minimal, safe version: if we spot the guard pattern, act
        // as if `#pragma once` was applied once the include completes.
        let has_guard = detect_include_guard(&inner_tokens);

        // Push frame, update __FILE__ / __LINE__ context.  A `#line`
        // override set inside the included file must not leak back to
        // the parent, so save the current translation alongside.
        let saved_file = std::mem::replace(&mut self.current_file, resolved.display().to_string());
        let saved_line_starts =
            std::mem::replace(&mut self.line_starts, compute_line_starts(&source));
        let saved_line_offset = self.line_offset.take();
        let saved_file_override = self.file_override.take();
        let new_depth = current_depth + 1;
        self.include_stack
            .push(IncludeFrame::file(resolved.clone(), new_depth));

        // Strip the inner stream's trailing EOF — `drive()` swallows EOF
        // but we want to preserve the invariant that the stream we drive
        // starts with ordinary tokens only.
        let mut inner_tokens = inner_tokens;
        if matches!(inner_tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)) {
            inner_tokens.pop();
        }

        let mut inner_cursor = TokenCursor::new(wrap_tokens(inner_tokens));
        self.drive(&mut inner_cursor);

        // Restore.
        self.include_stack.pop();
        self.current_file = saved_file;
        self.line_starts = saved_line_starts;
        self.line_offset = saved_line_offset;
        self.file_override = saved_file_override;

        if has_guard {
            self.pragma_once_files.insert(resolved);
        }
    }

    /// Decode a `#include` argument (already stripped of its directive
    /// name and end-of-line) into `(header_name, is_system)`.
    ///
    /// Three forms are recognised, per C17 §6.10.2:
    ///
    /// * `"foo.h"`   — quote include, `is_system = false`.
    /// * `<foo.h>`   — system include, `is_system = true`.
    /// * any other token sequence is macro-expanded once and re-examined
    ///   in the two forms above (the "computed" include case).
    ///
    /// Returns `None` after recording a diagnostic when the argument is
    /// malformed.
    fn parse_include_argument(
        &mut self,
        tokens: &[PPToken],
        hash: &Token,
    ) -> Option<(String, bool)> {
        let filtered: Vec<PPToken> = tokens
            .iter()
            .filter(|t| !matches!(t.kind(), TokenKind::Eof))
            .cloned()
            .collect();

        if filtered.is_empty() {
            self.diagnostics.push(
                Diagnostic::error("`#include` expects `<filename>` or `\"filename\"`")
                    .span(hash.span.range()),
            );
            return None;
        }

        if let Some(parsed) = header_name_from_tokens(&filtered) {
            return Some(parsed);
        }

        // Computed: expand once and try again.
        let expanded = self.expand_tokens(filtered);
        if let Some(parsed) = header_name_from_tokens(&expanded) {
            return Some(parsed);
        }

        self.diagnostics.push(
            Diagnostic::error("`#include` expects `<filename>` or `\"filename\"`")
                .span(hash.span.range()),
        );
        None
    }

    /// Search the configured include paths for `header` and return the
    /// canonicalised location, or `None` if no path yields a readable
    /// file.  For quote includes, the directory of the *current* source
    /// file is tried first.
    fn resolve_include(&self, header: &str, is_system: bool) -> Option<PathBuf> {
        let header_path = Path::new(header);

        // Absolute path: only try the absolute location.  (GCC does the
        // same thing.)
        if header_path.is_absolute() {
            let candidate = header_path.to_path_buf();
            if candidate.is_file() {
                return canonicalise(&candidate);
            }
            return None;
        }

        // Quote includes: current-file directory first.
        if !is_system {
            if let Some(dir) = self.current_file_directory() {
                let candidate = dir.join(header_path);
                if candidate.is_file() {
                    return canonicalise(&candidate);
                }
            }
        }

        // Configured search paths — same order for both forms, only the
        // *additional* first-search differs.
        for base in &self.include_paths {
            let candidate = base.join(header_path);
            if candidate.is_file() {
                return canonicalise(&candidate);
            }
        }

        None
    }

    /// Directory of the current source file, if known.  Returns `None`
    /// for the synthetic top-level frame that carries only a display
    /// name.
    fn current_file_directory(&self) -> Option<PathBuf> {
        let frame = self.include_stack.last()?;
        let path = frame.path.as_ref()?;
        path.parent().map(|p| p.to_path_buf())
    }

    // -----------------------------------------------------------------
    // #pragma / #error / #warning / #line
    // -----------------------------------------------------------------

    /// Handle `#pragma`.  The pragma body is classified by its first
    /// token:
    ///
    /// * `once`                — retire the current file from future
    ///   `#include`s (already implemented in Prompt 2.5).
    /// * `message ( "text" )`  — emit the quoted string as a note.
    /// * anything else         — silently ignored per C17 §6.10.6/1
    ///   (unknown pragmas are implementation-defined; real-world code
    ///   uses `#pragma GCC diagnostic`, `#pragma pack`, `#pragma STDC`
    ///   and countless project-specific pragmas — warning on every one
    ///   would drown out genuine problems).
    fn handle_pragma(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let body = cursor.collect_to_end_of_line();
        self.process_pragma_body(hash.span, &body);
    }

    /// Dispatch the classified content of a `#pragma` (or `_Pragma`)
    /// body.  `anchor_span` is the span used for any note the pragma
    /// emits — the `#` of `#pragma`, or the `_Pragma` identifier of an
    /// in-stream operator invocation.
    fn process_pragma_body(&mut self, anchor_span: Span, body: &[PPToken]) {
        let filtered: Vec<&PPToken> = body
            .iter()
            .filter(|t| !matches!(t.kind(), TokenKind::Eof))
            .collect();

        let Some(first) = filtered.first() else {
            // Empty pragma body — nothing to do.
            return;
        };

        // `#pragma once`.
        if matches!(&first.token.kind, TokenKind::Identifier(s) if s == "once") {
            if let Some(frame) = self.include_stack.last() {
                if let Some(path) = &frame.path {
                    self.pragma_once_files.insert(path.clone());
                }
            }
            return;
        }

        // `#pragma message ( "text" )` — emit the string as a note.
        // Everything else — GCC diagnostics, visibility, pack, STDC,
        // project-specific — is silently ignored.
        if matches!(&first.token.kind, TokenKind::Identifier(s) if s == "message") {
            if let Some(text) = pragma_message_text(&filtered[1..]) {
                self.diagnostics.push(
                    Diagnostic::note_diag(format!("#pragma message: {text}"))
                        .span(anchor_span.range()),
                );
            }
        }
    }

    /// Handle `#error`.  Per C17 §6.10.5 the preprocessor-token
    /// arguments are emitted verbatim (no macro expansion) and
    /// compilation is marked as failed.  We do not stop processing —
    /// matching GCC and Clang, subsequent lines are still scanned so
    /// the user sees every preprocessor error in one pass.
    fn handle_error(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let body = cursor.collect_to_end_of_line();
        let message = diagnostic_message_from_tokens(&body);
        let text = if message.is_empty() {
            "#error".to_string()
        } else {
            format!("#error: {message}")
        };
        self.diagnostics
            .push(Diagnostic::error(text).span(hash.span.range()));
        self.has_errors = true;
    }

    /// Handle `#warning` — the GNU-extension companion to `#error`,
    /// also ratified in C23 as `#warning`.  Same argument handling as
    /// `#error`, but the diagnostic is a warning and the `has_errors`
    /// flag is left unchanged.
    fn handle_warning(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let body = cursor.collect_to_end_of_line();
        let message = diagnostic_message_from_tokens(&body);
        let text = if message.is_empty() {
            "#warning".to_string()
        } else {
            format!("#warning: {message}")
        };
        self.diagnostics
            .push(Diagnostic::warning(text).span(hash.span.range()));
    }

    /// Handle `#line NUMBER` and `#line NUMBER "FILENAME"`.
    ///
    /// Per C17 §6.10.4 the arguments are macro-expanded first, so
    /// `#line __LINE__` and `#line 100 __FILE__` are both valid.  After
    /// expansion we expect either:
    ///
    /// * a single [`TokenKind::IntegerLiteral`] naming a 1-based line
    ///   number in the range `[1, 2_147_483_647]`, optionally followed
    ///   by
    /// * a [`TokenKind::StringLiteral`] naming the filename to use in
    ///   subsequent `__FILE__` expansions and diagnostics.
    ///
    /// The effect is local to the current include frame: when a frame
    /// is popped, the enclosing file's overrides come back into view.
    fn handle_line(&mut self, hash: &Token, cursor: &mut TokenCursor) {
        let body = cursor.collect_to_end_of_line();
        let expanded = self.expand_tokens(body);
        let filtered: Vec<PPToken> = expanded
            .into_iter()
            .filter(|t| !matches!(t.kind(), TokenKind::Eof))
            .collect();

        if filtered.is_empty() {
            self.diagnostics.push(
                Diagnostic::error("`#line` requires at least a line-number argument")
                    .span(hash.span.range()),
            );
            return;
        }

        // Parse the line number.
        let number = match &filtered[0].token.kind {
            TokenKind::IntegerLiteral { value, .. } => *value,
            _ => {
                self.diagnostics.push(
                    Diagnostic::error("`#line` expects an integer line number")
                        .span(filtered[0].token.span.range())
                        .label("line number must be a positive integer literal"),
                );
                return;
            }
        };
        if number == 0 || number > 2_147_483_647 {
            self.diagnostics.push(
                Diagnostic::error(format!(
                    "invalid line number `{number}` in `#line` directive"
                ))
                .span(filtered[0].token.span.range())
                .label("line number must be in the range 1..=2147483647"),
            );
            return;
        }
        let reported = number as u32;

        // Parse the optional filename.
        let filename = match filtered.get(1) {
            None => None,
            Some(tok) => match &tok.token.kind {
                TokenKind::StringLiteral { value, .. } => Some(value.clone()),
                _ => {
                    self.diagnostics.push(
                        Diagnostic::error("`#line` filename argument must be a string literal")
                            .span(tok.token.span.range()),
                    );
                    return;
                }
            },
        };
        if filtered.len() > 2 {
            self.diagnostics.push(
                Diagnostic::error("extra tokens at end of `#line` directive")
                    .span(filtered[2].token.span.range()),
            );
            // Diagnose but still apply the line-number change — matches
            // GCC's "extra tokens" warning-and-continue behaviour.
        }

        // The directive's own line counts as physical line A; the NEXT
        // physical line should report as `reported`.  Anchor the
        // translation at `A + 1` so a token at physical line L >= A + 1
        // reports `reported + (L - (A + 1))`.
        let actual_directive_line = line_number_at(&self.line_starts, hash.span.start) as u32;
        let anchor_actual = actual_directive_line.saturating_add(1);
        self.line_offset = Some((anchor_actual, reported));
        if let Some(name) = filename {
            self.file_override = Some(name);
        }
    }
}

/// If `tok` is an identifier-shaped directive head, return its spelling.
///
/// Some C keywords (`if`, `else`) are *also* valid preprocessing
/// directive names, so we recognise them explicitly here.
fn directive_name_of(tok: &Token) -> Option<String> {
    match &tok.kind {
        TokenKind::Identifier(s) => Some(s.clone()),
        TokenKind::If => Some("if".to_string()),
        TokenKind::Else => Some("else".to_string()),
        _ => None,
    }
}

/// After macro expansion, any surviving identifier in a `#if` expression
/// is treated as `0` — C17 §6.10.1/4.
///
/// This also swallows function-call-shaped syntax whose head identifier
/// survived macro expansion: `IDENT(anything)` becomes a single `0`.
/// Strict C17 would instead error on such a construct (since `0(...)` is
/// not a valid expression), but real-world headers use Clang/GCC
/// builtins like `__has_include_next` or `__building_module` that we
/// cannot be expected to enumerate exhaustively — treating an unknown
/// builtin-looking call as always-false matches what GCC does and keeps
/// system headers parseable.
fn zero_remaining_identifiers(tokens: Vec<PPToken>) -> Vec<PPToken> {
    let mut out: Vec<PPToken> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let tok = &tokens[i];
        if matches!(&tok.token.kind, TokenKind::Identifier(_)) {
            // If the next non-space token is `(`, consume the entire
            // balanced call and emit a single `0` in its place.
            let next_is_paren = matches!(
                tokens.get(i + 1).map(|t| t.kind()),
                Some(TokenKind::LeftParen)
            );
            if next_is_paren {
                let mut j = i + 2;
                let mut depth = 1;
                while j < tokens.len() && depth > 0 {
                    match tokens[j].kind() {
                        TokenKind::LeftParen => depth += 1,
                        TokenKind::RightParen => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                out.push(PPToken::new(replacement_int_literal(&tok.token, 0)));
                i = j;
                continue;
            }
            out.push(PPToken::new(replacement_int_literal(&tok.token, 0)));
            i += 1;
        } else {
            out.push(tok.clone());
            i += 1;
        }
    }
    out
}

/// Build a suffix-less [`TokenKind::IntegerLiteral`] carrying `value`
/// while keeping the original token's span and spacing flags.  Used by
/// `defined` substitution and by the "unknown identifier becomes 0"
/// rewrite.
fn replacement_int_literal(original: &Token, value: u64) -> Token {
    Token {
        kind: TokenKind::IntegerLiteral {
            value,
            suffix: IntSuffix::None,
        },
        span: original.span,
        at_start_of_line: original.at_start_of_line,
        has_leading_space: original.has_leading_space,
    }
}

// ---------------------------------------------------------------------------
// Predefined-macro data tables
// ---------------------------------------------------------------------------

/// Simple object-like macros defined as `(name, body)`.  Bodies are
/// re-lexed at install time so each entry reads like a real `#define`.
const STANDARD_INT_MACROS: &[(&str, &str)] = &[
    // C17 §6.10.8 mandated macros.
    ("__STDC__", "1"),
    ("__STDC_VERSION__", "201710L"),
    ("__STDC_HOSTED__", "1"),
    ("__STDC_UTF_16__", "1"),
    ("__STDC_UTF_32__", "1"),
    // Type-size macros — system headers routinely test these.
    ("__CHAR_BIT__", "8"),
    ("__SIZEOF_POINTER__", "8"),
    ("__SIZEOF_SHORT__", "2"),
    ("__SIZEOF_INT__", "4"),
    ("__SIZEOF_LONG__", "8"),
    ("__SIZEOF_LONG_LONG__", "8"),
    ("__SIZEOF_FLOAT__", "4"),
    ("__SIZEOF_DOUBLE__", "8"),
    ("__SIZEOF_LONG_DOUBLE__", "16"),
    ("__SIZEOF_SIZE_T__", "8"),
    ("__SIZEOF_PTRDIFF_T__", "8"),
    ("__SIZEOF_WCHAR_T__", "4"),
    ("__SIZEOF_WINT_T__", "4"),
    // Integer limits — values chosen for LP64.
    ("__SCHAR_MAX__", "127"),
    ("__SHRT_MAX__", "32767"),
    ("__INT_MAX__", "2147483647"),
    ("__LONG_MAX__", "9223372036854775807L"),
    ("__LONG_LONG_MAX__", "9223372036854775807LL"),
    ("__INTMAX_MAX__", "9223372036854775807LL"),
    ("__UINTMAX_MAX__", "18446744073709551615ULL"),
    ("__SIZE_MAX__", "18446744073709551615UL"),
    ("__PTRDIFF_MAX__", "9223372036854775807L"),
    ("__WCHAR_MAX__", "2147483647"),
    ("__WINT_MAX__", "2147483647"),
    ("__INT8_MAX__", "127"),
    ("__INT16_MAX__", "32767"),
    ("__INT32_MAX__", "2147483647"),
    ("__INT64_MAX__", "9223372036854775807L"),
    ("__UINT8_MAX__", "255"),
    ("__UINT16_MAX__", "65535"),
    ("__UINT32_MAX__", "4294967295U"),
    ("__UINT64_MAX__", "18446744073709551615UL"),
    // LP64 data-model flags.
    ("__LP64__", "1"),
    ("_LP64", "1"),
    // Float limits — a minimal subset matching IEEE 754 double/float.
    ("__FLT_RADIX__", "2"),
    ("__FLT_MANT_DIG__", "24"),
    ("__FLT_DIG__", "6"),
    ("__DBL_MANT_DIG__", "53"),
    ("__DBL_DIG__", "15"),
    ("__LDBL_MANT_DIG__", "64"),
    ("__LDBL_DIG__", "18"),
];

/// Type macros whose bodies name a C type rather than an integer literal.
const STANDARD_TYPE_MACROS: &[(&str, &str)] = &[
    ("__INT8_TYPE__", "signed char"),
    ("__INT16_TYPE__", "short"),
    ("__INT32_TYPE__", "int"),
    ("__INT64_TYPE__", "long"),
    ("__UINT8_TYPE__", "unsigned char"),
    ("__UINT16_TYPE__", "unsigned short"),
    ("__UINT32_TYPE__", "unsigned int"),
    ("__UINT64_TYPE__", "unsigned long"),
    ("__INTMAX_TYPE__", "long long"),
    ("__UINTMAX_TYPE__", "unsigned long long"),
    ("__INTPTR_TYPE__", "long"),
    ("__UINTPTR_TYPE__", "unsigned long"),
    ("__SIZE_TYPE__", "unsigned long"),
    ("__PTRDIFF_TYPE__", "long"),
    ("__WCHAR_TYPE__", "int"),
    ("__WINT_TYPE__", "int"),
];

/// GCC-compatibility macros — values advertised by the compatibility
/// shim so system headers accept us as a GCC workalike.
const GCC_COMPAT_OBJECT_MACROS: &[(&str, &str)] = &[
    ("__GNUC__", "14"),
    ("__GNUC_MINOR__", "0"),
    ("__GNUC_PATCHLEVEL__", "0"),
    ("__GXX_ABI_VERSION", "1018"),
    ("__VERSION__", "\"Forge C 0.1\""),
];

/// Compute `(date, time)` strings in the shapes mandated by C17 for
/// `__DATE__` (`"Mmm DD YYYY"`) and `__TIME__` (`"HH:MM:SS"`), taken
/// once at preprocessor construction.
///
/// Implemented without any date-formatting crate: we carve the values
/// out of the seconds-since-epoch provided by [`std::time::SystemTime`].
fn date_time_strings() -> (String, String) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Seconds within the current UTC day.
    let day_secs = secs % 86_400;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    // Civil-from-days (Howard Hinnant's algorithm) to get year / month /
    // day out of whole days since 1970-01-01.  This is a well-known
    // fixed-size conversion: no external state, no time-zone logic.
    let days: i64 = (secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);

    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS
        .get((month as usize).wrapping_sub(1))
        .copied()
        .unwrap_or("Jan");
    let date = format!("{mon} {day:2} {year:04}");
    let time = format!("{hours:02}:{minutes:02}:{seconds:02}");
    (date, time)
}

/// Howard Hinnant's "civil from days" conversion: turn days since
/// 1970-01-01 into a `(year, month, day)` triple.  The math is
/// proleptic-Gregorian and correct for any `i64` day count.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

// ---------------------------------------------------------------------------
// Include-handling helpers
// ---------------------------------------------------------------------------

/// Recognise `<foo.h>` or `"foo.h"` in `tokens` (already filtered of
/// trailing `Eof`).  Returns `(header_name, is_system)` on success.
///
/// Accepts:
///
/// * A single [`TokenKind::StringLiteral`]: quote include, `false`.
/// * A sequence opening with `<` and closing with `>`: system include,
///   `true`.  Intermediate tokens are spelled back out and joined with
///   a single space when they carried leading whitespace.
fn header_name_from_tokens(tokens: &[PPToken]) -> Option<(String, bool)> {
    if tokens.is_empty() {
        return None;
    }

    // Quote form.
    if tokens.len() == 1 {
        if let TokenKind::StringLiteral { value, .. } = &tokens[0].token.kind {
            return Some((value.clone(), false));
        }
    }

    // Quote form allows trailing garbage to be ignored in practice; only
    // fire when the first token is a string literal and the remaining
    // tokens are all non-informative (GCC accepts this as an extension
    // but we keep the strict form for now).
    if let TokenKind::StringLiteral { value, .. } = &tokens[0].token.kind {
        return Some((value.clone(), false));
    }

    // System form.
    if matches!(tokens[0].kind(), TokenKind::Less) {
        let mut parts = String::new();
        let mut saw_close = false;
        for tok in &tokens[1..] {
            if matches!(tok.kind(), TokenKind::Greater) {
                saw_close = true;
                break;
            }
            if tok.has_leading_space() && !parts.is_empty() {
                parts.push(' ');
            }
            parts.push_str(&spelling_of(tok.kind()));
        }
        if saw_close {
            return Some((parts, true));
        }
    }

    None
}

/// Canonicalise `path`, falling back to the path itself when the host
/// filesystem refuses (e.g. when the include is resolved relative to a
/// path that exists but whose containing directory is inaccessible).
fn canonicalise(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path)
        .ok()
        .or_else(|| Some(path.to_path_buf()))
}

/// Compute the `line_starts` table for `source`: byte offsets at which
/// each line begins, in ascending order.  Index `0` is always present
/// (the start of the file); one entry per `\n`.
fn compute_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in source.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            let next = (i + 1) as u32;
            starts.push(next);
        }
    }
    starts
}

/// Turn a byte offset into a 1-based line number using the table that
/// [`compute_line_starts`] produced.  Offsets past the end of the file
/// resolve to the last recorded line.
fn line_number_at(line_starts: &[u32], offset: u32) -> u64 {
    // partition_point returns the number of entries that are `<= offset`,
    // which is exactly the 1-based line number we want.
    let idx = line_starts.partition_point(|&start| start <= offset);
    idx.max(1) as u64
}

/// `true` for the two magic macros whose "replacement list" must be
/// computed dynamically at expansion time instead of stored.
fn is_magic_name(name: &str) -> bool {
    matches!(name, "__FILE__" | "__LINE__")
}

/// Concatenate a line of preprocessing tokens into a human-readable
/// message, respecting `has_leading_space` as the signal to insert a
/// single separator between tokens.  Used by `#error` and `#warning`
/// to turn their unexpanded argument tokens into a message string;
/// C17 §6.10.5 is explicit that these tokens are **not** macro
/// expanded, so the output reproduces what the user actually wrote.
fn diagnostic_message_from_tokens(tokens: &[PPToken]) -> String {
    let mut out = String::new();
    let mut first = true;
    for tok in tokens {
        if matches!(tok.kind(), TokenKind::Eof) {
            continue;
        }
        if !first && tok.has_leading_space() {
            out.push(' ');
        }
        out.push_str(&spelling_of(tok.kind()));
        first = false;
    }
    out
}

/// Undo the escaping C17 §6.10.9/1 applies to the string-literal
/// argument of `_Pragma`: `\"` → `"` and `\\` → `\`.  The resulting
/// text is what the enclosed pragma would have looked like as a
/// source-level `#pragma` body.
///
/// No other escape sequences are recognised here — the lexer already
/// decoded them when it built the input [`TokenKind::StringLiteral`].
/// Only the two outermost layers of escaping added by the `_Pragma`
/// quotation step need to be reversed.
fn destringise(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                Some('"') => {
                    chars.next();
                    out.push('"');
                }
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Decode the argument to a `#pragma message` directive — `(
/// "text" )` — into the contained string.  Extra tokens after the
/// closing `)` are tolerated silently (matching GCC).  Returns `None`
/// if the argument does not take the expected shape, in which case
/// `process_pragma_body` treats the pragma as an unknown one and
/// silently ignores it.
fn pragma_message_text(rest: &[&PPToken]) -> Option<String> {
    let mut iter = rest.iter().copied();
    let open = iter.next()?;
    if !matches!(open.kind(), TokenKind::LeftParen) {
        return None;
    }
    let literal = iter.next()?;
    let value = match &literal.token.kind {
        TokenKind::StringLiteral { value, .. } => value.clone(),
        _ => return None,
    };
    let close = iter.next()?;
    if !matches!(close.kind(), TokenKind::RightParen) {
        return None;
    }
    Some(value)
}

/// Heuristically detect an include guard: a file whose only top-level
/// content is
///
/// ```c
/// #ifndef NAME
/// #define NAME
/// ...
/// #endif
/// ```
///
/// Returns `true` when the pattern is present and the first `#define`
/// matches the `#ifndef` name.  Used to enable silent skipping of
/// re-inclusion — effectively converting the guard into `#pragma once`
/// behaviour *after* the first successful pass.
fn detect_include_guard(tokens: &[Token]) -> bool {
    // Drop EOF for easier indexing.
    let tokens: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Eof))
        .collect();

    let mut i = 0;
    // Step over any leading Hash/define `#pragma once` pieces — but for
    // this helper we only require the canonical shape, so no tolerance.
    // First `#` + `ifndef` + IDENT on a line.
    if !matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Hash))
        || !tokens[i].at_start_of_line
    {
        return false;
    }
    i += 1;
    let is_ifndef = matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokenKind::Identifier(s)) if s == "ifndef"
    );
    if !is_ifndef {
        return false;
    }
    i += 1;
    let guard_name = match tokens.get(i).map(|t| &t.kind) {
        Some(TokenKind::Identifier(s)) => s.clone(),
        _ => return false,
    };
    i += 1;

    // Skip to the next Hash at start of line.
    while let Some(tok) = tokens.get(i) {
        if matches!(tok.kind, TokenKind::Hash) && tok.at_start_of_line {
            break;
        }
        i += 1;
    }

    // `#define GUARD_NAME`.
    if !matches!(tokens.get(i).map(|t| &t.kind), Some(TokenKind::Hash)) {
        return false;
    }
    i += 1;
    let is_define = matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokenKind::Identifier(s)) if s == "define"
    );
    if !is_define {
        return false;
    }
    i += 1;
    let matches_name = matches!(
        tokens.get(i).map(|t| &t.kind),
        Some(TokenKind::Identifier(s)) if *s == guard_name
    );
    if !matches_name {
        return false;
    }

    // Require the final non-Eof token to be the `endif`'s `#` directive.
    // Look for the *last* `#` at start-of-line and check it is followed
    // by `endif`.
    let mut last_hash = None;
    for (idx, tok) in tokens.iter().enumerate() {
        if matches!(tok.kind, TokenKind::Hash) && tok.at_start_of_line {
            last_hash = Some(idx);
        }
    }
    let Some(last) = last_hash else {
        return false;
    };
    // There must be an `endif` identifier immediately after that `#`.
    matches!(
        tokens.get(last + 1).map(|t| &t.kind),
        Some(TokenKind::Identifier(s)) if s == "endif"
    )
}

// ---------------------------------------------------------------------------
// Stand-alone entry point
// ---------------------------------------------------------------------------

/// Preprocess `tokens` with `config`, consuming all directives.
///
/// Returns the produced token stream on success, or every collected
/// [`Diagnostic`] on failure (at least one of which has
/// [`Severity::Error`]).
///
/// Callers that want to observe warnings *and* tokens together should
/// build a [`Preprocessor`] directly and pull diagnostics with
/// [`Preprocessor::take_diagnostics`].
pub fn preprocess(
    tokens: Vec<Token>,
    config: PreprocessConfig,
) -> Result<Vec<Token>, Vec<Diagnostic>> {
    let mut pp = Preprocessor::new(config);
    let output = pp.run(tokens);
    let had_error_directive = pp.has_errors();
    let diagnostics = pp.take_diagnostics();
    let has_error_diag = diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error));
    if had_error_directive || has_error_diag {
        Err(diagnostics)
    } else {
        Ok(output)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destringise_undoes_the_two_escapes_pragma_operator_requires() {
        assert_eq!(destringise(r#"foo"#), "foo");
        assert_eq!(destringise(r#"a\"b"#), r#"a"b"#);
        assert_eq!(destringise(r"a\\b"), r"a\b");
        assert_eq!(destringise(r#"\"\\"#), "\"\\");
    }
}
