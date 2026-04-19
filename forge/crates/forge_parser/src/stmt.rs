//! Statement and translation-unit parsing (Prompt 3.5).
//!
//! Implements:
//!
//! * The statement dispatcher — compound, if/while/do/for/switch/case/
//!   default, jump statements (return/break/continue/goto), label, and
//!   expression statements (including the empty `;`).
//! * `parse_block_item`, which layers declarations, statements, and
//!   `_Static_assert` inside a compound statement.
//! * `parse_translation_unit` and `parse_external_declaration`, which
//!   distinguish function definitions from plain declarations by
//!   checking whether the first declarator is a function and the next
//!   token is `{`.
//! * `synchronize()` — panic-mode error recovery that skips to the next
//!   `;`, `{`, `}`, declaration start, or EOF.
//!
//! ## Scoping rules for the parser
//!
//! `parse_compound_statement` and `parse_for_statement` each push a
//! fresh typedef scope so that typedefs declared inside `{ ... }` or
//! inside `for (init;;)` shadow only within that scope.  The parser's
//! only concern is typedef tracking; value-scope checks happen in
//! sema.

use forge_lexer::TokenKind;

use crate::ast::*;
use crate::decl::declarator_name;
use crate::parser::{kind_name, Parser};

impl Parser {
    // =====================================================================
    // Compound statements and block items
    // =====================================================================

    /// Parse a `{ ... }` compound statement, pushing a fresh typedef
    /// scope for its duration.
    pub(crate) fn parse_compound_statement(&mut self) -> CompoundStmt {
        let start = self.peek().span;
        let _ = self.expect(&TokenKind::LeftBrace);
        self.push_scope();

        let mut items = Vec::new();
        while !self.at(&TokenKind::RightBrace) && !self.at_eof() {
            let before = self.cursor();
            items.push(self.parse_block_item());
            if self.cursor() == before {
                // Ensure forward progress on malformed input.
                self.synchronize();
            }
        }

        let _ = self.expect(&TokenKind::RightBrace);
        self.pop_scope();

        CompoundStmt {
            items,
            span: self.span_from(start),
        }
    }

    /// Parse a single block item: declaration, statement, or
    /// `_Static_assert`.
    ///
    /// A leading `__extension__` marker (GNU extension, no-op) is
    /// consumed before the block-item dispatch so that downstream
    /// classification sees the real first token.
    pub(crate) fn parse_block_item(&mut self) -> BlockItem {
        self.skip_extension_markers();
        if matches!(self.peek().kind, TokenKind::StaticAssert) {
            BlockItem::StaticAssert(self.parse_static_assert())
        } else if self.is_start_of_declaration() {
            BlockItem::Declaration(self.parse_declaration())
        } else {
            BlockItem::Statement(self.parse_statement())
        }
    }

    // =====================================================================
    // Statement dispatcher
    // =====================================================================

    /// Parse a single statement.
    pub(crate) fn parse_statement(&mut self) -> Stmt {
        // GNU `__extension__` is a no-op marker permitted before any
        // statement; consume as many as appear.
        self.skip_extension_markers();

        // GNU top-level `__asm__("...");` statement.
        //
        // The `__asm__` / `__asm` spellings are reserved for compiler
        // use, so seeing them at statement position is unambiguous —
        // accept them regardless of what follows and let
        // [`parse_asm_statement`] sort out modifiers (`__volatile__`,
        // `goto`, …) before the `(`.  The bare `asm` spelling could be
        // a user-chosen identifier, so restrict it to the immediate
        // `asm(` shape to avoid eating innocent `asm_foo()` calls.
        if self.at_asm_keyword() {
            let immediate_paren = matches!(self.peek_ahead(1).kind, TokenKind::LeftParen);
            let is_reserved_spelling = matches!(
                &self.peek().kind,
                TokenKind::Identifier(n) if n == "__asm__" || n == "__asm",
            );
            if immediate_paren || is_reserved_spelling {
                return self.parse_asm_statement();
            }
        }

        match &self.peek().kind {
            TokenKind::LeftBrace => Stmt::Compound(self.parse_compound_statement()),
            TokenKind::If => self.parse_if_statement(),
            TokenKind::While => self.parse_while_statement(),
            TokenKind::Do => self.parse_do_while_statement(),
            TokenKind::For => self.parse_for_statement(),
            TokenKind::Switch => self.parse_switch_statement(),
            TokenKind::Case => self.parse_case_statement(),
            TokenKind::Default => self.parse_default_statement(),
            TokenKind::Return => self.parse_return_statement(),
            TokenKind::Break => self.parse_break_statement(),
            TokenKind::Continue => self.parse_continue_statement(),
            TokenKind::Goto => self.parse_goto_statement(),
            TokenKind::Semicolon => {
                let start = self.peek().span;
                self.advance();
                Stmt::Expr {
                    expr: None,
                    span: self.span_from(start),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Identifier(_) if matches!(self.peek_ahead(1).kind, TokenKind::Colon) => {
                self.parse_label_statement()
            }
            _ => self.parse_expression_statement(),
        }
    }

    /// Consume any leading `__extension__` markers.  This is a no-op
    /// GNU marker used by glibc to silence pedantic warnings.
    pub(crate) fn skip_extension_markers(&mut self) {
        while matches!(
            &self.peek().kind,
            TokenKind::Identifier(n) if n == "__extension__"
        ) {
            self.advance();
        }
    }

    /// `true` if the current token is `__asm__`, `__asm`, or `asm`.
    pub(crate) fn at_asm_keyword(&self) -> bool {
        matches!(
            &self.peek().kind,
            TokenKind::Identifier(n) if matches!(n.as_str(), "__asm__" | "__asm" | "asm")
        )
    }

    /// Parse a GNU top-level `__asm__(...)` statement.
    ///
    /// We do not try to interpret the assembly — the entire balanced
    /// parenthesised body is consumed and dropped.  Returns a
    /// placeholder `Stmt::Expr { expr: None, .. }` so the AST keeps a
    /// record that *a* statement existed at this source location.
    ///
    /// Also tolerates `__volatile__` / `volatile` / `__inline__` /
    /// `goto` modifiers between the keyword and `(` (the flags GCC
    /// accepts on an `asm` statement).
    fn parse_asm_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `__asm__` / `__asm` / `asm`

        // Skip GCC asm-statement modifiers.
        loop {
            let is_mod = match &self.peek().kind {
                TokenKind::Volatile | TokenKind::Inline | TokenKind::Goto => true,
                TokenKind::Identifier(n) => matches!(
                    n.as_str(),
                    "__volatile__" | "__volatile" | "__inline__" | "__inline"
                ),
                _ => false,
            };
            if !is_mod {
                break;
            }
            self.advance();
        }

        if self.at(&TokenKind::LeftParen) {
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
                            break;
                        }
                    }
                    TokenKind::Eof => break,
                    _ => {
                        self.advance();
                    }
                }
            }
        }

        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Expr {
            expr: None,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =====================================================================
    // Control-flow statements
    // =====================================================================

    fn parse_if_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `if`
        let _ = self.expect(&TokenKind::LeftParen);
        let condition = Box::new(self.parse_expr());
        let _ = self.expect(&TokenKind::RightParen);
        let then_branch = Box::new(self.parse_statement());
        let else_branch = if self.eat(&TokenKind::Else).is_some() {
            Some(Box::new(self.parse_statement()))
        } else {
            None
        };
        Stmt::If {
            condition,
            then_branch,
            else_branch,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_while_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `while`
        let _ = self.expect(&TokenKind::LeftParen);
        let condition = Box::new(self.parse_expr());
        let _ = self.expect(&TokenKind::RightParen);
        let body = Box::new(self.parse_statement());
        Stmt::While {
            condition,
            body,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_do_while_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `do`
        let body = Box::new(self.parse_statement());
        let _ = self.expect(&TokenKind::While);
        let _ = self.expect(&TokenKind::LeftParen);
        let condition = Box::new(self.parse_expr());
        let _ = self.expect(&TokenKind::RightParen);
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::DoWhile {
            body,
            condition,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_for_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `for`
        let _ = self.expect(&TokenKind::LeftParen);

        // A single scope covers both the init-declaration and the body:
        // typedefs declared in the init are visible in the body only.
        self.push_scope();

        let init = if self.at(&TokenKind::Semicolon) {
            self.advance();
            None
        } else if self.is_start_of_declaration() {
            // parse_declaration consumes the trailing `;`.
            Some(ForInit::Declaration(self.parse_declaration()))
        } else {
            let e = self.parse_expr();
            let _ = self.expect(&TokenKind::Semicolon);
            Some(ForInit::Expr(Box::new(e)))
        };

        let condition = if self.at(&TokenKind::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expr()))
        };
        let _ = self.expect(&TokenKind::Semicolon);

        let update = if self.at(&TokenKind::RightParen) {
            None
        } else {
            Some(Box::new(self.parse_expr()))
        };
        let _ = self.expect(&TokenKind::RightParen);

        let body = Box::new(self.parse_statement());
        self.pop_scope();

        Stmt::For {
            init,
            condition,
            update,
            body,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_switch_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `switch`
        let _ = self.expect(&TokenKind::LeftParen);
        let expr = Box::new(self.parse_expr());
        let _ = self.expect(&TokenKind::RightParen);
        let body = Box::new(self.parse_statement());
        Stmt::Switch {
            expr,
            body,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_case_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `case`
        let value = Box::new(self.parse_constant_expr());
        let _ = self.expect(&TokenKind::Colon);
        let body = Box::new(self.parse_statement());
        Stmt::Case {
            value,
            body,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_default_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `default`
        let _ = self.expect(&TokenKind::Colon);
        let body = Box::new(self.parse_statement());
        Stmt::Default {
            body,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_return_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `return`
        let value = if self.at(&TokenKind::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expr()))
        };
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Return {
            value,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_break_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `break`
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Break {
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_continue_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `continue`
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Continue {
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_goto_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        self.advance(); // `goto`
        let label = match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let n = name.clone();
                self.advance();
                n
            }
            _ => {
                let tok = self.peek().clone();
                self.error(
                    format!(
                        "expected label after `goto`, found `{}`",
                        kind_name(&tok.kind)
                    ),
                    tok.span,
                );
                String::new()
            }
        };
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Goto {
            label,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_label_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        let name = match &self.peek().kind {
            TokenKind::Identifier(name) => name.clone(),
            _ => unreachable!("parse_statement dispatch ensures this is an identifier"),
        };
        self.advance(); // identifier
        let _ = self.expect(&TokenKind::Colon);
        let stmt = Box::new(self.parse_statement());
        Stmt::Label {
            name,
            stmt,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    fn parse_expression_statement(&mut self) -> Stmt {
        let start = self.peek().span;
        let expr = Box::new(self.parse_expr());
        let _ = self.expect(&TokenKind::Semicolon);
        Stmt::Expr {
            expr: Some(expr),
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =====================================================================
    // Translation unit
    // =====================================================================

    /// Parse a complete translation unit (the top-level entry point).
    ///
    /// Stray semicolons at file scope are permitted (and common in
    /// macro-heavy code).  On a malformed external declaration the
    /// parser calls [`Parser::synchronize`] to skip past the bad
    /// tokens and keep going.
    pub(crate) fn parse_translation_unit(&mut self) -> TranslationUnit {
        let start = self.peek().span;
        let mut declarations = Vec::new();

        while !self.at_eof() {
            if self.eat(&TokenKind::Semicolon).is_some() {
                continue;
            }
            let before = self.cursor();
            declarations.push(self.parse_external_declaration());
            if self.cursor() == before {
                self.synchronize();
                // `synchronize` returns without advancing when the peek
                // token is already a statement-boundary anchor (for
                // example a stray `{` or `}` at file scope).  If the
                // external-declaration attempt also failed to advance,
                // we would otherwise spin here forever — force progress
                // by consuming one token so the loop always terminates.
                if self.cursor() == before {
                    self.advance();
                }
            }
        }

        TranslationUnit {
            declarations,
            span: self.span_from(start),
        }
    }

    /// Parse a single external declaration.
    ///
    /// Distinguishes three cases:
    /// 1. `_Static_assert(cond, "msg");` at file scope.
    /// 2. A function definition — specifiers, a declarator whose
    ///    outermost direct part is a function declarator, followed by
    ///    `{ ... }`.
    /// 3. A regular declaration — everything else, ending in `;`.
    pub(crate) fn parse_external_declaration(&mut self) -> ExternalDeclaration {
        self.skip_extension_markers();
        if matches!(self.peek().kind, TokenKind::StaticAssert) {
            return ExternalDeclaration::StaticAssert(self.parse_static_assert());
        }

        let start = self.peek().span;
        let specifiers = self.parse_declaration_specifiers();
        let is_typedef = matches!(specifiers.storage_class, Some(StorageClass::Typedef));

        // Specifier-only declaration: `struct Foo { ... };`, `enum E;`.
        if self.at(&TokenKind::Semicolon) {
            self.advance();
            return ExternalDeclaration::Declaration(Declaration {
                specifiers,
                init_declarators: Vec::new(),
                span: self.span_from(start),
                node_id: self.next_id(),
            });
        }

        let decl_start = self.peek().span;
        let declarator = self.parse_declarator();

        // Function definition: outermost direct is a function declarator
        // and the next token opens a compound body.
        if self.at(&TokenKind::LeftBrace)
            && matches!(declarator.direct, DirectDeclarator::Function { .. })
        {
            let body = self.parse_compound_statement();
            return ExternalDeclaration::FunctionDef(FunctionDef {
                specifiers,
                declarator,
                body,
                span: self.span_from(start),
                node_id: self.next_id(),
            });
        }

        // Otherwise it's a plain declaration — register typedef names,
        // parse an optional initializer, then collect any further
        // init-declarators.
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
        self.skip_gnu_attributes();
        let mut init_declarators = vec![InitDeclarator {
            declarator,
            initializer,
            span: self.span_from(decl_start),
            node_id: self.next_id(),
        }];

        while self.eat(&TokenKind::Comma).is_some() {
            // Attributes after the comma, before the next declarator:
            // `int x, __attribute__((unused)) y;`.
            self.skip_gnu_attributes();
            let decl_start = self.peek().span;
            let declarator = self.parse_declarator();
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
            self.skip_gnu_attributes();
            init_declarators.push(InitDeclarator {
                declarator,
                initializer,
                span: self.span_from(decl_start),
                node_id: self.next_id(),
            });
        }

        let _ = self.expect(&TokenKind::Semicolon);

        ExternalDeclaration::Declaration(Declaration {
            specifiers,
            init_declarators,
            span: self.span_from(start),
            node_id: self.next_id(),
        })
    }

    // =====================================================================
    // Error recovery
    // =====================================================================

    /// Panic-mode recovery: skip tokens until we find a likely
    /// re-synchronization point.
    ///
    /// * `;` — consumed (the broken statement is behind us).
    /// * `{`, `}` — left in place (the enclosing compound will handle).
    /// * Any token that begins a declaration — left in place.
    /// * EOF — stop.
    pub(crate) fn synchronize(&mut self) {
        while !self.at_eof() {
            match &self.peek().kind {
                TokenKind::Semicolon => {
                    self.advance();
                    return;
                }
                TokenKind::LeftBrace | TokenKind::RightBrace => return,
                _ if self.is_start_of_declaration() => return,
                _ => {
                    self.advance();
                }
            }
        }
    }
}
