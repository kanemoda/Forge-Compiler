//! Tests for struct / union / enum layout computation.
//!
//! These cover the full pipeline: a parser [`StructDef`] or [`EnumDef`]
//! is built by hand, completion is invoked, and the resulting
//! [`StructLayout`] / [`UnionLayout`] / [`EnumLayout`] is inspected.
//! The System V AMD64 ABI provides the target.

use forge_lexer::{IntSuffix, Span};
use forge_parser::ast::{
    ArraySize as ParserArraySize, DeclSpecifiers, Declarator, DirectDeclarator, EnumDef,
    Enumerator, Expr, StructDef, StructField, StructFieldDeclarator, StructMember, StructOrUnion,
    TypeSpecifierToken,
};
use forge_parser::node_id::NodeId;

use crate::context::SemaContext;
use crate::layout::{complete_enum, complete_struct, complete_union};
use crate::resolve::resolve_declarator;
use crate::scope::SymbolTable;
use crate::types::{QualType, StructLayout, StructTypeId, Type, UnionLayout, UnionTypeId};

use super::helpers::ti;

const S: Span = Span::new(0, 0);
const N: NodeId = NodeId::DUMMY;

// ---------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------

fn specs(ts: Vec<TypeSpecifierToken>) -> DeclSpecifiers {
    DeclSpecifiers {
        storage_class: None,
        type_specifiers: ts,
        type_qualifiers: Vec::new(),
        function_specifiers: Vec::new(),
        alignment: None,
        attributes: Vec::new(),
        span: S,
    }
}

fn ident_decl(name: &str) -> Declarator {
    Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Identifier(name.to_string(), S),
        span: S,
    }
}

fn int_lit(v: u64) -> Expr {
    Expr::IntLiteral {
        value: v,
        suffix: IntSuffix::None,
        span: S,
        node_id: N,
    }
}

fn field(ts: Vec<TypeSpecifierToken>, name: &str) -> StructMember {
    StructMember::Field(StructField {
        specifiers: specs(ts),
        declarators: vec![StructFieldDeclarator {
            declarator: Some(ident_decl(name)),
            bit_width: None,
            span: S,
        }],
        span: S,
        node_id: N,
    })
}

fn bit_field(ts: Vec<TypeSpecifierToken>, name: Option<&str>, width: u64) -> StructMember {
    StructMember::Field(StructField {
        specifiers: specs(ts),
        declarators: vec![StructFieldDeclarator {
            declarator: name.map(ident_decl),
            bit_width: Some(Box::new(int_lit(width))),
            span: S,
        }],
        span: S,
        node_id: N,
    })
}

fn flexible_array_field(ts: Vec<TypeSpecifierToken>, name: &str) -> StructMember {
    StructMember::Field(StructField {
        specifiers: specs(ts),
        declarators: vec![StructFieldDeclarator {
            declarator: Some(Declarator {
                pointers: Vec::new(),
                direct: DirectDeclarator::Array {
                    base: Box::new(DirectDeclarator::Identifier(name.to_string(), S)),
                    size: ParserArraySize::Unspecified,
                    qualifiers: Vec::new(),
                    is_static: false,
                    span: S,
                },
                span: S,
            }),
            bit_width: None,
            span: S,
        }],
        span: S,
        node_id: N,
    })
}

fn struct_def(name: Option<&str>, members: Vec<StructMember>) -> StructDef {
    StructDef {
        kind: StructOrUnion::Struct,
        name: name.map(String::from),
        members: Some(members),
        attributes: Vec::new(),
        span: S,
    }
}

fn union_def(name: Option<&str>, members: Vec<StructMember>) -> StructDef {
    StructDef {
        kind: StructOrUnion::Union,
        name: name.map(String::from),
        members: Some(members),
        attributes: Vec::new(),
        span: S,
    }
}

fn enum_def(name: Option<&str>, enumerators: Vec<Enumerator>) -> EnumDef {
    EnumDef {
        name: name.map(String::from),
        enumerators: Some(enumerators),
        attributes: Vec::new(),
        span: S,
    }
}

fn enum_value(name: &str, value: Option<u64>) -> Enumerator {
    Enumerator {
        name: name.to_string(),
        value: value.map(|v| Box::new(int_lit(v))),
        attributes: Vec::new(),
        span: S,
    }
}

fn prepare_struct(ctx: &mut SemaContext, _def: &StructDef) -> StructTypeId {
    let sid = ctx.type_ctx.fresh_struct_id();
    ctx.type_ctx.set_struct(sid, StructLayout::default());
    sid
}

fn prepare_union(ctx: &mut SemaContext, _def: &StructDef) -> UnionTypeId {
    let uid = ctx.type_ctx.fresh_union_id();
    ctx.type_ctx.set_union(uid, UnionLayout::default());
    uid
}

// ---------------------------------------------------------------------
// Struct layout — basic sizes / alignment
// ---------------------------------------------------------------------

#[test]
fn struct_with_int_then_char_is_padded_to_eight_bytes_on_lp64() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { int a; char b; };  → size 8, align 4
    let def = struct_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Int], "a"),
            field(vec![TypeSpecifierToken::Char], "b"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout must exist");
    assert_eq!(layout.total_size, 8);
    assert_eq!(layout.alignment, 4);
    assert_eq!(layout.members[0].offset, 0);
    assert_eq!(layout.members[1].offset, 4);
}

#[test]
fn struct_with_char_then_int_pads_the_char() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { char a; int b; };  → offsets 0 and 4, size 8
    let def = struct_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Char], "a"),
            field(vec![TypeSpecifierToken::Int], "b"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout must exist");
    assert_eq!(layout.members[0].offset, 0);
    assert_eq!(layout.members[1].offset, 4);
    assert_eq!(layout.total_size, 8);
}

#[test]
fn struct_with_long_then_char_has_alignment_eight() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { long a; char b; };  → align 8, size 16
    let def = struct_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Long], "a"),
            field(vec![TypeSpecifierToken::Char], "b"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout must exist");
    assert_eq!(layout.alignment, 8);
    assert_eq!(layout.total_size, 16);
}

// ---------------------------------------------------------------------
// Bit fields
// ---------------------------------------------------------------------

#[test]
fn bit_fields_pack_into_one_storage_unit() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { unsigned a:3; unsigned b:5; unsigned c:24; };
    let def = struct_def(
        None,
        vec![
            bit_field(vec![TypeSpecifierToken::Unsigned], Some("a"), 3),
            bit_field(vec![TypeSpecifierToken::Unsigned], Some("b"), 5),
            bit_field(vec![TypeSpecifierToken::Unsigned], Some("c"), 24),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout");
    assert_eq!(layout.members.len(), 3);
    for m in &layout.members {
        assert!(m.bit_field.is_some(), "expected bit-field on {:?}", m.name);
    }
    let bf0 = layout.members[0].bit_field.as_ref().unwrap();
    let bf1 = layout.members[1].bit_field.as_ref().unwrap();
    let bf2 = layout.members[2].bit_field.as_ref().unwrap();
    assert_eq!(bf0.bit_offset, 0);
    assert_eq!(bf1.bit_offset, 3);
    assert_eq!(bf2.bit_offset, 8);
    // All three pack into a single 32-bit unit → size 4.
    assert_eq!(layout.total_size, 4);
}

#[test]
fn zero_width_bit_field_forces_boundary() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { unsigned a:3; unsigned :0; unsigned b:3; };
    let def = struct_def(
        None,
        vec![
            bit_field(vec![TypeSpecifierToken::Unsigned], Some("a"), 3),
            bit_field(vec![TypeSpecifierToken::Unsigned], None, 0),
            bit_field(vec![TypeSpecifierToken::Unsigned], Some("b"), 3),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout");
    // The `b` field starts in a fresh storage unit.
    let bf_b = layout
        .members
        .iter()
        .find(|m| m.name.as_deref() == Some("b"))
        .expect("b must be present")
        .bit_field
        .as_ref()
        .unwrap();
    assert_eq!(bf_b.bit_offset, 0, "b should start a fresh unit");
    // Size is 2 storage units = 8 bytes for unsigned.
    assert_eq!(layout.total_size, 8);
}

#[test]
fn bit_field_width_exceeding_type_errors() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { unsigned a : 64; };   unsigned is 4 bytes → 32 bits
    let def = struct_def(
        None,
        vec![bit_field(vec![TypeSpecifierToken::Unsigned], Some("a"), 64)],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected diagnostic for oversized bit-field"
    );
}

// ---------------------------------------------------------------------
// Flexible array members
// ---------------------------------------------------------------------

#[test]
fn flexible_array_member_last_is_ok() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { int n; char data[]; };
    let def = struct_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Int], "n"),
            flexible_array_field(vec![TypeSpecifierToken::Char], "data"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout");
    assert!(layout.has_flexible_array);
}

#[test]
fn flexible_array_member_not_last_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { int n; char data[]; int trailer; };  — illegal
    let def = struct_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Int], "n"),
            flexible_array_field(vec![TypeSpecifierToken::Char], "data"),
            field(vec![TypeSpecifierToken::Int], "trailer"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.has_errors(),
        "expected diagnostic for FAM not at end of struct"
    );
}

#[test]
fn flexible_array_as_sole_member_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { char data[]; };  — illegal per §6.7.2.1p18
    let def = struct_def(
        None,
        vec![flexible_array_field(vec![TypeSpecifierToken::Char], "data")],
    );
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(ctx.has_errors(), "expected diagnostic for FAM-only struct");
}

// ---------------------------------------------------------------------
// Unions
// ---------------------------------------------------------------------

#[test]
fn union_size_is_max_of_members_align_is_max() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // union { char a; int b; long c; };  → size 8, align 8
    let def = union_def(
        None,
        vec![
            field(vec![TypeSpecifierToken::Char], "a"),
            field(vec![TypeSpecifierToken::Int], "b"),
            field(vec![TypeSpecifierToken::Long], "c"),
        ],
    );
    let uid = prepare_union(&mut ctx, &def);
    complete_union(uid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.union_layout(uid).expect("layout");
    assert_eq!(layout.total_size, 8);
    assert_eq!(layout.alignment, 8);
    for m in &layout.members {
        assert_eq!(m.offset, 0);
    }
}

// ---------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------

#[test]
fn enum_with_small_positives_picks_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // enum { A, B = 5, C };  → int
    let def = enum_def(
        None,
        vec![
            enum_value("A", None),
            enum_value("B", Some(5)),
            enum_value("C", None),
        ],
    );
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx
        .set_enum(eid, crate::types::EnumLayout::default());
    complete_enum(eid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(matches!(
        layout.underlying_type,
        Some(Type::Int { is_unsigned: false })
    ));
    assert_eq!(layout.constants.len(), 3);
    assert_eq!(layout.constants[0].1, 0);
    assert_eq!(layout.constants[1].1, 5);
    assert_eq!(layout.constants[2].1, 6);
}

#[test]
fn enum_with_large_positive_picks_unsigned_int() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // enum { A = 0xFFFFFFFF };  → unsigned int
    let def = enum_def(None, vec![enum_value("A", Some(0xFFFF_FFFF))]);
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx
        .set_enum(eid, crate::types::EnumLayout::default());
    complete_enum(eid, &def, &mut table, &ti(), &mut ctx);

    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(matches!(
        layout.underlying_type,
        Some(Type::Int { is_unsigned: true })
    ));
}

#[test]
fn enum_with_negative_value_picks_int() {
    // An enumerator expression can be `-1` only via a UnaryOp Minus,
    // which the ICX evaluator handles.  Here we emulate by supplying a
    // constant negative via `Expr::UnaryOp` wrapping.
    use forge_parser::ast_ops::UnaryOp;
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    let neg1 = Expr::UnaryOp {
        op: UnaryOp::Minus,
        operand: Box::new(int_lit(1)),
        span: S,
        node_id: N,
    };
    let def = EnumDef {
        name: None,
        enumerators: Some(vec![
            Enumerator {
                name: "NEG".into(),
                value: Some(Box::new(neg1)),
                attributes: Vec::new(),
                span: S,
            },
            enum_value("ZERO", Some(0)),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx
        .set_enum(eid, crate::types::EnumLayout::default());
    complete_enum(eid, &def, &mut table, &ti(), &mut ctx);

    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(matches!(
        layout.underlying_type,
        Some(Type::Int { is_unsigned: false })
    ));
    assert_eq!(layout.constants[0].1, -1);
    assert_eq!(layout.constants[1].1, 0);
}

#[test]
fn enum_registers_enumerators_in_current_scope() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();
    let def = enum_def(
        Some("Color"),
        vec![
            enum_value("Red", None),
            enum_value("Green", None),
            enum_value("Blue", None),
        ],
    );
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx
        .set_enum(eid, crate::types::EnumLayout::default());
    complete_enum(eid, &def, &mut table, &ti(), &mut ctx);

    for name in ["Red", "Green", "Blue"] {
        let sym = table
            .lookup(name)
            .unwrap_or_else(|| panic!("{name} not registered"));
        match sym.kind {
            crate::scope::SymbolKind::EnumConstant { enum_id, .. } => {
                assert_eq!(enum_id, eid);
            }
            _ => panic!("{name} has unexpected kind {:?}", sym.kind),
        }
    }
}

// ---------------------------------------------------------------------
// Self-referential / tag tests
// ---------------------------------------------------------------------

#[test]
fn struct_with_pointer_to_same_struct_is_fine() {
    use forge_parser::ast::{Declarator as PDecl, DirectDeclarator as PDir, PointerQualifiers};
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // Pre-register a tag so the upcoming `struct node *next;` member
    // resolves.  A real driver path goes through
    // `resolve_struct_or_union`, which does this for us.
    let sid = ctx.type_ctx.fresh_struct_id();
    ctx.type_ctx.set_struct(
        sid,
        StructLayout {
            tag: Some("node".into()),
            ..StructLayout::default()
        },
    );
    let tag_entry = crate::scope::TagEntry::Struct(sid);
    let _ = table.declare_tag("node", tag_entry, S, &mut ctx);

    // struct node { struct node *next; };
    let next_decl = PDecl {
        pointers: vec![PointerQualifiers {
            qualifiers: Vec::new(),
            attributes: Vec::new(),
        }],
        direct: PDir::Identifier("next".into(), S),
        span: S,
    };

    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: Some("node".into()),
        members: Some(vec![StructMember::Field(StructField {
            specifiers: specs(vec![TypeSpecifierToken::Struct(StructDef {
                kind: StructOrUnion::Struct,
                name: Some("node".into()),
                members: None,
                attributes: Vec::new(),
                span: S,
            })]),
            declarators: vec![StructFieldDeclarator {
                declarator: Some(next_decl),
                bit_width: None,
                span: S,
            }],
            span: S,
            node_id: N,
        })]),
        attributes: Vec::new(),
        span: S,
    };
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout");
    // On LP64 a pointer is 8 bytes.
    assert_eq!(layout.total_size, 8);
    assert_eq!(layout.alignment, 8);
}

// ---------------------------------------------------------------------
// Anonymous nested struct members (C11)
// ---------------------------------------------------------------------

#[test]
fn anonymous_nested_struct_flattens_field_names() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { int outer; struct { int x; int y; }; };
    let inner_def = StructDef {
        kind: StructOrUnion::Struct,
        name: None,
        members: Some(vec![
            field(vec![TypeSpecifierToken::Int], "x"),
            field(vec![TypeSpecifierToken::Int], "y"),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let outer_def = StructDef {
        kind: StructOrUnion::Struct,
        name: None,
        members: Some(vec![
            field(vec![TypeSpecifierToken::Int], "outer"),
            // An anonymous nested struct member: specifiers only, no
            // declarators.
            StructMember::Field(StructField {
                specifiers: specs(vec![TypeSpecifierToken::Struct(inner_def)]),
                declarators: Vec::new(),
                span: S,
                node_id: N,
            }),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let sid = prepare_struct(&mut ctx, &outer_def);
    complete_struct(sid, &outer_def, &mut table, &ti(), &mut ctx);
    assert!(!ctx.has_errors(), "diagnostics: {:?}", ctx.diagnostics);

    let layout = ctx.type_ctx.struct_layout(sid).expect("layout");
    // The anonymous struct should contribute an entry with a populated
    // `anon_members` map including both `x` and `y`.
    let anon = layout
        .members
        .iter()
        .find(|m| m.name.is_none() && m.anon_members.is_some())
        .expect("an anonymous nested struct should be present");
    let map = anon.anon_members.as_ref().unwrap();
    assert!(map.fields.contains_key("x"));
    assert!(map.fields.contains_key("y"));
}

// ---------------------------------------------------------------------
// Inline _Static_assert in struct
// ---------------------------------------------------------------------

#[test]
fn struct_inline_static_assert_fails_emits_diag() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct { _Static_assert(0, "nope"); int x; };
    let def = StructDef {
        kind: StructOrUnion::Struct,
        name: None,
        members: Some(vec![
            StructMember::StaticAssert(forge_parser::ast::StaticAssert {
                condition: Box::new(int_lit(0)),
                message: Some("nope".into()),
                span: S,
            }),
            field(vec![TypeSpecifierToken::Int], "x"),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let sid = prepare_struct(&mut ctx, &def);
    complete_struct(sid, &def, &mut table, &ti(), &mut ctx);
    assert!(
        ctx.diagnostics.iter().any(|d| d.message.contains("nope")),
        "expected 'nope' diagnostic, got {:?}",
        ctx.diagnostics
    );
}

// ---------------------------------------------------------------------
// Enum underlying-type widening (C17 §6.7.2.2p4)
// ---------------------------------------------------------------------

fn build_enum_and_complete(def: &EnumDef, ctx: &mut SemaContext) -> crate::types::EnumTypeId {
    let eid = ctx.type_ctx.fresh_enum_id();
    ctx.type_ctx
        .set_enum(eid, crate::types::EnumLayout::default());
    let mut table = SymbolTable::new();
    complete_enum(eid, def, &mut table, &ti(), ctx);
    eid
}

fn neg_expr(v: u64) -> Expr {
    Expr::UnaryOp {
        op: forge_parser::ast_ops::UnaryOp::Minus,
        operand: Box::new(int_lit(v)),
        span: S,
        node_id: N,
    }
}

#[test]
fn enum_small_values_uses_int() {
    let mut ctx = SemaContext::new();
    let def = enum_def(
        None,
        vec![
            enum_value("A", None),
            enum_value("B", None),
            enum_value("C", None),
        ],
    );
    let eid = build_enum_and_complete(&def, &mut ctx);
    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(matches!(
        layout.underlying_type,
        Some(Type::Int { is_unsigned: false })
    ));
}

#[test]
fn enum_with_negative_uses_signed_int() {
    // enum { NEG = -1, POS = 1 };  → signed int
    let mut ctx = SemaContext::new();
    let def = EnumDef {
        name: None,
        enumerators: Some(vec![
            Enumerator {
                name: "NEG".into(),
                value: Some(Box::new(neg_expr(1))),
                attributes: Vec::new(),
                span: S,
            },
            enum_value("POS", Some(1)),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let eid = build_enum_and_complete(&def, &mut ctx);
    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(matches!(
        layout.underlying_type,
        Some(Type::Int { is_unsigned: false })
    ));
}

#[test]
fn enum_exceeding_int_max_widens_to_unsigned_int() {
    // enum { BIG = 3_000_000_000 }; → unsigned int
    let mut ctx = SemaContext::new();
    let def = enum_def(None, vec![enum_value("BIG", Some(3_000_000_000))]);
    let eid = build_enum_and_complete(&def, &mut ctx);
    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(
        matches!(
            layout.underlying_type,
            Some(Type::Int { is_unsigned: true })
        ),
        "got underlying_type = {:?}",
        layout.underlying_type
    );
}

#[test]
fn enum_exceeding_uint_max_widens_to_long() {
    // enum { HUGE = 5_000_000_000 }; → signed long (LP64)
    let mut ctx = SemaContext::new();
    let def = enum_def(None, vec![enum_value("HUGE", Some(5_000_000_000))]);
    let eid = build_enum_and_complete(&def, &mut ctx);
    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(
        matches!(
            layout.underlying_type,
            Some(Type::Long { is_unsigned: false })
        ),
        "got underlying_type = {:?}",
        layout.underlying_type
    );
}

#[test]
fn enum_large_and_negative_uses_long() {
    // enum { NEG = -1, HUGE = 5_000_000_000 }; → signed long
    let mut ctx = SemaContext::new();
    let def = EnumDef {
        name: None,
        enumerators: Some(vec![
            Enumerator {
                name: "NEG".into(),
                value: Some(Box::new(neg_expr(1))),
                attributes: Vec::new(),
                span: S,
            },
            enum_value("HUGE", Some(5_000_000_000)),
        ]),
        attributes: Vec::new(),
        span: S,
    };
    let eid = build_enum_and_complete(&def, &mut ctx);
    let layout = ctx.type_ctx.enum_layout(eid).expect("layout");
    assert!(
        matches!(
            layout.underlying_type,
            Some(Type::Long { is_unsigned: false })
        ),
        "got underlying_type = {:?}",
        layout.underlying_type
    );
}

#[test]
fn sizeof_widened_enum_big_is_four_bytes() {
    // sizeof(enum { BIG = 3_000_000_000 }) == 4
    let mut ctx = SemaContext::new();
    let def = enum_def(None, vec![enum_value("BIG", Some(3_000_000_000))]);
    let eid = build_enum_and_complete(&def, &mut ctx);
    let ty = Type::Enum(eid);
    assert_eq!(ty.size_of(&ti(), &ctx.type_ctx), Some(4));
}

#[test]
fn sizeof_widened_enum_huge_is_eight_bytes() {
    // sizeof(enum { HUGE = 5_000_000_000 }) == 8 on LP64
    let mut ctx = SemaContext::new();
    let def = enum_def(None, vec![enum_value("HUGE", Some(5_000_000_000))]);
    let eid = build_enum_and_complete(&def, &mut ctx);
    let ty = Type::Enum(eid);
    assert_eq!(ty.size_of(&ti(), &ctx.type_ctx), Some(8));
}

// ---------------------------------------------------------------------
// FAM-bearing struct as an array element (C17 §6.7.2.1p18b)
// ---------------------------------------------------------------------

#[test]
fn array_of_flexible_array_member_struct_is_error() {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    // struct has_fam { int n; int data[]; };
    let fam_def = struct_def(
        Some("has_fam"),
        vec![
            field(vec![TypeSpecifierToken::Int], "n"),
            flexible_array_field(vec![TypeSpecifierToken::Int], "data"),
        ],
    );
    let sid = prepare_struct(&mut ctx, &fam_def);
    complete_struct(sid, &fam_def, &mut table, &ti(), &mut ctx);
    assert!(
        !ctx.has_errors(),
        "has_fam definition should be valid; got: {:?}",
        ctx.diagnostics
    );

    // struct has_fam items[10];
    let base = QualType::unqualified(Type::Struct(sid));
    let decl = Declarator {
        pointers: Vec::new(),
        direct: DirectDeclarator::Array {
            base: Box::new(DirectDeclarator::Identifier("items".into(), S)),
            size: ParserArraySize::Expr(Box::new(int_lit(10))),
            qualifiers: Vec::new(),
            is_static: false,
            span: S,
        },
        span: S,
    };
    let _ = resolve_declarator(base, &decl, false, &mut table, &ti(), &mut ctx);

    assert!(
        ctx.has_errors(),
        "expected error for array of FAM-bearing struct"
    );
    assert!(
        ctx.diagnostics
            .iter()
            .any(|d| d.message.contains("flexible array")),
        "expected 'flexible array' in message, got: {:?}",
        ctx.diagnostics
    );
}
