//! Struct, union, and enum layout computation.
//!
//! This module owns the *second phase* of tag analysis: once the
//! parser has produced an incomplete placeholder (so self-referential
//! types such as `struct node { struct node *next; };` can refer to
//! themselves), the functions here walk the declared members, compute
//! byte offsets, and install the finished [`StructLayout`],
//! [`UnionLayout`], or [`EnumLayout`] back into the type context.
//!
//! The layout rules follow the System V AMD64 ABI that Forge targets:
//!
//! * Natural alignment with trailing tail padding to the struct's
//!   alignment.
//! * One storage unit per bit-field of the declared base type
//!   (simplified from the full ABI packing rules — runs of same-type
//!   bit-fields still fit inside one unit).
//! * Flexible array members (`T arr[];`) occupy zero bytes and must
//!   appear last.  C17 §6.7.2.1p18 structural restrictions on FAM
//!   structs are enforced here.
//! * C11 anonymous struct/union members flatten their inner field
//!   names into the outer type so `outer.inner_field` is legal.
//! * Enum underlying type is selected from the narrowest standard
//!   integer type that contains every enumerator value, matching
//!   GCC/Clang behaviour.

use forge_diagnostics::Diagnostic;
use forge_lexer::Span;
use forge_parser::ast::{EnumDef, Enumerator, StructDef, StructField, StructMember, StructOrUnion};

use crate::const_eval::eval_icx_as_i64;
use crate::context::SemaContext;
use crate::resolve::{resolve_declarator, resolve_type_specifiers};
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::types::{
    AnonMemberMap, ArraySize, BitFieldLayout, EnumLayout, EnumTypeId, MemberLayout, QualType,
    StructLayout, StructTypeId, TargetInfo, Type, TypeContext, UnionLayout, UnionTypeId,
};

// =========================================================================
// Public entry points
// =========================================================================

/// Complete a registered-but-incomplete struct.
///
/// The caller must have already allocated a [`StructTypeId`] via
/// [`TypeContext::fresh_struct_id`] and installed a placeholder layout
/// (so that members can reference the struct via pointer).  This
/// function walks the member list, lays out fields, validates flexible
/// array members, and installs the completed [`StructLayout`].
pub fn complete_struct(
    id: StructTypeId,
    def: &StructDef,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    debug_assert!(matches!(def.kind, StructOrUnion::Struct));
    let Some(members_src) = def.members.as_ref() else {
        return;
    };

    let mut builder = StructBuilder::new(def.name.clone());
    let mut named_count: u32 = 0;

    for member in members_src {
        match member {
            StructMember::Field(field) => {
                lay_out_field(&mut builder, field, table, target, ctx, &mut named_count);
            }
            StructMember::StaticAssert(sa) => {
                let cond = eval_icx_as_i64(&sa.condition, table, target, ctx).unwrap_or(1);
                if cond == 0 {
                    let msg = sa
                        .message
                        .clone()
                        .unwrap_or_else(|| "static assertion failed".into());
                    ctx.emit(Diagnostic::error(msg).span(sa.span));
                }
            }
        }
    }

    // FAM structural rule (a): a struct with a flexible array member
    // cannot consist SOLELY of the flexible array member.
    if builder.has_flexible_array && named_count <= 1 {
        ctx.emit(
            Diagnostic::error(
                "a struct with a flexible array member must have at least one other named member",
            )
            .span(def.span),
        );
    }

    let layout = builder.finish();
    ctx.type_ctx.set_struct(id, layout);
}

/// Complete a registered-but-incomplete union.
pub fn complete_union(
    id: UnionTypeId,
    def: &StructDef,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    debug_assert!(matches!(def.kind, StructOrUnion::Union));
    let Some(members_src) = def.members.as_ref() else {
        return;
    };

    let mut total_size: u64 = 0;
    let mut alignment: u64 = 1;
    let mut members: Vec<MemberLayout> = Vec::new();

    for member in members_src {
        match member {
            StructMember::Field(field) => {
                lay_out_union_field(
                    field,
                    table,
                    target,
                    ctx,
                    &mut members,
                    &mut total_size,
                    &mut alignment,
                );
            }
            StructMember::StaticAssert(sa) => {
                let cond = eval_icx_as_i64(&sa.condition, table, target, ctx).unwrap_or(1);
                if cond == 0 {
                    let msg = sa
                        .message
                        .clone()
                        .unwrap_or_else(|| "static assertion failed".into());
                    ctx.emit(Diagnostic::error(msg).span(sa.span));
                }
            }
        }
    }

    let total_size = align_up(total_size, alignment.max(1));

    let layout = UnionLayout {
        tag: def.name.clone(),
        members,
        total_size,
        alignment: alignment.max(1),
        is_complete: true,
    };
    ctx.type_ctx.set_union(id, layout);
}

/// Complete a registered-but-incomplete enum.
///
/// Walks the enumerator list, assigning successive values (starting at
/// 0, or one past the previous explicit value), registers each
/// enumerator in the current scope as an ordinary-namespace symbol, and
/// chooses an underlying integer type that contains every value.
pub fn complete_enum(
    id: EnumTypeId,
    def: &EnumDef,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
) {
    let Some(enumerators) = def.enumerators.as_ref() else {
        return;
    };

    let mut next_value: i64 = 0;
    let mut constants: Vec<(String, i64)> = Vec::new();

    // Pre-install a placeholder so `EnumConstant` references during the
    // enum body can resolve to this id.  The placeholder is finalised
    // after the loop once `underlying_type` is known.
    ctx.type_ctx.set_enum(
        id,
        EnumLayout {
            tag: def.name.clone(),
            ..EnumLayout::default()
        },
    );

    for Enumerator {
        name, value, span, ..
    } in enumerators
    {
        let v = if let Some(expr) = value {
            eval_icx_as_i64(expr, table, target, ctx).unwrap_or(next_value)
        } else {
            next_value
        };

        constants.push((name.clone(), v));

        let sym = Symbol {
            id: 0,
            name: name.clone(),
            ty: QualType::unqualified(Type::Int { is_unsigned: false }),
            kind: SymbolKind::EnumConstant {
                value: v,
                enum_id: id,
            },
            storage: StorageClass::None,
            linkage: Linkage::None,
            span: *span,
            is_defined: true,
            is_inline: false,
            is_noreturn: false,
            has_noreturn_attr: false,
        };
        let _ = table.declare(sym, ctx);

        next_value = v.wrapping_add(1);
    }

    let underlying = select_enum_underlying(&constants);

    ctx.type_ctx.set_enum(
        id,
        EnumLayout {
            tag: def.name.clone(),
            constants,
            underlying_type: Some(underlying),
            is_complete: true,
        },
    );
}

// =========================================================================
// Struct-builder internals
// =========================================================================

struct StructBuilder {
    tag: Option<String>,
    members: Vec<MemberLayout>,
    offset: u64,
    alignment: u64,
    has_flexible_array: bool,
}

impl StructBuilder {
    fn new(tag: Option<String>) -> Self {
        Self {
            tag,
            members: Vec::new(),
            offset: 0,
            alignment: 1,
            has_flexible_array: false,
        }
    }

    fn finish(self) -> StructLayout {
        let total_size = align_up(self.offset, self.alignment.max(1));
        StructLayout {
            tag: self.tag,
            members: self.members,
            total_size,
            alignment: self.alignment.max(1),
            is_packed: false,
            is_complete: true,
            has_flexible_array: self.has_flexible_array,
        }
    }
}

fn lay_out_field(
    b: &mut StructBuilder,
    field: &StructField,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
    named_count: &mut u32,
) {
    let Some(base) = resolve_type_specifiers(&field.specifiers, table, target, ctx) else {
        return;
    };

    // `struct S { struct { int x; }; };` — an anonymous tag member
    // with no declarators contributes its fields directly.
    if field.declarators.is_empty() {
        if let Some(member) = build_anon_aggregate(base.clone(), &ctx.type_ctx) {
            place_member(b, member, target, ctx, field.span);
            *named_count += 1;
        }
        return;
    }

    for sfd in &field.declarators {
        let mut ty = base.clone();
        let mut name: Option<String> = None;

        if let Some(decl) = &sfd.declarator {
            let Some((decl_name, resolved)) =
                resolve_declarator(ty.clone(), decl, false, table, target, ctx)
            else {
                continue;
            };
            ty = resolved;
            name = decl_name;
        } else if sfd.bit_width.is_none() {
            ctx.emit(
                Diagnostic::error("struct member must have a name or a bit-field width")
                    .span(sfd.span),
            );
            continue;
        }

        if let Some(width_expr) = sfd.bit_width.as_deref() {
            let width = eval_icx_as_i64(width_expr, table, target, ctx).unwrap_or(0);
            place_bit_field(b, name, ty, width, target, ctx, sfd.span);
            *named_count += 1;
        } else {
            let member = MemberLayout {
                name,
                ty,
                offset: 0,
                bit_field: None,
                anon_members: None,
            };
            place_member(b, member, target, ctx, sfd.span);
            *named_count += 1;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn lay_out_union_field(
    field: &StructField,
    table: &mut SymbolTable,
    target: &TargetInfo,
    ctx: &mut SemaContext,
    members: &mut Vec<MemberLayout>,
    total_size: &mut u64,
    alignment: &mut u64,
) {
    let Some(base) = resolve_type_specifiers(&field.specifiers, table, target, ctx) else {
        return;
    };

    if field.declarators.is_empty() {
        if let Some(member) = build_anon_aggregate(base.clone(), &ctx.type_ctx) {
            update_union_size(&member, target, &ctx.type_ctx, total_size, alignment);
            members.push(member);
        }
        return;
    }

    for sfd in &field.declarators {
        let mut ty = base.clone();
        let mut name: Option<String> = None;

        if let Some(decl) = &sfd.declarator {
            let Some((decl_name, resolved)) =
                resolve_declarator(ty.clone(), decl, false, table, target, ctx)
            else {
                continue;
            };
            ty = resolved;
            name = decl_name;
        }

        let member = if let Some(width_expr) = sfd.bit_width.as_deref() {
            let width = eval_icx_as_i64(width_expr, table, target, ctx).unwrap_or(0);
            if width < 0 {
                ctx.emit(Diagnostic::error("bit-field width cannot be negative").span(sfd.span));
            }
            let storage_unit_size = ty.ty.size_of(target, &ctx.type_ctx).unwrap_or(4) as u32;
            MemberLayout {
                name,
                ty,
                offset: 0,
                bit_field: Some(BitFieldLayout {
                    width: width.max(0) as u32,
                    bit_offset: 0,
                    storage_unit_size,
                }),
                anon_members: None,
            }
        } else {
            MemberLayout {
                name,
                ty,
                offset: 0,
                bit_field: None,
                anon_members: None,
            }
        };

        update_union_size(&member, target, &ctx.type_ctx, total_size, alignment);
        members.push(member);
    }
}

fn update_union_size(
    member: &MemberLayout,
    target: &TargetInfo,
    tctx: &TypeContext,
    total_size: &mut u64,
    alignment: &mut u64,
) {
    let size = match &member.bit_field {
        Some(bf) => u64::from(bf.storage_unit_size),
        None => member.ty.ty.size_of(target, tctx).unwrap_or(0),
    };
    let align = match &member.bit_field {
        Some(bf) => u64::from(bf.storage_unit_size),
        None => member.ty.ty.align_of(target, tctx).unwrap_or(1),
    };
    *total_size = (*total_size).max(size);
    *alignment = (*alignment).max(align.max(1));
}

fn place_member(
    b: &mut StructBuilder,
    mut member: MemberLayout,
    target: &TargetInfo,
    ctx: &mut SemaContext,
    span: Span,
) {
    let is_incomplete_array = matches!(
        &member.ty.ty,
        Type::Array {
            size: ArraySize::Incomplete,
            ..
        }
    );
    let contains_fam = type_has_flexible_array(&member.ty, &ctx.type_ctx);

    // If a previously placed member was a FAM, any subsequent member is
    // an error (FAM must appear last — rule c).
    if b.has_flexible_array {
        ctx.emit(
            Diagnostic::error("a flexible array member must appear at the end of a struct")
                .span(span),
        );
    }

    if is_incomplete_array {
        let align = member_align(&member, target, &ctx.type_ctx);
        member.offset = align_up(b.offset, align);
        b.alignment = b.alignment.max(align);
        b.has_flexible_array = true;
        b.members.push(member);
        return;
    }

    // A nested struct that itself ends in a FAM may only be placed as
    // the last member (rule c).  We flag the outer struct as
    // FAM-bearing; any subsequent member will trigger the diagnostic
    // above.
    if contains_fam {
        b.has_flexible_array = true;
    }

    let size = member_size(&member, target, &ctx.type_ctx);
    let align = member_align(&member, target, &ctx.type_ctx);

    // Self-containment (by value) check — `struct X { struct X x; }`.
    if matches!(
        &member.ty.ty,
        Type::Struct(_) | Type::Union(_) | Type::Array { .. }
    ) && size == 0
    {
        ctx.emit(
            Diagnostic::error("struct member has incomplete type (recursive inclusion by value?)")
                .span(span),
        );
    }

    let offset = align_up(b.offset, align.max(1));
    member.offset = offset;
    b.alignment = b.alignment.max(align.max(1));
    b.offset = offset.saturating_add(size);
    b.members.push(member);
}

#[allow(clippy::too_many_arguments)]
fn place_bit_field(
    b: &mut StructBuilder,
    name: Option<String>,
    ty: QualType,
    width: i64,
    target: &TargetInfo,
    ctx: &mut SemaContext,
    span: Span,
) {
    if !ty.ty.is_integer() {
        ctx.emit(Diagnostic::error("bit-field must have an integer type").span(span));
        return;
    }
    if width < 0 {
        ctx.emit(Diagnostic::error("bit-field width cannot be negative").span(span));
        return;
    }
    let width = width as u32;
    let storage_bytes = ty.ty.size_of(target, &ctx.type_ctx).unwrap_or(4);
    let storage_bits = (storage_bytes.saturating_mul(8)) as u32;
    if width > storage_bits {
        ctx.emit(
            Diagnostic::error(format!(
                "bit-field width ({width}) exceeds the width of its type"
            ))
            .span(span),
        );
        return;
    }

    let align = storage_bytes.max(1);

    // Zero-width bit-field: forces the next field to a fresh storage
    // unit boundary.
    if width == 0 {
        let aligned = align_up(b.offset, align);
        b.offset = aligned;
        b.alignment = b.alignment.max(align);
        b.members.push(MemberLayout {
            name,
            ty,
            offset: aligned,
            bit_field: Some(BitFieldLayout {
                width: 0,
                bit_offset: 0,
                storage_unit_size: storage_bytes as u32,
            }),
            anon_members: None,
        });
        return;
    }

    // Try to pack into the previously opened storage unit when the
    // previous member is a same-storage-size bit-field and there is
    // still room in its unit.
    let mut bit_offset: u32 = 0;
    let mut open_new_unit = true;
    if let Some(last) = b.members.last() {
        if let Some(last_bf) = &last.bit_field {
            let same_storage = u64::from(last_bf.storage_unit_size) == storage_bytes;
            let used = last_bf.bit_offset + last_bf.width;
            if same_storage && used + width <= storage_bits {
                bit_offset = used;
                open_new_unit = false;
            }
        }
    }

    let offset = if open_new_unit {
        align_up(b.offset, align)
    } else {
        // Reuse the last bit-field's storage-unit offset.
        b.members
            .last()
            .map(|m| m.offset)
            .unwrap_or_else(|| align_up(b.offset, align))
    };

    b.alignment = b.alignment.max(align);

    b.members.push(MemberLayout {
        name,
        ty,
        offset,
        bit_field: Some(BitFieldLayout {
            width,
            bit_offset,
            storage_unit_size: storage_bytes as u32,
        }),
        anon_members: None,
    });

    if open_new_unit {
        b.offset = offset.saturating_add(storage_bytes);
    } else {
        // Packing continues into the same storage unit; advance the
        // working offset only to the end of that unit (not beyond).
        b.offset = offset.saturating_add(storage_bytes).max(b.offset);
    }
}

// =========================================================================
// Anonymous struct/union members (C11)
// =========================================================================

fn build_anon_aggregate(base: QualType, tctx: &TypeContext) -> Option<MemberLayout> {
    match &base.ty {
        Type::Struct(sid) => {
            let layout = tctx.struct_layout(*sid)?;
            // Only tag-less inner aggregates propagate as C11 anonymous
            // members; a tagged nested struct with no declarator is a
            // stray tag declaration rather than an anonymous member.
            if layout.tag.is_some() {
                return None;
            }
            let mut map = AnonMemberMap::default();
            flatten_struct(layout, 0, &mut map);
            Some(MemberLayout {
                name: None,
                ty: base,
                offset: 0,
                bit_field: None,
                anon_members: Some(map),
            })
        }
        Type::Union(uid) => {
            let layout = tctx.union_layout(*uid)?;
            if layout.tag.is_some() {
                return None;
            }
            let mut map = AnonMemberMap::default();
            flatten_union(layout, 0, &mut map);
            Some(MemberLayout {
                name: None,
                ty: base,
                offset: 0,
                bit_field: None,
                anon_members: Some(map),
            })
        }
        _ => None,
    }
}

fn flatten_struct(layout: &StructLayout, base_offset: u64, out: &mut AnonMemberMap) {
    for m in &layout.members {
        let abs = base_offset + m.offset;
        if let Some(name) = &m.name {
            out.fields.insert(name.clone(), (abs, m.ty.clone()));
        } else if let Some(nested) = &m.anon_members {
            for (k, (off, ty)) in &nested.fields {
                out.fields
                    .insert(k.clone(), (base_offset + off, ty.clone()));
            }
        }
    }
}

fn flatten_union(layout: &UnionLayout, base_offset: u64, out: &mut AnonMemberMap) {
    for m in &layout.members {
        if let Some(name) = &m.name {
            out.fields.insert(name.clone(), (base_offset, m.ty.clone()));
        } else if let Some(nested) = &m.anon_members {
            for (k, (off, ty)) in &nested.fields {
                out.fields
                    .insert(k.clone(), (base_offset + off, ty.clone()));
            }
        }
    }
}

// =========================================================================
// Enum helpers
// =========================================================================

fn select_enum_underlying(constants: &[(String, i64)]) -> Type {
    if constants.is_empty() {
        return Type::Int { is_unsigned: false };
    }
    let (mut min, mut max) = (i64::MAX, i64::MIN);
    for (_, v) in constants {
        if *v < min {
            min = *v;
        }
        if *v > max {
            max = *v;
        }
    }
    let any_negative = min < 0;

    if any_negative {
        if min >= i64::from(i32::MIN) && max <= i64::from(i32::MAX) {
            Type::Int { is_unsigned: false }
        } else {
            Type::Long { is_unsigned: false }
        }
    } else {
        let max_u = max as u64;
        if max_u <= i32::MAX as u64 {
            Type::Int { is_unsigned: false }
        } else if max_u <= u32::MAX as u64 {
            Type::Int { is_unsigned: true }
        } else if max_u <= i64::MAX as u64 {
            Type::Long { is_unsigned: false }
        } else {
            Type::Long { is_unsigned: true }
        }
    }
}

// =========================================================================
// Miscellaneous helpers
// =========================================================================

pub(crate) fn align_up(offset: u64, align: u64) -> u64 {
    if align == 0 {
        return offset;
    }
    let mask = align - 1;
    (offset + mask) & !mask
}

fn member_size(member: &MemberLayout, target: &TargetInfo, tctx: &TypeContext) -> u64 {
    if let Some(bf) = &member.bit_field {
        return u64::from(bf.storage_unit_size);
    }
    member.ty.ty.size_of(target, tctx).unwrap_or(0)
}

fn member_align(member: &MemberLayout, target: &TargetInfo, tctx: &TypeContext) -> u64 {
    if let Some(align) = member.ty.explicit_align {
        return align;
    }
    if let Some(bf) = &member.bit_field {
        return u64::from(bf.storage_unit_size.max(1));
    }
    member.ty.ty.align_of(target, tctx).unwrap_or(1)
}

fn type_has_flexible_array(qt: &QualType, tctx: &TypeContext) -> bool {
    match &qt.ty {
        Type::Struct(sid) => tctx
            .struct_layout(*sid)
            .is_some_and(|s| s.has_flexible_array),
        _ => false,
    }
}
