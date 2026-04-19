//! Declaration, declarator, type-name, and parameter-list parsing.
//!
//! This module implements Prompt 3.3 of the parser: declaration
//! specifiers, declarators (concrete + abstract), typedef tracking, and
//! the full `parse_type_name()` used by casts, `sizeof`, `_Alignof`,
//! `_Alignas`, `_Atomic(type-name)`, and `_Generic` associations.
//!
//! ## Declaration-specifier loop
//!
//! A declaration specifier is a free-order collection of
//! storage-classes, type specifiers, type qualifiers, function
//! specifiers, alignment specifiers, attributes, and *at most one*
//! typedef-name.  We collect them into a [`DeclSpecifiers`] and let
//! Phase 4 (sema) validate the combination.
//!
//! The classic edge case is `typedef int T; { T T; }` — the inner `T T`
//! means "variable named `T` of type `T`".  We track whether a
//! type-specifier has already been seen; if so, a subsequent identifier
//! is *not* treated as a typedef even if it currently resolves to one.
//!
//! ## Abstract vs concrete declarators
//!
//! Parameter lists allow either named (concrete) or unnamed (abstract)
//! declarators.  We decide which to parse by scanning ahead for a
//! non-typedef identifier — if one appears inside the declarator (at
//! any paren/bracket depth), it's concrete.  This mis-classifies a
//! handful of pathological nested-prototype cases (`int f(int (int x))`)
//! but those are vanishingly rare in real C.

use forge_lexer::TokenKind;

use crate::ast::*;
use crate::parser::{is_gnu_type_keyword, kind_name, Parser};

// =========================================================================
// GNU keyword dispatch
// =========================================================================

/// Classification of a GNU `__keyword__` identifier, for use inside the
/// declaration-specifier loop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GnuKeywordKind {
    NotGnu,
    Const,
    Volatile,
    Restrict,
    Inline,
    Signed,
    Extension,
    Attribute,
    Typeof,
    Asm,
    /// Compiler-extension type names (`_Float128`, `__int128`, …).
    ///
    /// Lexed as plain identifiers because they are not in the C17
    /// keyword set, but they appear in declaration-specifier positions
    /// inside glibc's system headers.  Treated like a typedef-name.
    ExtType,
}

fn gnu_keyword_kind(s: &str) -> GnuKeywordKind {
    match s {
        "__const" | "__const__" => GnuKeywordKind::Const,
        "__volatile" | "__volatile__" => GnuKeywordKind::Volatile,
        "__restrict" | "__restrict__" => GnuKeywordKind::Restrict,
        "__inline" | "__inline__" => GnuKeywordKind::Inline,
        "__signed" | "__signed__" => GnuKeywordKind::Signed,
        "__extension__" => GnuKeywordKind::Extension,
        "__attribute__" => GnuKeywordKind::Attribute,
        "__typeof__" | "__typeof" | "typeof" => GnuKeywordKind::Typeof,
        "__asm__" | "__asm" | "asm" => GnuKeywordKind::Asm,
        s if is_ext_type_name(s) => GnuKeywordKind::ExtType,
        _ => GnuKeywordKind::NotGnu,
    }
}

/// `true` if `s` names a compiler-extension scalar type that glibc and
/// system headers embed in declaration specifiers.  The parser treats
/// these as opaque typedef names — they have no dedicated AST variant.
fn is_ext_type_name(s: &str) -> bool {
    matches!(
        s,
        // ISO/IEC TS 18661-3 extended float types (used in glibc math.h)
        "_Float16"
            | "_Float32"
            | "_Float64"
            | "_Float128"
            | "_Float32x"
            | "_Float64x"
            | "_Float128x"
            // GCC / Clang __int128
            | "__int128"
            | "__int128_t"
            | "__uint128_t"
            // C decimal floating-point (TR 24732)
            | "_Decimal32"
            | "_Decimal64"
            | "_Decimal128"
    )
}

// =========================================================================
// Parser entry points
// =========================================================================

impl Parser {
    // ---------------------------------------------------------------------
    // Declaration specifiers
    // ---------------------------------------------------------------------

    /// Parse a `declaration-specifiers` sequence.
    ///
    /// Collects storage class, type specifiers, type qualifiers,
    /// function specifiers, alignment specifiers, and GNU attributes
    /// into a [`DeclSpecifiers`] struct.  Stops at the first token that
    /// is not a specifier or qualifier.
    ///
    /// **Typedef edge case** — once a type specifier has been seen,
    /// subsequent identifiers are treated as declarator names even if
    /// they currently resolve to a typedef.  This implements the
    /// `typedef int T; { T T; }` rule.
    pub(crate) fn parse_declaration_specifiers(&mut self) -> DeclSpecifiers {
        let start = self.peek().span;
        let mut storage_class: Option<StorageClass> = None;
        let mut type_specifiers: Vec<TypeSpecifierToken> = Vec::new();
        let mut type_qualifiers: Vec<TypeQualifier> = Vec::new();
        let mut function_specifiers: Vec<FunctionSpecifier> = Vec::new();
        let mut alignment: Option<AlignSpec> = None;
        let attributes: Vec<GnuAttribute> = Vec::new();
        let mut seen_type_specifier = false;

        loop {
            match &self.peek().kind {
                // --- Storage class keywords -----------------------------
                TokenKind::Auto => {
                    self.record_storage_class(&mut storage_class, StorageClass::Auto);
                }
                TokenKind::Register => {
                    self.record_storage_class(&mut storage_class, StorageClass::Register);
                }
                TokenKind::Static => {
                    self.record_storage_class(&mut storage_class, StorageClass::Static);
                }
                TokenKind::Extern => {
                    self.record_storage_class(&mut storage_class, StorageClass::Extern);
                }
                TokenKind::Typedef => {
                    self.record_storage_class(&mut storage_class, StorageClass::Typedef);
                }
                TokenKind::ThreadLocal => {
                    self.record_storage_class(&mut storage_class, StorageClass::ThreadLocal);
                }

                // --- Primitive type specifiers --------------------------
                TokenKind::Void => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Void);
                    seen_type_specifier = true;
                }
                TokenKind::Char => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Char);
                    seen_type_specifier = true;
                }
                TokenKind::Short => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Short);
                    seen_type_specifier = true;
                }
                TokenKind::Int => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Int);
                    seen_type_specifier = true;
                }
                TokenKind::Long => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Long);
                    seen_type_specifier = true;
                }
                TokenKind::Float => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Float);
                    seen_type_specifier = true;
                }
                TokenKind::Double => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Double);
                    seen_type_specifier = true;
                }
                TokenKind::Signed => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Signed);
                    seen_type_specifier = true;
                }
                TokenKind::Unsigned => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Unsigned);
                    seen_type_specifier = true;
                }
                TokenKind::Bool => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Bool);
                    seen_type_specifier = true;
                }
                TokenKind::Complex => {
                    self.advance();
                    type_specifiers.push(TypeSpecifierToken::Complex);
                    seen_type_specifier = true;
                }

                // --- Type qualifiers ------------------------------------
                TokenKind::Const => {
                    self.advance();
                    type_qualifiers.push(TypeQualifier::Const);
                }
                TokenKind::Volatile => {
                    self.advance();
                    type_qualifiers.push(TypeQualifier::Volatile);
                }
                TokenKind::Restrict => {
                    self.advance();
                    type_qualifiers.push(TypeQualifier::Restrict);
                }

                // --- _Atomic: specifier if `(` follows, else qualifier --
                TokenKind::Atomic => {
                    if matches!(self.peek_ahead(1).kind, TokenKind::LeftParen) {
                        self.advance(); // _Atomic
                        self.advance(); // (
                        let tn = self.parse_type_name().unwrap_or_else(|| {
                            self.error(
                                "expected type-name inside `_Atomic(...)`",
                                self.peek().span,
                            );
                            self.dummy_type_name()
                        });
                        let _ = self.expect(&TokenKind::RightParen);
                        type_specifiers.push(TypeSpecifierToken::Atomic(Box::new(tn)));
                        seen_type_specifier = true;
                    } else {
                        self.advance();
                        type_qualifiers.push(TypeQualifier::Atomic);
                    }
                }

                // --- Function specifiers --------------------------------
                TokenKind::Inline => {
                    self.advance();
                    function_specifiers.push(FunctionSpecifier::Inline);
                }
                TokenKind::Noreturn => {
                    self.advance();
                    function_specifiers.push(FunctionSpecifier::Noreturn);
                }

                // --- _Alignas -------------------------------------------
                TokenKind::Alignas => {
                    // Last-wins if user wrote multiple `_Alignas`.
                    alignment = Some(self.parse_alignas_specifier());
                }

                // --- struct / union / enum (3.4 replaces the stubs) -----
                TokenKind::Struct => {
                    let def = self.parse_struct_or_union_specifier(StructOrUnion::Struct);
                    type_specifiers.push(TypeSpecifierToken::Struct(def));
                    seen_type_specifier = true;
                }
                TokenKind::Union => {
                    let def = self.parse_struct_or_union_specifier(StructOrUnion::Union);
                    type_specifiers.push(TypeSpecifierToken::Union(def));
                    seen_type_specifier = true;
                }
                TokenKind::Enum => {
                    let def = self.parse_enum_specifier();
                    type_specifiers.push(TypeSpecifierToken::Enum(def));
                    seen_type_specifier = true;
                }

                // --- Identifiers: GNU keywords + typedef names ----------
                TokenKind::Identifier(name) => match gnu_keyword_kind(name) {
                    GnuKeywordKind::Const => {
                        self.advance();
                        type_qualifiers.push(TypeQualifier::Const);
                    }
                    GnuKeywordKind::Volatile => {
                        self.advance();
                        type_qualifiers.push(TypeQualifier::Volatile);
                    }
                    GnuKeywordKind::Restrict => {
                        self.advance();
                        type_qualifiers.push(TypeQualifier::Restrict);
                    }
                    GnuKeywordKind::Inline => {
                        self.advance();
                        function_specifiers.push(FunctionSpecifier::Inline);
                    }
                    GnuKeywordKind::Signed => {
                        self.advance();
                        type_specifiers.push(TypeSpecifierToken::Signed);
                        seen_type_specifier = true;
                    }
                    GnuKeywordKind::Extension => {
                        // `__extension__` is a no-op marker used by glibc
                        // to silence pedantic warnings.  Just drop it.
                        self.advance();
                    }
                    GnuKeywordKind::Attribute => {
                        self.skip_gnu_attributes();
                    }
                    GnuKeywordKind::Typeof => {
                        if seen_type_specifier {
                            // Already have a type specifier — a following
                            // typeof would be an error.  Break so the
                            // caller can treat it as a declarator name.
                            break;
                        }
                        let spec = self.parse_typeof_specifier();
                        type_specifiers.push(spec);
                        seen_type_specifier = true;
                    }
                    GnuKeywordKind::Asm => {
                        // `__asm__` is never a declaration specifier; it
                        // only appears as a label after a declarator.
                        break;
                    }
                    GnuKeywordKind::ExtType => {
                        // A compiler-extension scalar type (`_Float128`,
                        // `__int128`, …).  If we already saw a type
                        // specifier, break so it becomes the declarator
                        // name; otherwise consume it as a typedef-like
                        // opaque type.
                        if seen_type_specifier {
                            break;
                        }
                        let spelling = name.clone();
                        self.advance();
                        type_specifiers.push(TypeSpecifierToken::TypedefName(spelling));
                        seen_type_specifier = true;
                    }
                    GnuKeywordKind::NotGnu => {
                        // A plain identifier.  Treat as typedef name only
                        // if we have not already seen a type specifier.
                        if !seen_type_specifier && self.is_typedef(name) {
                            let name = name.clone();
                            self.advance();
                            type_specifiers.push(TypeSpecifierToken::TypedefName(name));
                            seen_type_specifier = true;
                        } else {
                            break;
                        }
                    }
                },

                _ => break,
            }
        }

        DeclSpecifiers {
            storage_class,
            type_specifiers,
            type_qualifiers,
            function_specifiers,
            alignment,
            attributes,
            span: self.span_from(start),
        }
    }

    /// Record a storage-class keyword, emitting a diagnostic if another
    /// storage class was already seen.
    fn record_storage_class(&mut self, slot: &mut Option<StorageClass>, sc: StorageClass) {
        let tok = self.advance();
        match *slot {
            None => *slot = Some(sc),
            Some(existing) if existing == sc => {
                self.error(
                    format!(
                        "duplicate storage class specifier `{}`",
                        kind_name(&tok.kind)
                    ),
                    tok.span,
                );
            }
            Some(_) => {
                self.error(
                    "conflicting storage class specifiers in declaration",
                    tok.span,
                );
            }
        }
    }

    /// Parse a GNU `__typeof__ ( ... )` specifier.
    ///
    /// The argument may be either a type-name or an arbitrary
    /// expression.  We try type-name first (with state rollback on
    /// failure) because the C grammar treats declaration-specifier
    /// contexts as prefer-type, matching GCC's resolution strategy.
    fn parse_typeof_specifier(&mut self) -> TypeSpecifierToken {
        self.advance(); // consume `typeof` / `__typeof__` / `__typeof`
        let _ = self.expect(&TokenKind::LeftParen);

        let saved = self.save_state();
        if let Some(tn) = self.parse_type_name() {
            if self.at(&TokenKind::RightParen) {
                self.advance();
                return TypeSpecifierToken::TypeofType(Box::new(tn));
            }
        }
        self.restore_state(saved);

        let expr = self.parse_expr();
        let _ = self.expect(&TokenKind::RightParen);
        TypeSpecifierToken::TypeofExpr(Box::new(expr))
    }

    /// Parse `_Alignas ( type-name )` or `_Alignas ( constant-expression )`.
    ///
    /// Prefers the type-name interpretation; falls back to an expression
    /// on failure (with full state rollback including any diagnostics
    /// emitted during the speculative type-name parse).  Always produces
    /// an `AlignSpec` — callers wrap in `Some` to distinguish "no
    /// `_Alignas` seen" from "`_Alignas` seen".
    fn parse_alignas_specifier(&mut self) -> AlignSpec {
        self.advance(); // consume `_Alignas`
        let _ = self.expect(&TokenKind::LeftParen);

        let saved = self.save_state();
        if let Some(tn) = self.parse_type_name() {
            if self.at(&TokenKind::RightParen) {
                self.advance();
                return AlignSpec::AlignAsType(Box::new(tn));
            }
        }
        self.restore_state(saved);

        let expr = self.parse_constant_expr();
        let _ = self.expect(&TokenKind::RightParen);
        AlignSpec::AlignAsExpr(Box::new(expr))
    }

    // ---------------------------------------------------------------------
    // struct / union
    // ---------------------------------------------------------------------

    /// Parse a `struct` or `union` specifier after the `struct`/`union`
    /// keyword:
    ///
    /// * `struct Foo { ... }` — named definition
    /// * `struct { ... }`     — anonymous definition
    /// * `struct Foo`         — forward reference / incomplete type
    ///
    /// GNU `__attribute__((...))` is tolerated before and after the tag
    /// and after the body (it's dropped until Prompt 3.6).
    fn parse_struct_or_union_specifier(&mut self, kind: StructOrUnion) -> StructDef {
        let start = self.peek().span;
        self.advance(); // consume `struct` or `union`

        self.skip_gnu_attributes();

        let name = self.parse_optional_tag_name();

        self.skip_gnu_attributes();

        let members = if self.at(&TokenKind::LeftBrace) {
            self.advance();
            let list = self.parse_struct_member_list();
            let _ = self.expect(&TokenKind::RightBrace);
            Some(list)
        } else {
            if name.is_none() {
                let kw = match kind {
                    StructOrUnion::Struct => "struct",
                    StructOrUnion::Union => "union",
                };
                self.error(
                    format!("expected tag name or `{{ ... }}` after `{kw}`"),
                    self.peek().span,
                );
            }
            None
        };

        self.skip_gnu_attributes();

        StructDef {
            kind,
            name,
            members,
            attributes: Vec::new(),
            span: self.span_from(start),
        }
    }

    /// Parse the member list of a struct/union body, already past the
    /// opening `{`.  Stops at the closing `}` (not consumed).
    ///
    /// GNU `__extension__` at the start of a member is ignored (glibc
    /// uses it inside anonymous-union members to silence pedantic
    /// warnings).  `__attribute__` is tolerated via
    /// [`parse_declaration_specifiers`].
    fn parse_struct_member_list(&mut self) -> Vec<StructMember> {
        let mut members = Vec::new();
        while !self.at(&TokenKind::RightBrace) && !self.at_eof() {
            // Accept a leading `__extension__` marker on the member.
            while matches!(
                &self.peek().kind,
                TokenKind::Identifier(n) if n == "__extension__"
            ) {
                self.advance();
            }

            if self.at(&TokenKind::StaticAssert) {
                let sa = self.parse_static_assert();
                members.push(StructMember::StaticAssert(sa));
                continue;
            }

            let field_start = self.peek().span;
            let pos_before = self.cursor();
            let specifiers = self.parse_declaration_specifiers();

            if self.cursor() == pos_before {
                // No progress — malformed token; skip it and retry.
                let tok = self.peek().clone();
                self.error(
                    format!(
                        "unexpected token `{}` in struct/union member list",
                        kind_name(&tok.kind)
                    ),
                    tok.span,
                );
                self.advance();
                continue;
            }

            let declarators = self.parse_struct_declarator_list();
            let _ = self.expect(&TokenKind::Semicolon);
            members.push(StructMember::Field(StructField {
                specifiers,
                declarators,
                span: self.span_from(field_start),
                node_id: self.next_id(),
            }));
        }
        members
    }

    /// Parse the comma-separated `struct-declarator-list` that follows
    /// specifiers inside a struct body.  Handles:
    ///
    /// * `x` → declarator=Some, bit_width=None
    /// * `x : N` → declarator=Some, bit_width=Some
    /// * `: N` → declarator=None, bit_width=Some (anonymous bit-field)
    /// * (immediate `;`) → returns an empty list (anonymous struct/union
    ///   member per C11)
    ///
    /// `__attribute__` is tolerated after each declarator (before the
    /// bit-width, before `,`, and before `;`).
    fn parse_struct_declarator_list(&mut self) -> Vec<StructFieldDeclarator> {
        let mut list = Vec::new();

        if self.at(&TokenKind::Semicolon) {
            return list;
        }

        loop {
            let sd_start = self.peek().span;

            let declarator = if self.at(&TokenKind::Colon) {
                None
            } else {
                Some(self.parse_declarator())
            };

            let bit_width = if self.eat(&TokenKind::Colon).is_some() {
                Some(Box::new(self.parse_constant_expr()))
            } else {
                None
            };

            // Attributes between the declarator/bit-width and the
            // separator: `int x __attribute__((packed)), y;`.
            self.skip_gnu_attributes();

            list.push(StructFieldDeclarator {
                declarator,
                bit_width,
                span: self.span_from(sd_start),
            });

            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        list
    }

    // ---------------------------------------------------------------------
    // enum
    // ---------------------------------------------------------------------

    /// Parse an `enum` specifier after the `enum` keyword:
    ///
    /// * `enum Color { RED, GREEN, BLUE }` — named definition
    /// * `enum { A, B, C }` — anonymous definition
    /// * `enum Color` — forward reference
    ///
    /// Trailing commas are accepted (C99+).  An empty body (`enum {}`)
    /// is diagnosed but still returns an empty enumerator list.
    fn parse_enum_specifier(&mut self) -> EnumDef {
        let start = self.peek().span;
        self.advance(); // consume `enum`

        self.skip_gnu_attributes();

        let name = self.parse_optional_tag_name();

        self.skip_gnu_attributes();

        let enumerators = if self.at(&TokenKind::LeftBrace) {
            let body_start = self.peek().span;
            self.advance();
            let list = self.parse_enumerator_list();
            let _ = self.expect(&TokenKind::RightBrace);
            if list.is_empty() {
                self.error(
                    "empty enumerator list is not allowed",
                    self.span_from(body_start),
                );
            }
            Some(list)
        } else {
            if name.is_none() {
                self.error(
                    "expected tag name or `{ ... }` after `enum`",
                    self.peek().span,
                );
            }
            None
        };

        self.skip_gnu_attributes();

        EnumDef {
            name,
            enumerators,
            attributes: Vec::new(),
            span: self.span_from(start),
        }
    }

    /// Parse the comma-separated list of enumerators between `{` and `}`.
    /// A trailing comma before `}` is allowed.
    fn parse_enumerator_list(&mut self) -> Vec<Enumerator> {
        let mut list = Vec::new();
        while !self.at(&TokenKind::RightBrace) && !self.at_eof() {
            let e_start = self.peek().span;

            let name = match &self.peek().kind {
                TokenKind::Identifier(n) => {
                    let n = n.clone();
                    self.advance();
                    n
                }
                _ => {
                    let tok = self.peek().clone();
                    self.error(
                        format!("expected enumerator name, found `{}`", kind_name(&tok.kind)),
                        tok.span,
                    );
                    // Recover: skip to next separator.
                    while !self.at(&TokenKind::Comma)
                        && !self.at(&TokenKind::RightBrace)
                        && !self.at_eof()
                    {
                        self.advance();
                    }
                    if self.eat(&TokenKind::Comma).is_some() {
                        continue;
                    }
                    break;
                }
            };

            self.skip_gnu_attributes();

            let value = if self.eat(&TokenKind::Equal).is_some() {
                Some(Box::new(self.parse_constant_expr()))
            } else {
                None
            };

            self.skip_gnu_attributes();

            list.push(Enumerator {
                name,
                value,
                attributes: Vec::new(),
                span: self.span_from(e_start),
            });

            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        list
    }

    /// Consume an identifier as a tag name iff it isn't a GNU keyword.
    /// Returns `None` when no identifier is present.
    fn parse_optional_tag_name(&mut self) -> Option<String> {
        if let TokenKind::Identifier(n) = &self.peek().kind {
            if is_gnu_type_keyword(n) {
                return None;
            }
            let n = n.clone();
            self.advance();
            return Some(n);
        }
        None
    }

    // ---------------------------------------------------------------------
    // _Static_assert
    // ---------------------------------------------------------------------

    /// Parse `_Static_assert ( constant-expression [ , string-literal ] ) ;`.
    ///
    /// C17 requires the string-literal; C23 makes it optional.  We
    /// accept either form.  Adjacent string literals are concatenated.
    pub(crate) fn parse_static_assert(&mut self) -> StaticAssert {
        let start = self.peek().span;
        self.advance(); // `_Static_assert`
        let _ = self.expect(&TokenKind::LeftParen);

        let condition = Box::new(self.parse_constant_expr());

        let message = if self.eat(&TokenKind::Comma).is_some() {
            let mut s = String::new();
            let mut seen_any = false;
            loop {
                let next_val = match &self.peek().kind {
                    TokenKind::StringLiteral { value, .. } => Some(value.clone()),
                    _ => None,
                };
                if let Some(v) = next_val {
                    s.push_str(&v);
                    self.advance();
                    seen_any = true;
                } else {
                    break;
                }
            }
            if !seen_any {
                let tok = self.peek().clone();
                self.error(
                    format!(
                        "expected string literal in `_Static_assert`, found `{}`",
                        kind_name(&tok.kind)
                    ),
                    tok.span,
                );
            }
            Some(s)
        } else {
            None
        };

        let _ = self.expect(&TokenKind::RightParen);
        let _ = self.expect(&TokenKind::Semicolon);

        StaticAssert {
            condition,
            message,
            span: self.span_from(start),
        }
    }

    // ---------------------------------------------------------------------
    // GNU declarator decorations (__attribute__ and __asm__ labels)
    // ---------------------------------------------------------------------

    /// Skip GNU `__attribute__((...))` lists and `__asm__(...)` labels.
    ///
    /// Both appear in the same syntactic slots around declarators and
    /// both are semantically inert from the parser's perspective, so we
    /// coalesce the skip into a single loop.  Attribute contents are
    /// dropped for now (Phase 4+ may parse them into [`GnuAttribute`]).
    ///
    /// Also tolerates the `asm`/`__asm` spellings of the assembler
    /// label, provided they are followed by `(` (so that a plain user
    /// identifier named `asm` outside declarator context is not
    /// erroneously consumed).
    pub(crate) fn skip_gnu_attributes(&mut self) {
        loop {
            match &self.peek().kind {
                TokenKind::Identifier(name) if name == "__attribute__" => {
                    self.advance();
                    self.skip_balanced_parens();
                }
                TokenKind::Identifier(name)
                    if matches!(name.as_str(), "__asm__" | "__asm" | "asm")
                        && matches!(self.peek_ahead(1).kind, TokenKind::LeftParen) =>
                {
                    self.advance();
                    self.skip_balanced_parens();
                }
                _ => return,
            }
        }
    }

    /// Consume a balanced parenthesised token group starting at the
    /// current `(`.  Does nothing if the cursor is not at `(`.
    fn skip_balanced_parens(&mut self) {
        if !self.at(&TokenKind::LeftParen) {
            return;
        }
        let mut depth = 0u32;
        loop {
            match &self.peek().kind {
                TokenKind::LeftParen => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RightParen => {
                    self.advance();
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                TokenKind::Eof => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Declarators
    // ---------------------------------------------------------------------

    /// Parse a `declarator`: zero or more pointer prefixes followed by a
    /// direct declarator.
    ///
    /// Tolerates trailing GNU decorations (`__attribute__`, `__asm__`
    /// labels) — these may appear after a declarator at any position
    /// where a declarator is used (declarations, struct members, etc.).
    pub(crate) fn parse_declarator(&mut self) -> Declarator {
        let start = self.peek().span;
        let pointers = self.parse_pointer_prefix();
        let direct = self.parse_direct_declarator();
        let decl = Declarator {
            pointers,
            direct,
            span: self.span_from(start),
        };
        self.skip_gnu_attributes();
        decl
    }

    /// Parse a `direct-declarator`: an identifier or parenthesised
    /// declarator followed by zero or more array/function suffixes.
    ///
    /// GNU `__attribute__` and `__asm__` labels are tolerated after
    /// every suffix so that declarations like `int foo(void)
    /// __attribute__((noreturn))` or `int arr[16]
    /// __attribute__((aligned))` parse cleanly.
    fn parse_direct_declarator(&mut self) -> DirectDeclarator {
        let mut current = self.parse_direct_declarator_base();
        loop {
            match &self.peek().kind {
                TokenKind::LeftBracket => {
                    let suffix_start = self.peek().span;
                    self.advance();
                    let (qualifiers, size, is_static) = self.parse_array_suffix_contents();
                    let _ = self.expect(&TokenKind::RightBracket);
                    current = DirectDeclarator::Array {
                        base: Box::new(current),
                        size,
                        qualifiers,
                        is_static,
                        span: self.span_from(suffix_start),
                    };
                    self.skip_gnu_attributes();
                }
                TokenKind::LeftParen => {
                    let suffix_start = self.peek().span;
                    self.advance();
                    let (params, is_variadic) = self.parse_parameter_list();
                    let _ = self.expect(&TokenKind::RightParen);
                    current = DirectDeclarator::Function {
                        base: Box::new(current),
                        params,
                        is_variadic,
                        span: self.span_from(suffix_start),
                    };
                    self.skip_gnu_attributes();
                }
                _ => break,
            }
        }
        current
    }

    /// Parse the base of a direct-declarator: either an identifier or a
    /// parenthesised sub-declarator.
    fn parse_direct_declarator_base(&mut self) -> DirectDeclarator {
        match &self.peek().kind {
            TokenKind::Identifier(_) => {
                let tok = self.advance();
                let (name, span) = match tok.kind {
                    TokenKind::Identifier(n) => (n, tok.span),
                    _ => unreachable!(),
                };
                DirectDeclarator::Identifier(name, span)
            }
            TokenKind::LeftParen => {
                self.advance();
                let inner = self.parse_declarator();
                let _ = self.expect(&TokenKind::RightParen);
                DirectDeclarator::Parenthesized(Box::new(inner))
            }
            _ => {
                let tok = self.peek().clone();
                self.error(
                    format!("expected declarator, found `{}`", kind_name(&tok.kind)),
                    tok.span,
                );
                DirectDeclarator::Identifier(String::new(), tok.span)
            }
        }
    }

    /// Parse zero or more pointer prefixes: `* const * volatile`, etc.
    ///
    /// Also accepts GNU equivalents (`__const`, `__volatile`,
    /// `__restrict`) and `__attribute__` (which is skipped).
    fn parse_pointer_prefix(&mut self) -> Vec<PointerQualifiers> {
        let mut pointers = Vec::new();
        while self.at(&TokenKind::Star) {
            self.advance();
            let mut qualifiers: Vec<TypeQualifier> = Vec::new();
            let attributes: Vec<GnuAttribute> = Vec::new();
            loop {
                match &self.peek().kind {
                    TokenKind::Const => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Const);
                    }
                    TokenKind::Volatile => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Volatile);
                    }
                    TokenKind::Restrict => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Restrict);
                    }
                    TokenKind::Atomic => {
                        // `_Atomic` as qualifier (never `_Atomic(...)` here).
                        self.advance();
                        qualifiers.push(TypeQualifier::Atomic);
                    }
                    TokenKind::Identifier(name) => match gnu_keyword_kind(name) {
                        GnuKeywordKind::Const => {
                            self.advance();
                            qualifiers.push(TypeQualifier::Const);
                        }
                        GnuKeywordKind::Volatile => {
                            self.advance();
                            qualifiers.push(TypeQualifier::Volatile);
                        }
                        GnuKeywordKind::Restrict => {
                            self.advance();
                            qualifiers.push(TypeQualifier::Restrict);
                        }
                        GnuKeywordKind::Attribute => {
                            self.skip_gnu_attributes();
                        }
                        _ => break,
                    },
                    _ => break,
                }
            }
            pointers.push(PointerQualifiers {
                qualifiers,
                attributes,
            });
        }
        pointers
    }

    /// Parse the contents of an array suffix `[...]` (after the `[`).
    ///
    /// Handles all legal combinations of `static`, type qualifiers, a
    /// size expression, a `*` VLA marker, or an empty `[]`.  Returns
    /// the qualifiers, the parsed [`ArraySize`], and whether `static`
    /// was present.
    fn parse_array_suffix_contents(&mut self) -> (Vec<TypeQualifier>, ArraySize, bool) {
        let mut qualifiers: Vec<TypeQualifier> = Vec::new();
        let mut is_static = false;

        loop {
            match &self.peek().kind {
                TokenKind::Static => {
                    if is_static {
                        let tok = self.advance();
                        self.error("duplicate `static` in array declarator", tok.span);
                    } else {
                        self.advance();
                        is_static = true;
                    }
                }
                TokenKind::Const => {
                    self.advance();
                    qualifiers.push(TypeQualifier::Const);
                }
                TokenKind::Volatile => {
                    self.advance();
                    qualifiers.push(TypeQualifier::Volatile);
                }
                TokenKind::Restrict => {
                    self.advance();
                    qualifiers.push(TypeQualifier::Restrict);
                }
                TokenKind::Identifier(name) => match gnu_keyword_kind(name) {
                    GnuKeywordKind::Const => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Const);
                    }
                    GnuKeywordKind::Volatile => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Volatile);
                    }
                    GnuKeywordKind::Restrict => {
                        self.advance();
                        qualifiers.push(TypeQualifier::Restrict);
                    }
                    _ => break,
                },
                _ => break,
            }
        }

        let size = if self.at(&TokenKind::RightBracket) {
            ArraySize::Unspecified
        } else if self.at(&TokenKind::Star)
            && matches!(self.peek_ahead(1).kind, TokenKind::RightBracket)
        {
            self.advance();
            ArraySize::VLAStar
        } else {
            ArraySize::Expr(Box::new(self.parse_assignment_expr()))
        };

        (qualifiers, size, is_static)
    }

    // ---------------------------------------------------------------------
    // Abstract declarators
    // ---------------------------------------------------------------------

    /// Parse an abstract declarator if one is present.  Returns `None`
    /// when the next tokens do not start an abstract declarator.
    pub(crate) fn parse_abstract_declarator_opt(&mut self) -> Option<AbstractDeclarator> {
        let start = self.peek().span;
        let pointers = self.parse_pointer_prefix();
        let direct = self.parse_direct_abstract_declarator();
        if pointers.is_empty() && direct.is_none() {
            return None;
        }
        Some(AbstractDeclarator {
            pointers,
            direct,
            span: self.span_from(start),
        })
    }

    /// Parse a direct-abstract-declarator, if one is present.
    ///
    /// The first `(` is ambiguous: it could start `(abstract-declarator)`
    /// or `(parameter-type-list)`.  We peek at the next token — `*`,
    /// `(`, or `[` means a parenthesised abstract declarator; anything
    /// else means a function-suffix.
    fn parse_direct_abstract_declarator(&mut self) -> Option<DirectAbstractDeclarator> {
        let mut current: Option<DirectAbstractDeclarator> = None;

        // Optional leading parenthesised abstract declarator.
        if self.at(&TokenKind::LeftParen) && self.looks_like_parenthesized_abstract_declarator() {
            self.advance(); // `(`
            if let Some(inner) = self.parse_abstract_declarator_opt() {
                let _ = self.expect(&TokenKind::RightParen);
                current = Some(DirectAbstractDeclarator::Parenthesized(Box::new(inner)));
            } else {
                // Heuristic said "parenthesised" but nothing parsed —
                // recover by consuming the `)` and moving on.
                let _ = self.expect(&TokenKind::RightParen);
            }
        }

        // Suffix loop.
        loop {
            match &self.peek().kind {
                TokenKind::LeftBracket => {
                    let suffix_start = self.peek().span;
                    self.advance();
                    let (_quals, size, _is_static) = self.parse_array_suffix_contents();
                    let _ = self.expect(&TokenKind::RightBracket);
                    current = Some(DirectAbstractDeclarator::Array {
                        base: current.map(Box::new),
                        size,
                        span: self.span_from(suffix_start),
                    });
                }
                TokenKind::LeftParen => {
                    let suffix_start = self.peek().span;
                    self.advance();
                    let (params, is_variadic) = self.parse_parameter_list();
                    let _ = self.expect(&TokenKind::RightParen);
                    current = Some(DirectAbstractDeclarator::Function {
                        base: current.map(Box::new),
                        params,
                        is_variadic,
                        span: self.span_from(suffix_start),
                    });
                }
                _ => break,
            }
        }

        current
    }

    /// Heuristic: when peek() is `(`, decide whether it starts
    /// `(abstract-declarator)` (true) or `(parameter-type-list?)`
    /// (false).  The inside of a parenthesised abstract declarator
    /// always begins with `*`, `(`, or `[`.
    fn looks_like_parenthesized_abstract_declarator(&self) -> bool {
        matches!(
            self.peek_ahead(1).kind,
            TokenKind::Star | TokenKind::LeftParen | TokenKind::LeftBracket
        )
    }

    // ---------------------------------------------------------------------
    // Parameter list
    // ---------------------------------------------------------------------

    /// Parse a parameter list starting just after `(`.
    ///
    /// Handles `()` (unspecified), `(void)` (explicitly none), and
    /// comma-separated parameter declarations with optional trailing
    /// `, ...` for variadics.  Returns `(params, is_variadic)`.
    pub(crate) fn parse_parameter_list(&mut self) -> (Vec<ParamDecl>, bool) {
        let mut params: Vec<ParamDecl> = Vec::new();
        let mut is_variadic = false;

        // `()` — empty parameter list (unspecified in C).
        if self.at(&TokenKind::RightParen) {
            return (params, is_variadic);
        }

        // `(void)` — explicitly no parameters.  Only treat as such when
        // `void` is followed immediately by `)` with no declarator.
        if matches!(self.peek().kind, TokenKind::Void)
            && matches!(self.peek_ahead(1).kind, TokenKind::RightParen)
        {
            self.advance();
            return (params, is_variadic);
        }

        loop {
            if self.at(&TokenKind::Ellipsis) {
                self.advance();
                is_variadic = true;
                break;
            }

            let param = self.parse_param_decl();
            params.push(param);

            if self.eat(&TokenKind::Comma).is_some() {
                if self.at(&TokenKind::Ellipsis) {
                    self.advance();
                    is_variadic = true;
                    break;
                }
                continue;
            }
            break;
        }

        (params, is_variadic)
    }

    /// Parse a single parameter declaration: specifiers then an
    /// optional declarator (concrete) or abstract-declarator.
    ///
    /// In the AST, abstract-declarators collapse to `declarator: None` —
    /// full abstract-declarator structure in parameters is deferred
    /// until a later phase.
    fn parse_param_decl(&mut self) -> ParamDecl {
        let start = self.peek().span;
        let specifiers = self.parse_declaration_specifiers();

        let declarator = if self.at(&TokenKind::RightParen) || self.at(&TokenKind::Comma) {
            None
        } else if self.looks_like_concrete_param_declarator() {
            Some(self.parse_declarator())
        } else {
            // Abstract declarator — parsed then discarded (3.3 limit).
            let _ = self.parse_abstract_declarator_opt();
            None
        };

        // Attributes on a parameter appear after the declarator:
        // `int f(int x __attribute__((unused)))` and
        // `int f(int __attribute__((unused)))` (abstract).
        self.skip_gnu_attributes();

        ParamDecl {
            specifiers,
            declarator,
            span: self.span_from(start),
        }
    }

    /// `true` if the upcoming declarator-shaped tokens contain a
    /// non-typedef identifier before the enclosing `)` or separating
    /// `,` — the heuristic for "this parameter has a name".
    fn looks_like_concrete_param_declarator(&self) -> bool {
        let mut pos = self.cursor();
        let mut depth: u32 = 0;
        loop {
            let Some(tok) = self.token_at(pos) else {
                return false;
            };
            match &tok.kind {
                TokenKind::Eof => return false,
                TokenKind::LeftParen | TokenKind::LeftBracket => depth += 1,
                TokenKind::RightParen | TokenKind::RightBracket => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Comma if depth == 0 => return false,
                TokenKind::Identifier(name)
                    if !self.is_typedef(name) && !is_gnu_type_keyword(name) =>
                {
                    return true;
                }
                _ => {}
            }
            pos += 1;
        }
    }

    // ---------------------------------------------------------------------
    // Full parse_type_name (replaces the 3.2 stub)
    // ---------------------------------------------------------------------

    /// Parse a `type-name`: declaration-specifiers plus an optional
    /// abstract-declarator.  Returns `None` when no specifiers were
    /// found (so the caller can retry as an expression).
    pub(crate) fn parse_type_name(&mut self) -> Option<TypeName> {
        let start = self.peek().span;
        let specifiers = self.parse_declaration_specifiers();

        // If absolutely nothing was collected, this wasn't a type-name.
        let nothing_seen = specifiers.type_specifiers.is_empty()
            && specifiers.type_qualifiers.is_empty()
            && specifiers.function_specifiers.is_empty()
            && specifiers.storage_class.is_none()
            && specifiers.alignment.is_none()
            && specifiers.attributes.is_empty();
        if nothing_seen {
            return None;
        }

        let abstract_declarator = self.parse_abstract_declarator_opt();

        Some(TypeName {
            specifiers,
            abstract_declarator,
            span: self.span_from(start),
            node_id: self.next_id(),
        })
    }

    // ---------------------------------------------------------------------
    // Declaration with init-declarator list and typedef tracking
    // ---------------------------------------------------------------------

    /// Parse a declaration: specifiers, optional init-declarator list,
    /// terminating `;`.
    ///
    /// Typedef names introduced by this declaration are added to the
    /// current typedef scope so that subsequent declarations can use
    /// them.  This uses [`declarator_name`] to find the declared
    /// identifier inside each declarator.
    pub(crate) fn parse_declaration(&mut self) -> Declaration {
        let start = self.peek().span;
        let specifiers = self.parse_declaration_specifiers();
        let mut init_declarators: Vec<InitDeclarator> = Vec::new();
        let is_typedef = matches!(specifiers.storage_class, Some(StorageClass::Typedef));

        // Specifier-only declaration: `struct foo;`, `enum bar;`, etc.
        if self.at(&TokenKind::Semicolon) {
            self.advance();
            return Declaration {
                specifiers,
                init_declarators,
                span: self.span_from(start),
                node_id: self.next_id(),
            };
        }

        loop {
            let decl_start = self.peek().span;
            let declarator = self.parse_declarator();

            // Register typedef names *before* the initializer so the
            // name is in scope for any self-reference.  Harmless for
            // non-typedef declarations because `is_typedef` is false.
            if is_typedef {
                if let Some(name) = declarator_name(&declarator) {
                    let name = name.to_string();
                    self.add_typedef(&name);
                }
            }

            let initializer = if self.eat(&TokenKind::Equal).is_some() {
                Some(self.parse_initializer())
            } else {
                None
            };

            // Attributes between this init-declarator and the next
            // comma or terminating semicolon.
            self.skip_gnu_attributes();

            init_declarators.push(InitDeclarator {
                declarator,
                initializer,
                span: self.span_from(decl_start),
                node_id: self.next_id(),
            });

            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }

            // Attributes after the comma, before the next declarator:
            // `int x, __attribute__((unused)) y;`.
            self.skip_gnu_attributes();
        }

        let _ = self.expect(&TokenKind::Semicolon);

        Declaration {
            specifiers,
            init_declarators,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    /// Parse an initializer: either a single assignment expression or a
    /// brace-enclosed list.
    pub(crate) fn parse_initializer(&mut self) -> Initializer {
        if self.at(&TokenKind::LeftBrace) {
            self.parse_initializer_list()
        } else {
            Initializer::Expr(Box::new(self.parse_assignment_expr()))
        }
    }

    // ---------------------------------------------------------------------
    // Declaration lookahead
    // ---------------------------------------------------------------------

    /// `true` if the current token begins a declaration.
    ///
    /// This resolves the declaration-vs-expression ambiguity in
    /// statements: `T * x;` is a declaration iff `T` is a typedef.
    pub(crate) fn is_start_of_declaration(&self) -> bool {
        match &self.peek().kind {
            TokenKind::Auto
            | TokenKind::Register
            | TokenKind::Static
            | TokenKind::Extern
            | TokenKind::Typedef
            | TokenKind::ThreadLocal
            | TokenKind::Void
            | TokenKind::Char
            | TokenKind::Short
            | TokenKind::Int
            | TokenKind::Long
            | TokenKind::Float
            | TokenKind::Double
            | TokenKind::Signed
            | TokenKind::Unsigned
            | TokenKind::Bool
            | TokenKind::Complex
            | TokenKind::Atomic
            | TokenKind::Struct
            | TokenKind::Union
            | TokenKind::Enum
            | TokenKind::Const
            | TokenKind::Volatile
            | TokenKind::Restrict
            | TokenKind::Inline
            | TokenKind::Noreturn
            | TokenKind::Alignas
            | TokenKind::StaticAssert => true,
            TokenKind::Identifier(name) => self.is_typedef(name) || is_gnu_type_keyword(name),
            _ => false,
        }
    }
}

// =========================================================================
// declarator_name — spiral-rule name extraction
// =========================================================================

/// Extract the declared identifier from a [`Declarator`], walking the
/// spiral-rule tree outward-in.  Returns `None` for a declarator with
/// no identifier (which shouldn't happen in concrete declarators, but
/// we tolerate it for error recovery).
pub(crate) fn declarator_name(d: &Declarator) -> Option<&str> {
    direct_declarator_name(&d.direct)
}

fn direct_declarator_name(d: &DirectDeclarator) -> Option<&str> {
    match d {
        DirectDeclarator::Identifier(name, _) if name.is_empty() => None,
        DirectDeclarator::Identifier(name, _) => Some(name.as_str()),
        DirectDeclarator::Parenthesized(inner) => declarator_name(inner),
        DirectDeclarator::Array { base, .. } | DirectDeclarator::Function { base, .. } => {
            direct_declarator_name(base)
        }
    }
}
