//! AST pretty-printer.
//!
//! Produces an indented tree dump of a [`TranslationUnit`] that is
//! human-readable and diffable.  The format is not stable across
//! compiler versions — it is a debugging tool, not a serialization
//! format.  Two-space indentation; one node per line; Debug-format
//! leaves for enum values (`Unsigned`, `Long`, ...).
//!
//! The printer is the canonical way to verify parser output in tests —
//! structural assertions against `Debug` are brittle, but stable
//! printed trees can be snapshot-tested.

use crate::ast::*;
use crate::ast_ops::*;

/// Render a translation unit as an indented tree string.
pub fn print_ast(tu: &TranslationUnit) -> String {
    let mut p = Printer::default();
    p.translation_unit(tu);
    p.buf
}

// =========================================================================
// Printer state
// =========================================================================

#[derive(Default)]
struct Printer {
    buf: String,
    depth: usize,
}

impl Printer {
    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.buf.push_str("  ");
        }
        self.buf.push_str(s);
        self.buf.push('\n');
    }

    fn with_indent<F>(&mut self, f: F)
    where
        F: FnOnce(&mut Self),
    {
        self.depth += 1;
        f(self);
        self.depth -= 1;
    }

    // ---------------------------------------------------------------------
    // Translation unit
    // ---------------------------------------------------------------------

    fn translation_unit(&mut self, tu: &TranslationUnit) {
        self.line("TranslationUnit");
        self.with_indent(|p| {
            for decl in &tu.declarations {
                p.external_declaration(decl);
            }
        });
    }

    fn external_declaration(&mut self, decl: &ExternalDeclaration) {
        match decl {
            ExternalDeclaration::FunctionDef(f) => self.function_def(f),
            ExternalDeclaration::Declaration(d) => self.declaration(d),
            ExternalDeclaration::StaticAssert(s) => self.static_assert(s),
        }
    }

    // ---------------------------------------------------------------------
    // Function definitions
    // ---------------------------------------------------------------------

    fn function_def(&mut self, f: &FunctionDef) {
        self.line("FunctionDef");
        self.with_indent(|p| {
            p.specifiers("Specifiers", &f.specifiers);
            p.declarator(&f.declarator);
            p.line("Body:");
            p.with_indent(|p| p.compound_stmt(&f.body));
        });
    }

    // ---------------------------------------------------------------------
    // Declarations
    // ---------------------------------------------------------------------

    fn declaration(&mut self, d: &Declaration) {
        self.line("Declaration");
        self.with_indent(|p| {
            p.specifiers("Specifiers", &d.specifiers);
            if d.init_declarators.is_empty() {
                p.line("(no declarators)");
            } else {
                for id in &d.init_declarators {
                    p.init_declarator(id);
                }
            }
        });
    }

    fn init_declarator(&mut self, id: &InitDeclarator) {
        self.line("InitDeclarator");
        self.with_indent(|p| {
            p.declarator(&id.declarator);
            if let Some(init) = &id.initializer {
                p.line("Initializer:");
                p.with_indent(|p| p.initializer(init));
            }
        });
    }

    fn static_assert(&mut self, s: &StaticAssert) {
        match &s.message {
            Some(m) => self.line(&format!("StaticAssert message={m:?}")),
            None => self.line("StaticAssert"),
        }
        self.with_indent(|p| {
            p.line("Condition:");
            p.with_indent(|p| p.expr(&s.condition));
        });
    }

    // ---------------------------------------------------------------------
    // Specifiers
    // ---------------------------------------------------------------------

    fn specifiers(&mut self, label: &str, specs: &DeclSpecifiers) {
        let mut parts: Vec<String> = Vec::new();
        if let Some(sc) = specs.storage_class {
            parts.push(format!("{sc:?}"));
        }
        for q in &specs.type_qualifiers {
            parts.push(format!("{q:?}"));
        }
        for fs in &specs.function_specifiers {
            parts.push(format!("{fs:?}"));
        }
        for ts in &specs.type_specifiers {
            parts.push(type_specifier_name(ts));
        }
        let summary = parts.join(", ");
        if summary.is_empty() {
            self.line(&format!("{label} []"));
        } else {
            self.line(&format!("{label} [{summary}]"));
        }

        // Compound specifiers — struct, union, enum, typeof — get
        // nested structure underneath.
        self.with_indent(|p| {
            for ts in &specs.type_specifiers {
                p.type_specifier_details(ts);
            }
            if let Some(a) = &specs.alignment {
                p.line("Alignment:");
                p.with_indent(|p| p.align_spec(a));
            }
        });
    }

    fn type_specifier_details(&mut self, ts: &TypeSpecifierToken) {
        match ts {
            TypeSpecifierToken::Struct(s) | TypeSpecifierToken::Union(s) => {
                self.struct_def(s);
            }
            TypeSpecifierToken::Enum(e) => self.enum_def(e),
            TypeSpecifierToken::Atomic(tn) => {
                self.line("_Atomic:");
                self.with_indent(|p| p.type_name(tn));
            }
            TypeSpecifierToken::TypeofExpr(e) => {
                self.line("__typeof__ expr:");
                self.with_indent(|p| p.expr(e));
            }
            TypeSpecifierToken::TypeofType(tn) => {
                self.line("__typeof__ type:");
                self.with_indent(|p| p.type_name(tn));
            }
            _ => {}
        }
    }

    fn align_spec(&mut self, a: &AlignSpec) {
        match a {
            AlignSpec::AlignAsType(tn) => {
                self.line("_Alignas type:");
                self.with_indent(|p| p.type_name(tn));
            }
            AlignSpec::AlignAsExpr(e) => {
                self.line("_Alignas expr:");
                self.with_indent(|p| p.expr(e));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Declarators
    // ---------------------------------------------------------------------

    fn declarator(&mut self, d: &Declarator) {
        let name = crate::decl::declarator_name(d).unwrap_or("<anonymous>");
        let depth = d.pointers.len();
        if depth == 0 {
            self.line(&format!("Declarator: {name}"));
        } else {
            self.line(&format!("Declarator: {name} (pointer_depth={depth})"));
        }
        self.with_indent(|p| {
            for (i, pq) in d.pointers.iter().enumerate() {
                let quals: Vec<String> = pq.qualifiers.iter().map(|q| format!("{q:?}")).collect();
                let summary = if quals.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", quals.join(", "))
                };
                p.line(&format!("Pointer #{i}{summary}"));
            }
            p.direct_declarator(&d.direct);
        });
    }

    fn direct_declarator(&mut self, d: &DirectDeclarator) {
        match d {
            DirectDeclarator::Identifier(name, _) => {
                self.line(&format!("Identifier: {name}"));
            }
            DirectDeclarator::Parenthesized(inner) => {
                self.line("Parenthesized:");
                self.with_indent(|p| p.declarator(inner));
            }
            DirectDeclarator::Array {
                base,
                size,
                qualifiers,
                is_static,
                ..
            } => {
                let quals: Vec<String> = qualifiers.iter().map(|q| format!("{q:?}")).collect();
                let static_part = if *is_static { " static" } else { "" };
                let quals_part = if quals.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", quals.join(", "))
                };
                self.line(&format!("Array{static_part}{quals_part}"));
                self.with_indent(|p| {
                    p.line("Size:");
                    p.with_indent(|p| p.array_size(size));
                    p.line("Base:");
                    p.with_indent(|p| p.direct_declarator(base));
                });
            }
            DirectDeclarator::Function {
                base,
                params,
                is_variadic,
                ..
            } => {
                let variadic_part = if *is_variadic { " (variadic)" } else { "" };
                self.line(&format!(
                    "Function{variadic_part} ({} param(s))",
                    params.len()
                ));
                self.with_indent(|p| {
                    p.line("Params:");
                    p.with_indent(|p| {
                        if params.is_empty() {
                            p.line("(none)");
                        } else {
                            for param in params {
                                p.param_decl(param);
                            }
                        }
                    });
                    p.line("Base:");
                    p.with_indent(|p| p.direct_declarator(base));
                });
            }
        }
    }

    fn array_size(&mut self, size: &ArraySize) {
        match size {
            ArraySize::Unspecified => self.line("Unspecified"),
            ArraySize::Expr(e) => self.expr(e),
            ArraySize::VLAStar => self.line("VLAStar"),
        }
    }

    fn param_decl(&mut self, p: &ParamDecl) {
        self.line("ParamDecl");
        self.with_indent(|pp| {
            pp.specifiers("Specifiers", &p.specifiers);
            match &p.declarator {
                Some(d) => pp.declarator(d),
                None => pp.line("(abstract)"),
            }
        });
    }

    // ---------------------------------------------------------------------
    // Struct / union / enum
    // ---------------------------------------------------------------------

    fn struct_def(&mut self, s: &StructDef) {
        let kw = match s.kind {
            StructOrUnion::Struct => "struct",
            StructOrUnion::Union => "union",
        };
        let tag = s.name.as_deref().unwrap_or("<anonymous>");
        match &s.members {
            Some(members) => {
                self.line(&format!("{kw} {tag} ({} member(s))", members.len()));
                self.with_indent(|p| {
                    for m in members {
                        p.struct_member(m);
                    }
                });
            }
            None => {
                self.line(&format!("{kw} {tag} (incomplete)"));
            }
        }
    }

    fn struct_member(&mut self, m: &StructMember) {
        match m {
            StructMember::Field(f) => {
                self.line("Field");
                self.with_indent(|p| {
                    p.specifiers("Specifiers", &f.specifiers);
                    for d in &f.declarators {
                        p.struct_field_declarator(d);
                    }
                });
            }
            StructMember::StaticAssert(s) => self.static_assert(s),
        }
    }

    fn struct_field_declarator(&mut self, d: &StructFieldDeclarator) {
        self.line("FieldDeclarator");
        self.with_indent(|p| {
            match &d.declarator {
                Some(dec) => p.declarator(dec),
                None => p.line("(anonymous bit-field)"),
            }
            if let Some(bw) = &d.bit_width {
                p.line("BitWidth:");
                p.with_indent(|p| p.expr(bw));
            }
        });
    }

    fn enum_def(&mut self, e: &EnumDef) {
        let tag = e.name.as_deref().unwrap_or("<anonymous>");
        match &e.enumerators {
            Some(list) => {
                self.line(&format!("enum {tag} ({} enumerator(s))", list.len()));
                self.with_indent(|p| {
                    for en in list {
                        p.enumerator(en);
                    }
                });
            }
            None => {
                self.line(&format!("enum {tag} (incomplete)"));
            }
        }
    }

    fn enumerator(&mut self, e: &Enumerator) {
        self.line(&format!("Enumerator: {}", e.name));
        if let Some(v) = &e.value {
            self.with_indent(|p| {
                p.line("Value:");
                p.with_indent(|p| p.expr(v));
            });
        }
    }

    // ---------------------------------------------------------------------
    // Initializers
    // ---------------------------------------------------------------------

    fn initializer(&mut self, i: &Initializer) {
        match i {
            Initializer::Expr(e) => self.expr(e),
            Initializer::List { items, .. } => {
                self.line(&format!("InitializerList ({} item(s))", items.len()));
                self.with_indent(|p| {
                    for di in items {
                        p.designated_init(di);
                    }
                });
            }
        }
    }

    fn designated_init(&mut self, d: &DesignatedInit) {
        self.line("DesignatedInit");
        self.with_indent(|p| {
            if !d.designators.is_empty() {
                p.line("Designators:");
                p.with_indent(|p| {
                    for dg in &d.designators {
                        p.designator(dg);
                    }
                });
            }
            p.initializer(&d.initializer);
        });
    }

    fn designator(&mut self, d: &Designator) {
        match d {
            Designator::Index(e) => {
                self.line("Index:");
                self.with_indent(|p| p.expr(e));
            }
            Designator::Field(name) => {
                self.line(&format!("Field: .{name}"));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Statements
    // ---------------------------------------------------------------------

    fn compound_stmt(&mut self, c: &CompoundStmt) {
        self.line(&format!("CompoundStmt ({} item(s))", c.items.len()));
        self.with_indent(|p| {
            for item in &c.items {
                p.block_item(item);
            }
        });
    }

    fn block_item(&mut self, item: &BlockItem) {
        match item {
            BlockItem::Declaration(d) => self.declaration(d),
            BlockItem::Statement(s) => self.stmt(s),
            BlockItem::StaticAssert(sa) => self.static_assert(sa),
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Compound(c) => self.compound_stmt(c),
            Stmt::Expr { expr, .. } => match expr {
                Some(e) => {
                    self.line("ExprStmt");
                    self.with_indent(|p| p.expr(e));
                }
                None => self.line("EmptyStmt"),
            },
            Stmt::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.line("If");
                self.with_indent(|p| {
                    p.line("Cond:");
                    p.with_indent(|p| p.expr(condition));
                    p.line("Then:");
                    p.with_indent(|p| p.stmt(then_branch));
                    if let Some(eb) = else_branch {
                        p.line("Else:");
                        p.with_indent(|p| p.stmt(eb));
                    }
                });
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.line("While");
                self.with_indent(|p| {
                    p.line("Cond:");
                    p.with_indent(|p| p.expr(condition));
                    p.line("Body:");
                    p.with_indent(|p| p.stmt(body));
                });
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                self.line("DoWhile");
                self.with_indent(|p| {
                    p.line("Body:");
                    p.with_indent(|p| p.stmt(body));
                    p.line("Cond:");
                    p.with_indent(|p| p.expr(condition));
                });
            }
            Stmt::For {
                init,
                condition,
                update,
                body,
                ..
            } => {
                self.line("For");
                self.with_indent(|p| {
                    match init {
                        Some(ForInit::Declaration(d)) => {
                            p.line("Init:");
                            p.with_indent(|p| p.declaration(d));
                        }
                        Some(ForInit::Expr(e)) => {
                            p.line("Init:");
                            p.with_indent(|p| p.expr(e));
                        }
                        None => p.line("Init: (none)"),
                    }
                    match condition {
                        Some(c) => {
                            p.line("Cond:");
                            p.with_indent(|p| p.expr(c));
                        }
                        None => p.line("Cond: (none)"),
                    }
                    match update {
                        Some(u) => {
                            p.line("Update:");
                            p.with_indent(|p| p.expr(u));
                        }
                        None => p.line("Update: (none)"),
                    }
                    p.line("Body:");
                    p.with_indent(|p| p.stmt(body));
                });
            }
            Stmt::Switch { expr, body, .. } => {
                self.line("Switch");
                self.with_indent(|p| {
                    p.line("Expr:");
                    p.with_indent(|p| p.expr(expr));
                    p.line("Body:");
                    p.with_indent(|p| p.stmt(body));
                });
            }
            Stmt::Case { value, body, .. } => {
                self.line("Case");
                self.with_indent(|p| {
                    p.line("Value:");
                    p.with_indent(|p| p.expr(value));
                    p.line("Body:");
                    p.with_indent(|p| p.stmt(body));
                });
            }
            Stmt::Default { body, .. } => {
                self.line("Default");
                self.with_indent(|p| p.stmt(body));
            }
            Stmt::Return { value, .. } => {
                self.line("Return");
                if let Some(v) = value {
                    self.with_indent(|p| p.expr(v));
                }
            }
            Stmt::Break { .. } => self.line("Break"),
            Stmt::Continue { .. } => self.line("Continue"),
            Stmt::Goto { label, .. } => self.line(&format!("Goto: {label}")),
            Stmt::Label { name, stmt, .. } => {
                self.line(&format!("Label: {name}"));
                self.with_indent(|p| p.stmt(stmt));
            }
        }
    }

    // ---------------------------------------------------------------------
    // Expressions
    // ---------------------------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::IntLiteral { value, suffix, .. } => {
                self.line(&format!("IntLiteral {value} suffix={suffix:?}"));
            }
            Expr::FloatLiteral { value, suffix, .. } => {
                self.line(&format!("FloatLiteral {value} suffix={suffix:?}"));
            }
            Expr::CharLiteral { value, prefix, .. } => {
                self.line(&format!("CharLiteral {value} prefix={prefix:?}"));
            }
            Expr::StringLiteral { value, prefix, .. } => {
                self.line(&format!("StringLiteral {value:?} prefix={prefix:?}"));
            }
            Expr::Ident { name, .. } => {
                self.line(&format!("Ident {name}"));
            }
            Expr::BinaryOp {
                op, left, right, ..
            } => {
                self.line(&format!("BinaryOp {}", binop_name(*op)));
                self.with_indent(|p| {
                    p.expr(left);
                    p.expr(right);
                });
            }
            Expr::UnaryOp { op, operand, .. } => {
                self.line(&format!("UnaryOp {}", unop_name(*op)));
                self.with_indent(|p| p.expr(operand));
            }
            Expr::PostfixOp { op, operand, .. } => {
                self.line(&format!("PostfixOp {}", postfix_name(*op)));
                self.with_indent(|p| p.expr(operand));
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.line("Conditional");
                self.with_indent(|p| {
                    p.line("Cond:");
                    p.with_indent(|p| p.expr(condition));
                    p.line("Then:");
                    p.with_indent(|p| p.expr(then_expr));
                    p.line("Else:");
                    p.with_indent(|p| p.expr(else_expr));
                });
            }
            Expr::Assignment {
                op, target, value, ..
            } => {
                self.line(&format!("Assignment {}", assign_name(*op)));
                self.with_indent(|p| {
                    p.expr(target);
                    p.expr(value);
                });
            }
            Expr::FunctionCall { callee, args, .. } => {
                self.line(&format!("FunctionCall ({} arg(s))", args.len()));
                self.with_indent(|p| {
                    p.line("Callee:");
                    p.with_indent(|p| p.expr(callee));
                    if !args.is_empty() {
                        p.line("Args:");
                        p.with_indent(|p| {
                            for a in args {
                                p.expr(a);
                            }
                        });
                    }
                });
            }
            Expr::MemberAccess {
                object,
                member,
                is_arrow,
                ..
            } => {
                let arrow = if *is_arrow { "->" } else { "." };
                self.line(&format!("MemberAccess {arrow}{member}"));
                self.with_indent(|p| p.expr(object));
            }
            Expr::ArraySubscript { array, index, .. } => {
                self.line("ArraySubscript");
                self.with_indent(|p| {
                    p.line("Array:");
                    p.with_indent(|p| p.expr(array));
                    p.line("Index:");
                    p.with_indent(|p| p.expr(index));
                });
            }
            Expr::Cast {
                type_name, expr, ..
            } => {
                self.line("Cast");
                self.with_indent(|p| {
                    p.line("Type:");
                    p.with_indent(|p| p.type_name(type_name));
                    p.line("Expr:");
                    p.with_indent(|p| p.expr(expr));
                });
            }
            Expr::SizeofExpr { expr, .. } => {
                self.line("SizeofExpr");
                self.with_indent(|p| p.expr(expr));
            }
            Expr::SizeofType { type_name, .. } => {
                self.line("SizeofType");
                self.with_indent(|p| p.type_name(type_name));
            }
            Expr::AlignofType { type_name, .. } => {
                self.line("AlignofType");
                self.with_indent(|p| p.type_name(type_name));
            }
            Expr::CompoundLiteral {
                type_name,
                initializer,
                ..
            } => {
                self.line("CompoundLiteral");
                self.with_indent(|p| {
                    p.line("Type:");
                    p.with_indent(|p| p.type_name(type_name));
                    p.line("Init:");
                    p.with_indent(|p| p.initializer(initializer));
                });
            }
            Expr::GenericSelection {
                controlling,
                associations,
                ..
            } => {
                self.line(&format!(
                    "GenericSelection ({} assoc(s))",
                    associations.len()
                ));
                self.with_indent(|p| {
                    p.line("Controlling:");
                    p.with_indent(|p| p.expr(controlling));
                    for a in associations {
                        p.line("Association:");
                        p.with_indent(|p| {
                            match &a.type_name {
                                Some(tn) => {
                                    p.line("Type:");
                                    p.with_indent(|p| p.type_name(tn));
                                }
                                None => p.line("Type: default"),
                            }
                            p.line("Expr:");
                            p.with_indent(|p| p.expr(&a.expr));
                        });
                    }
                });
            }
            Expr::Comma { exprs, .. } => {
                self.line(&format!("Comma ({} expr(s))", exprs.len()));
                self.with_indent(|p| {
                    for e in exprs {
                        p.expr(e);
                    }
                });
            }
            Expr::BuiltinOffsetof { ty, designator, .. } => {
                self.line("BuiltinOffsetof");
                self.with_indent(|p| {
                    p.line("Type:");
                    p.with_indent(|p| p.type_name(ty));
                    p.line("Designator:");
                    p.with_indent(|p| {
                        for step in designator {
                            match step {
                                OffsetofMember::Field(name) => p.line(&format!(".{name}")),
                                OffsetofMember::Subscript(idx) => {
                                    p.line("[");
                                    p.with_indent(|p| p.expr(idx));
                                    p.line("]");
                                }
                            }
                        }
                    });
                });
            }
            Expr::BuiltinTypesCompatibleP { t1, t2, .. } => {
                self.line("BuiltinTypesCompatibleP");
                self.with_indent(|p| {
                    p.line("T1:");
                    p.with_indent(|p| p.type_name(t1));
                    p.line("T2:");
                    p.with_indent(|p| p.type_name(t2));
                });
            }
        }
    }

    fn type_name(&mut self, tn: &TypeName) {
        self.specifiers("TypeName", &tn.specifiers);
        if let Some(ad) = &tn.abstract_declarator {
            self.abstract_declarator(ad);
        }
    }

    fn abstract_declarator(&mut self, ad: &AbstractDeclarator) {
        let depth = ad.pointers.len();
        if depth == 0 && ad.direct.is_none() {
            self.line("AbstractDeclarator (empty)");
            return;
        }
        self.line(&format!("AbstractDeclarator (pointer_depth={depth})"));
        self.with_indent(|p| {
            for (i, pq) in ad.pointers.iter().enumerate() {
                let quals: Vec<String> = pq.qualifiers.iter().map(|q| format!("{q:?}")).collect();
                let summary = if quals.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", quals.join(", "))
                };
                p.line(&format!("Pointer #{i}{summary}"));
            }
            if let Some(d) = &ad.direct {
                p.direct_abstract_declarator(d);
            }
        });
    }

    fn direct_abstract_declarator(&mut self, d: &DirectAbstractDeclarator) {
        match d {
            DirectAbstractDeclarator::Parenthesized(inner) => {
                self.line("Parenthesized:");
                self.with_indent(|p| p.abstract_declarator(inner));
            }
            DirectAbstractDeclarator::Array { base, size, .. } => {
                self.line("Array");
                self.with_indent(|p| {
                    p.line("Size:");
                    p.with_indent(|p| p.array_size(size));
                    if let Some(b) = base {
                        p.line("Base:");
                        p.with_indent(|p| p.direct_abstract_declarator(b));
                    }
                });
            }
            DirectAbstractDeclarator::Function {
                base,
                params,
                is_variadic,
                ..
            } => {
                let variadic_part = if *is_variadic { " (variadic)" } else { "" };
                self.line(&format!(
                    "Function{variadic_part} ({} param(s))",
                    params.len()
                ));
                self.with_indent(|p| {
                    for param in params {
                        p.param_decl(param);
                    }
                    if let Some(b) = base {
                        p.line("Base:");
                        p.with_indent(|p| p.direct_abstract_declarator(b));
                    }
                });
            }
        }
    }
}

// =========================================================================
// Leaf formatting helpers
// =========================================================================

fn type_specifier_name(ts: &TypeSpecifierToken) -> String {
    match ts {
        TypeSpecifierToken::Void => "Void".into(),
        TypeSpecifierToken::Char => "Char".into(),
        TypeSpecifierToken::Short => "Short".into(),
        TypeSpecifierToken::Int => "Int".into(),
        TypeSpecifierToken::Long => "Long".into(),
        TypeSpecifierToken::Float => "Float".into(),
        TypeSpecifierToken::Double => "Double".into(),
        TypeSpecifierToken::Signed => "Signed".into(),
        TypeSpecifierToken::Unsigned => "Unsigned".into(),
        TypeSpecifierToken::Bool => "Bool".into(),
        TypeSpecifierToken::Complex => "Complex".into(),
        TypeSpecifierToken::Struct(s) => {
            format!("Struct({})", s.name.as_deref().unwrap_or("<anonymous>"))
        }
        TypeSpecifierToken::Union(s) => {
            format!("Union({})", s.name.as_deref().unwrap_or("<anonymous>"))
        }
        TypeSpecifierToken::Enum(e) => {
            format!("Enum({})", e.name.as_deref().unwrap_or("<anonymous>"))
        }
        TypeSpecifierToken::TypedefName(n) => format!("Typedef({n})"),
        TypeSpecifierToken::Atomic(_) => "Atomic".into(),
        TypeSpecifierToken::TypeofExpr(_) => "TypeofExpr".into(),
        TypeSpecifierToken::TypeofType(_) => "TypeofType".into(),
    }
}

fn binop_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::BitAnd => "&",
        BinaryOp::BitOr => "|",
        BinaryOp::BitXor => "^",
        BinaryOp::Shl => "<<",
        BinaryOp::Shr => ">>",
        BinaryOp::LogAnd => "&&",
        BinaryOp::LogOr => "||",
        BinaryOp::Eq => "==",
        BinaryOp::Ne => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Gt => ">",
        BinaryOp::Le => "<=",
        BinaryOp::Ge => ">=",
    }
}

fn unop_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::PreIncrement => "++(pre)",
        UnaryOp::PreDecrement => "--(pre)",
        UnaryOp::AddrOf => "&",
        UnaryOp::Deref => "*",
        UnaryOp::Plus => "+",
        UnaryOp::Minus => "-",
        UnaryOp::BitNot => "~",
        UnaryOp::LogNot => "!",
    }
}

fn postfix_name(op: PostfixOp) -> &'static str {
    match op {
        PostfixOp::PostIncrement => "++(post)",
        PostfixOp::PostDecrement => "--(post)",
    }
}

fn assign_name(op: AssignOp) -> &'static str {
    match op {
        AssignOp::Assign => "=",
        AssignOp::AddAssign => "+=",
        AssignOp::SubAssign => "-=",
        AssignOp::MulAssign => "*=",
        AssignOp::DivAssign => "/=",
        AssignOp::ModAssign => "%=",
        AssignOp::BitAndAssign => "&=",
        AssignOp::BitOrAssign => "|=",
        AssignOp::BitXorAssign => "^=",
        AssignOp::ShlAssign => "<<=",
        AssignOp::ShrAssign => ">>=",
    }
}
