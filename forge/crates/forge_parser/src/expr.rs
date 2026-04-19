//! Pratt expression parser for C17.
//!
//! All expression parsing methods are `impl Parser` — they live here
//! rather than in `parser.rs` only for file-size reasons.
//!
//! ## Binding-power table
//!
//! ```text
//! Level  Operators                              Assoc    BP (left, right)
//! ─────  ──────────────────────────────────────  ─────    ────────────────
//! 15     , (comma)                               Left     (2, 3)
//! 14     = += -= *= /= %= <<= >>= &= ^= |=     Right    (4, 3)
//! 13     ? : (ternary)                           Right    (6, 5)
//! 12     ||                                      Left     (8, 9)
//! 11     &&                                      Left     (10, 11)
//! 10     |                                       Left     (12, 13)
//!  9     ^                                       Left     (14, 15)
//!  8     &                                       Left     (16, 17)
//!  7     == !=                                   Left     (18, 19)
//!  6     < > <= >=                               Left     (20, 21)
//!  5     << >>                                   Left     (22, 23)
//!  4     + -                                     Left     (24, 25)
//!  3     * / %                                   Left     (26, 27)
//!  2     prefix: ++ -- & * + - ~ ! sizeof cast   Right    (_, 29)
//!  1     postfix: () [] . -> ++ --               Left     (31, _)
//! ```

use forge_lexer::{IntSuffix, Span, TokenKind};

use crate::ast::*;
use crate::ast_ops::*;
use crate::parser::Parser;

impl Parser {
    // =================================================================
    // Public entry points
    // =================================================================

    /// Parse a full expression including the comma operator.
    pub(crate) fn parse_expr(&mut self) -> Expr {
        self.parse_pratt(0)
    }

    /// Parse an assignment-expression (no comma).
    ///
    /// Used in initializers, function arguments, and anywhere the comma
    /// would be ambiguous (e.g. function-call arguments).
    pub(crate) fn parse_assignment_expr(&mut self) -> Expr {
        // Comma has left-bp 2, so min_bp=3 excludes it.
        self.parse_pratt(3)
    }

    /// Parse a constant expression (conditional-expression level).
    ///
    /// Used for `case`, enum values, bit-field widths, array sizes, and
    /// `_Static_assert`.  Semantically a constant — but at parse time
    /// the only difference is excluding comma and assignment.
    pub(crate) fn parse_constant_expr(&mut self) -> Expr {
        // Assignment has left-bp 4, so min_bp=5 excludes it.
        self.parse_pratt(5)
    }

    // =================================================================
    // Core Pratt loop
    // =================================================================

    /// Pratt parser core.  `min_bp` is the minimum binding power an
    /// infix/postfix operator must have for us to consume it.
    fn parse_pratt(&mut self, min_bp: u8) -> Expr {
        let mut lhs = self.parse_prefix();

        loop {
            // Postfix operators (bp 31) — highest precedence.
            if let Some(post_lhs) = self.try_postfix(&lhs) {
                lhs = post_lhs;
                continue;
            }

            // Infix / assignment / ternary / comma.
            let Some((left_bp, right_bp, op_kind)) = self.infix_bp() else {
                break;
            };

            if left_bp < min_bp {
                break;
            }

            lhs = self.parse_infix(lhs, right_bp, op_kind);
        }

        lhs
    }

    // =================================================================
    // Prefix (nud)
    // =================================================================

    /// Parse a prefix expression (nud in Pratt terminology).
    fn parse_prefix(&mut self) -> Expr {
        // GNU `__extension__` is a no-op marker permitted before any
        // expression; consume as many as appear.
        self.skip_extension_markers();

        // GNU `__builtin_*` intrinsics that take type-names as
        // arguments can't parse as generic function calls.  Intercept
        // them here.
        if let TokenKind::Identifier(name) = &self.peek().kind {
            if matches!(self.peek_ahead(1).kind, TokenKind::LeftParen) {
                match name.as_str() {
                    "__builtin_offsetof" => return self.parse_builtin_offsetof(),
                    "__builtin_types_compatible_p" => {
                        return self.parse_builtin_types_compatible_p()
                    }
                    "__builtin_choose_expr" => return self.parse_builtin_choose_expr(),
                    _ => {}
                }
            }
        }

        let tok = self.peek().clone();
        match &tok.kind {
            // --- Literals ---
            TokenKind::IntegerLiteral { value, suffix } => {
                let value = *value;
                let suffix = *suffix;
                self.advance();
                Expr::IntLiteral {
                    value,
                    suffix,
                    span: tok.span,
                    node_id: self.next_id(),
                }
            }

            TokenKind::FloatLiteral { value, suffix } => {
                let value = *value;
                let suffix = *suffix;
                self.advance();
                Expr::FloatLiteral {
                    value,
                    suffix,
                    span: tok.span,
                    node_id: self.next_id(),
                }
            }

            TokenKind::CharLiteral { value, prefix } => {
                let value = *value;
                let prefix = *prefix;
                self.advance();
                Expr::CharLiteral {
                    value,
                    prefix,
                    span: tok.span,
                    node_id: self.next_id(),
                }
            }

            TokenKind::StringLiteral { .. } => self.parse_string_literal(),

            // --- Identifier ---
            TokenKind::Identifier(_) => {
                let tok = self.advance();
                let name = match tok.kind {
                    TokenKind::Identifier(s) => s,
                    _ => unreachable!(),
                };
                Expr::Ident {
                    name,
                    span: tok.span,
                    node_id: self.next_id(),
                }
            }

            // --- Parenthesised / cast / compound literal ---
            TokenKind::LeftParen => self.parse_paren_expr(),

            // --- Unary prefix operators ---
            TokenKind::PlusPlus => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::PreIncrement,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::MinusMinus => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::PreDecrement,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Ampersand => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::AddrOf,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Star => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::Deref,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Plus => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::Plus,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Minus => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::Minus,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Tilde => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::BitNot,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }
            TokenKind::Bang => {
                self.advance();
                let operand = self.parse_pratt(29);
                Expr::UnaryOp {
                    op: UnaryOp::LogNot,
                    span: self.span_from(tok.span),
                    operand: Box::new(operand),
                    node_id: self.next_id(),
                }
            }

            // --- sizeof ---
            TokenKind::Sizeof => self.parse_sizeof(),

            // --- _Alignof ---
            TokenKind::Alignof => self.parse_alignof(),

            // --- _Generic ---
            TokenKind::Generic => self.parse_generic(),

            // --- Error recovery ---
            _ => {
                let tok = self.advance();
                self.error(
                    format!(
                        "expected expression, found `{}`",
                        crate::parser::kind_name(&tok.kind)
                    ),
                    tok.span,
                );
                // Return a dummy node so parsing can continue.
                Expr::IntLiteral {
                    value: 0,
                    suffix: IntSuffix::None,
                    span: tok.span,
                    node_id: self.next_id(),
                }
            }
        }
    }

    // =================================================================
    // Parenthesised / cast / compound literal
    // =================================================================

    /// Resolve the three-way `(` ambiguity: parenthesised expression,
    /// cast, or compound literal.
    fn parse_paren_expr(&mut self) -> Expr {
        let open = self.advance(); // consume `(`
        let start = open.span;

        // Check whether the first token inside looks like a type-name.
        if self.token_starts_type_name(&self.peek().kind) {
            let saved = self.save_state();

            if let Some(tn) = self.parse_type_name() {
                if self.at(&TokenKind::RightParen) {
                    self.advance(); // consume `)`

                    // `{` after `)` → compound literal.
                    if self.at(&TokenKind::LeftBrace) {
                        let init = self.parse_initializer_list();
                        return Expr::CompoundLiteral {
                            type_name: Box::new(tn),
                            initializer: init,
                            span: self.span_from(start),
                            node_id: self.next_id(),
                        };
                    }

                    // Otherwise → cast.
                    let operand = self.parse_pratt(29);
                    return Expr::Cast {
                        type_name: Box::new(tn),
                        expr: Box::new(operand),
                        span: self.span_from(start),
                        node_id: self.next_id(),
                    };
                }
            }

            // Type-name parse failed or no `)` → backtrack and treat as
            // parenthesised expression.  Restoring state also drops any
            // diagnostics emitted during the speculative parse.
            self.restore_state(saved);
        }

        // Parenthesised expression.
        let inner = self.parse_expr();
        let _ = self.expect(&TokenKind::RightParen);
        inner
    }

    // =================================================================
    // sizeof, _Alignof, _Generic
    // =================================================================

    /// Parse `sizeof(type)` or `sizeof expr`.
    fn parse_sizeof(&mut self) -> Expr {
        let kw = self.advance(); // consume `sizeof`
        let start = kw.span;

        // sizeof(TYPE) vs sizeof(expr) — check for `(` + type-name start.
        if self.at(&TokenKind::LeftParen) {
            let after_paren = &self.peek_ahead(1).kind;
            if self.token_starts_type_name(after_paren) {
                let saved = self.save_state();
                self.advance(); // consume `(`
                if let Some(tn) = self.parse_type_name() {
                    if self.at(&TokenKind::RightParen) {
                        self.advance(); // consume `)`
                        return Expr::SizeofType {
                            type_name: Box::new(tn),
                            span: self.span_from(start),
                            node_id: self.next_id(),
                        };
                    }
                }
                // Backtrack — it's sizeof(expr).
                self.restore_state(saved);
            }
        }

        let operand = self.parse_pratt(29);
        Expr::SizeofExpr {
            expr: Box::new(operand),
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    /// Parse `_Alignof(type-name)`.
    fn parse_alignof(&mut self) -> Expr {
        let kw = self.advance(); // consume `_Alignof`
        let start = kw.span;
        let _ = self.expect(&TokenKind::LeftParen);
        let tn = self.parse_type_name().unwrap_or_else(|| {
            self.error("expected type-name in `_Alignof`", self.peek().span);
            self.dummy_type_name()
        });
        let _ = self.expect(&TokenKind::RightParen);
        Expr::AlignofType {
            type_name: Box::new(tn),
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    /// Parse `_Generic(expr, type: expr, ..., default: expr)`.
    fn parse_generic(&mut self) -> Expr {
        let kw = self.advance(); // consume `_Generic`
        let start = kw.span;
        let _ = self.expect(&TokenKind::LeftParen);

        let controlling = self.parse_assignment_expr();
        let _ = self.expect(&TokenKind::Comma);

        let mut associations = Vec::new();
        loop {
            let assoc_start = self.peek().span;
            if self.eat(&TokenKind::Default).is_some() {
                let _ = self.expect(&TokenKind::Colon);
                let expr = self.parse_assignment_expr();
                associations.push(GenericAssociation {
                    type_name: None,
                    expr: Box::new(expr),
                    span: self.span_from(assoc_start),
                });
            } else {
                let tn = self.parse_type_name().unwrap_or_else(|| {
                    self.error(
                        "expected type-name in `_Generic` association",
                        self.peek().span,
                    );
                    self.dummy_type_name()
                });
                let _ = self.expect(&TokenKind::Colon);
                let expr = self.parse_assignment_expr();
                associations.push(GenericAssociation {
                    type_name: Some(tn),
                    expr: Box::new(expr),
                    span: self.span_from(assoc_start),
                });
            }

            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }

        let _ = self.expect(&TokenKind::RightParen);
        Expr::GenericSelection {
            controlling: Box::new(controlling),
            associations,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =================================================================
    // String literal with adjacent concatenation
    // =================================================================

    /// Parse one or more adjacent string literals, concatenating them.
    fn parse_string_literal(&mut self) -> Expr {
        let tok = self.advance();
        let start = tok.span;

        let (mut value, first_prefix) = match tok.kind {
            TokenKind::StringLiteral { value, prefix } => (value, prefix),
            _ => unreachable!(),
        };

        // Adjacent string concatenation.
        while let TokenKind::StringLiteral {
            value: ref next_val,
            ..
        } = self.peek().kind
        {
            value.push_str(next_val);
            self.advance();
        }

        Expr::StringLiteral {
            value,
            prefix: first_prefix,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =================================================================
    // Initializer list (for compound literals)
    // =================================================================

    /// Parse `{ init, init, ... }` (with optional trailing comma).
    pub(crate) fn parse_initializer_list(&mut self) -> Initializer {
        let open = self.advance(); // consume `{`
        let start = open.span;
        let mut items = Vec::new();

        while !self.at(&TokenKind::RightBrace) && !self.at_eof() {
            let item_start = self.peek().span;
            let mut designators = Vec::new();

            // Parse designators: .field or [index]
            loop {
                if self.at(&TokenKind::Dot) {
                    self.advance();
                    let name_tok = self.advance();
                    let name = match name_tok.kind {
                        TokenKind::Identifier(s) => s,
                        _ => {
                            self.error("expected field name after `.`", name_tok.span);
                            String::new()
                        }
                    };
                    designators.push(Designator::Field(name));
                } else if self.at(&TokenKind::LeftBracket) {
                    self.advance();
                    let idx = self.parse_constant_expr();
                    let _ = self.expect(&TokenKind::RightBracket);
                    designators.push(Designator::Index(Box::new(idx)));
                } else {
                    break;
                }
            }

            if !designators.is_empty() {
                let _ = self.expect(&TokenKind::Equal);
            }

            // Parse the initializer value (which may itself be a nested { ... }).
            let init = if self.at(&TokenKind::LeftBrace) {
                Box::new(self.parse_initializer_list())
            } else {
                Box::new(Initializer::Expr(Box::new(self.parse_assignment_expr())))
            };

            items.push(DesignatedInit {
                designators,
                initializer: init,
                span: self.span_from(item_start),
            });

            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }

        let _ = self.expect(&TokenKind::RightBrace);
        Initializer::List {
            items,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =================================================================
    // Postfix (postfix binding power = 31)
    // =================================================================

    /// Try to parse a postfix operator on `lhs`.  Returns `None` if the
    /// next token is not a postfix operator.
    fn try_postfix(&mut self, lhs: &Expr) -> Option<Expr> {
        let start = expr_span(lhs);
        match &self.peek().kind {
            // Function call
            TokenKind::LeftParen => {
                self.advance();
                let args = self.parse_argument_list();
                let _ = self.expect(&TokenKind::RightParen);
                Some(Expr::FunctionCall {
                    callee: Box::new(lhs.clone()),
                    args,
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            // Array subscript
            TokenKind::LeftBracket => {
                self.advance();
                let index = self.parse_expr();
                let _ = self.expect(&TokenKind::RightBracket);
                Some(Expr::ArraySubscript {
                    array: Box::new(lhs.clone()),
                    index: Box::new(index),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            // Member access: .field
            TokenKind::Dot => {
                self.advance();
                let member_tok = self.advance();
                let member = match member_tok.kind {
                    TokenKind::Identifier(s) => s,
                    _ => {
                        self.error("expected member name after `.`", member_tok.span);
                        String::new()
                    }
                };
                Some(Expr::MemberAccess {
                    object: Box::new(lhs.clone()),
                    member,
                    is_arrow: false,
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            // Member access: ->field
            TokenKind::Arrow => {
                self.advance();
                let member_tok = self.advance();
                let member = match member_tok.kind {
                    TokenKind::Identifier(s) => s,
                    _ => {
                        self.error("expected member name after `->`", member_tok.span);
                        String::new()
                    }
                };
                Some(Expr::MemberAccess {
                    object: Box::new(lhs.clone()),
                    member,
                    is_arrow: true,
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            // Post-increment
            TokenKind::PlusPlus => {
                self.advance();
                Some(Expr::PostfixOp {
                    op: PostfixOp::PostIncrement,
                    operand: Box::new(lhs.clone()),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            // Post-decrement
            TokenKind::MinusMinus => {
                self.advance();
                Some(Expr::PostfixOp {
                    op: PostfixOp::PostDecrement,
                    operand: Box::new(lhs.clone()),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                })
            }

            _ => None,
        }
    }

    /// Parse a comma-separated list of assignment-expressions (function
    /// call arguments).
    fn parse_argument_list(&mut self) -> Vec<Expr> {
        if self.at(&TokenKind::RightParen) {
            return Vec::new();
        }
        let mut args = vec![self.parse_assignment_expr()];
        while self.eat(&TokenKind::Comma).is_some() {
            args.push(self.parse_assignment_expr());
        }
        args
    }

    // =================================================================
    // Infix binding-power table
    // =================================================================

    /// Return `(left_bp, right_bp, discriminant)` for the current token
    /// if it is a valid infix/assignment/ternary/comma operator.
    /// Returns `None` for non-infix tokens.
    fn infix_bp(&self) -> Option<(u8, u8, InfixKind)> {
        let kind = &self.peek().kind;
        let result = match kind {
            // Comma (left-assoc)
            TokenKind::Comma => (2, 3, InfixKind::Comma),

            // Assignment (right-assoc)
            TokenKind::Equal => (4, 3, InfixKind::Assign(AssignOp::Assign)),
            TokenKind::PlusEqual => (4, 3, InfixKind::Assign(AssignOp::AddAssign)),
            TokenKind::MinusEqual => (4, 3, InfixKind::Assign(AssignOp::SubAssign)),
            TokenKind::StarEqual => (4, 3, InfixKind::Assign(AssignOp::MulAssign)),
            TokenKind::SlashEqual => (4, 3, InfixKind::Assign(AssignOp::DivAssign)),
            TokenKind::PercentEqual => (4, 3, InfixKind::Assign(AssignOp::ModAssign)),
            TokenKind::LessLessEqual => (4, 3, InfixKind::Assign(AssignOp::ShlAssign)),
            TokenKind::GreaterGreaterEqual => (4, 3, InfixKind::Assign(AssignOp::ShrAssign)),
            TokenKind::AmpEqual => (4, 3, InfixKind::Assign(AssignOp::BitAndAssign)),
            TokenKind::CaretEqual => (4, 3, InfixKind::Assign(AssignOp::BitXorAssign)),
            TokenKind::PipeEqual => (4, 3, InfixKind::Assign(AssignOp::BitOrAssign)),

            // Ternary (right-assoc)
            TokenKind::Question => (6, 5, InfixKind::Ternary),

            // Logical OR
            TokenKind::PipePipe => (8, 9, InfixKind::Binary(BinaryOp::LogOr)),
            // Logical AND
            TokenKind::AmpAmp => (10, 11, InfixKind::Binary(BinaryOp::LogAnd)),
            // Bitwise OR
            TokenKind::Pipe => (12, 13, InfixKind::Binary(BinaryOp::BitOr)),
            // Bitwise XOR
            TokenKind::Caret => (14, 15, InfixKind::Binary(BinaryOp::BitXor)),
            // Bitwise AND
            TokenKind::Ampersand => (16, 17, InfixKind::Binary(BinaryOp::BitAnd)),

            // Equality
            TokenKind::EqualEqual => (18, 19, InfixKind::Binary(BinaryOp::Eq)),
            TokenKind::BangEqual => (18, 19, InfixKind::Binary(BinaryOp::Ne)),

            // Relational
            TokenKind::Less => (20, 21, InfixKind::Binary(BinaryOp::Lt)),
            TokenKind::Greater => (20, 21, InfixKind::Binary(BinaryOp::Gt)),
            TokenKind::LessEqual => (20, 21, InfixKind::Binary(BinaryOp::Le)),
            TokenKind::GreaterEqual => (20, 21, InfixKind::Binary(BinaryOp::Ge)),

            // Shift
            TokenKind::LessLess => (22, 23, InfixKind::Binary(BinaryOp::Shl)),
            TokenKind::GreaterGreater => (22, 23, InfixKind::Binary(BinaryOp::Shr)),

            // Additive
            TokenKind::Plus => (24, 25, InfixKind::Binary(BinaryOp::Add)),
            TokenKind::Minus => (24, 25, InfixKind::Binary(BinaryOp::Sub)),

            // Multiplicative
            TokenKind::Star => (26, 27, InfixKind::Binary(BinaryOp::Mul)),
            TokenKind::Slash => (26, 27, InfixKind::Binary(BinaryOp::Div)),
            TokenKind::Percent => (26, 27, InfixKind::Binary(BinaryOp::Mod)),

            _ => return None,
        };
        Some(result)
    }

    /// Consume an infix operator and build the corresponding AST node.
    fn parse_infix(&mut self, lhs: Expr, right_bp: u8, op_kind: InfixKind) -> Expr {
        let start = expr_span(&lhs);
        self.advance(); // consume operator token

        match op_kind {
            InfixKind::Binary(op) => {
                let rhs = self.parse_pratt(right_bp);
                Expr::BinaryOp {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                }
            }

            InfixKind::Assign(op) => {
                let rhs = self.parse_pratt(right_bp);
                Expr::Assignment {
                    op,
                    target: Box::new(lhs),
                    value: Box::new(rhs),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                }
            }

            InfixKind::Ternary => {
                // C allows full expression (including comma) in the "then" position.
                let then_expr = self.parse_expr();
                let _ = self.expect(&TokenKind::Colon);
                let else_expr = self.parse_pratt(right_bp);
                Expr::Conditional {
                    condition: Box::new(lhs),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                    span: self.span_from(start),
                    node_id: self.next_id(),
                }
            }

            InfixKind::Comma => {
                let rhs = self.parse_pratt(right_bp);
                // Flatten left-nested commas: Comma([a, b]), c → Comma([a, b, c]).
                let mut exprs = match lhs {
                    Expr::Comma { exprs, .. } => exprs,
                    other => vec![other],
                };
                exprs.push(rhs);
                Expr::Comma {
                    span: self.span_from(start),
                    exprs,
                    node_id: self.next_id(),
                }
            }
        }
    }

    // =================================================================
    // GNU __builtin_* intrinsics
    // =================================================================

    /// Parse `__builtin_offsetof(type-name, member-designator)`.
    ///
    /// The designator is `ident ('.' ident | '[' expr ']')*`; the
    /// leading field is a bare identifier, not a dot-prefixed one.
    /// Subscript indices must be integer constant expressions — that
    /// requirement is enforced by sema, not here.
    ///
    /// *(Phase 4 follow-up: the prior implementation tolerantly
    /// consumed the designator and returned `IntLiteral(0)`; the real
    /// [`Expr::BuiltinOffsetof`] AST landed here so sema can compute
    /// the offset.)*
    fn parse_builtin_offsetof(&mut self) -> Expr {
        let kw = self.advance(); // `__builtin_offsetof`
        let start = kw.span;
        let _ = self.expect(&TokenKind::LeftParen);
        let ty = self.parse_type_name().unwrap_or_else(|| {
            self.error(
                "expected type-name in `__builtin_offsetof`",
                self.peek().span,
            );
            self.dummy_type_name()
        });
        let _ = self.expect(&TokenKind::Comma);

        let mut designator: Vec<OffsetofMember> = Vec::new();
        match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                designator.push(OffsetofMember::Field(name));
            }
            _ => {
                let span = self.peek().span;
                self.error("expected identifier in offsetof designator", span);
            }
        }

        loop {
            match &self.peek().kind {
                TokenKind::Dot => {
                    self.advance();
                    match &self.peek().kind {
                        TokenKind::Identifier(name) => {
                            let name = name.clone();
                            self.advance();
                            designator.push(OffsetofMember::Field(name));
                        }
                        _ => {
                            let span = self.peek().span;
                            self.error(
                                "expected identifier after '.' in offsetof designator",
                                span,
                            );
                            break;
                        }
                    }
                }
                TokenKind::LeftBracket => {
                    self.advance();
                    let idx = self.parse_expr();
                    let _ = self.expect(&TokenKind::RightBracket);
                    designator.push(OffsetofMember::Subscript(Box::new(idx)));
                }
                _ => break,
            }
        }

        let _ = self.expect(&TokenKind::RightParen);
        Expr::BuiltinOffsetof {
            ty: Box::new(ty),
            designator,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    /// Parse `__builtin_types_compatible_p(type-a, type-b)`.
    ///
    /// *(Phase 4 follow-up: the prior implementation discarded the
    /// parsed types and returned `IntLiteral(0)`; the dedicated
    /// [`Expr::BuiltinTypesCompatibleP`] AST lets sema perform the
    /// real compatibility check.)*
    fn parse_builtin_types_compatible_p(&mut self) -> Expr {
        let kw = self.advance(); // `__builtin_types_compatible_p`
        let start = kw.span;
        let _ = self.expect(&TokenKind::LeftParen);
        let t1 = self.parse_type_name().unwrap_or_else(|| {
            self.error(
                "expected first type-name in `__builtin_types_compatible_p`",
                self.peek().span,
            );
            self.dummy_type_name()
        });
        let _ = self.expect(&TokenKind::Comma);
        let t2 = self.parse_type_name().unwrap_or_else(|| {
            self.error(
                "expected second type-name in `__builtin_types_compatible_p`",
                self.peek().span,
            );
            self.dummy_type_name()
        });
        let _ = self.expect(&TokenKind::RightParen);
        Expr::BuiltinTypesCompatibleP {
            t1: Box::new(t1),
            t2: Box::new(t2),
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    /// Parse `__builtin_choose_expr(const-expr, expr, expr)`.
    ///
    /// All three arguments are full expressions (the first must be a
    /// constant, but syntactically it's just an expression).  We
    /// evaluate the const-expr at parse time as `0` (placeholder) and
    /// select the third argument — Phase 4+ may refine this.
    fn parse_builtin_choose_expr(&mut self) -> Expr {
        let kw = self.advance(); // `__builtin_choose_expr`
        let start = kw.span;
        let _ = self.expect(&TokenKind::LeftParen);
        let _ = self.parse_assignment_expr();
        let _ = self.expect(&TokenKind::Comma);
        let _ = self.parse_assignment_expr();
        let _ = self.expect(&TokenKind::Comma);
        let _ = self.parse_assignment_expr();
        let _ = self.expect(&TokenKind::RightParen);
        Expr::IntLiteral {
            value: 0,
            suffix: IntSuffix::None,
            span: self.span_from(start),
            node_id: self.next_id(),
        }
    }

    // =================================================================
    // Helpers
    // =================================================================

    /// Build a dummy `TypeName` for error recovery.
    pub(crate) fn dummy_type_name(&mut self) -> TypeName {
        let span = self.peek().span;
        let node_id = self.next_id();
        TypeName {
            specifiers: DeclSpecifiers {
                storage_class: None,
                type_specifiers: vec![TypeSpecifierToken::Int],
                type_qualifiers: Vec::new(),
                function_specifiers: Vec::new(),
                alignment: None,
                attributes: Vec::new(),
                span,
            },
            abstract_declarator: None,
            span,
            node_id,
        }
    }
}

// =====================================================================
// Infix kind — internal discriminant for the Pratt table
// =====================================================================

/// Discriminant used by [`Parser::infix_bp`] to tell the Pratt loop
/// which AST node to build.
enum InfixKind {
    Binary(BinaryOp),
    Assign(AssignOp),
    Ternary,
    Comma,
}

// =====================================================================
// Free helpers
// =====================================================================

/// Extract the span from an expression node (all variants carry one).
pub(crate) fn expr_span(expr: &Expr) -> Span {
    match expr {
        Expr::IntLiteral { span, .. }
        | Expr::FloatLiteral { span, .. }
        | Expr::CharLiteral { span, .. }
        | Expr::StringLiteral { span, .. }
        | Expr::Ident { span, .. }
        | Expr::BinaryOp { span, .. }
        | Expr::UnaryOp { span, .. }
        | Expr::PostfixOp { span, .. }
        | Expr::Conditional { span, .. }
        | Expr::Assignment { span, .. }
        | Expr::FunctionCall { span, .. }
        | Expr::MemberAccess { span, .. }
        | Expr::ArraySubscript { span, .. }
        | Expr::Cast { span, .. }
        | Expr::SizeofExpr { span, .. }
        | Expr::SizeofType { span, .. }
        | Expr::AlignofType { span, .. }
        | Expr::CompoundLiteral { span, .. }
        | Expr::GenericSelection { span, .. }
        | Expr::Comma { span, .. }
        | Expr::BuiltinOffsetof { span, .. }
        | Expr::BuiltinTypesCompatibleP { span, .. } => *span,
    }
}
