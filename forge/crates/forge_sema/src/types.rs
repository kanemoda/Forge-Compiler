//! Core C17 type system.
//!
//! This module defines the semantic representation of C types used by
//! sema and every downstream phase.  The representation mirrors the
//! abstract grammar of C17 §6.2.5 — scalar arithmetic types, derived
//! types (pointer / array / function), and user-defined compound types
//! (struct / union / enum).
//!
//! ## Identity
//!
//! * [`Type`] is a *structural* value — two `Type`s are equal iff their
//!   shape is equal.
//! * [`QualType`] layers the four C qualifiers (`const`, `volatile`,
//!   `restrict`, `_Atomic`) and the `_Alignas` override on top of a
//!   `Type`.
//! * User-defined compound types are referenced by opaque IDs
//!   ([`StructTypeId`], [`UnionTypeId`], [`EnumTypeId`]) — the actual
//!   member layout lives in a separately-maintained [`TypeContext`].
//!
//! ## Size and alignment
//!
//! All size / alignment queries are parameterised on a [`TargetInfo`]
//! because C leaves these implementation-defined.  The crate currently
//! only ships [`TargetInfo::x86_64_linux`]; other targets will grow as
//! code generation does.
//!
//! ## Implicit conversions
//!
//! The three canonical C conversion algorithms live here:
//!
//! * [`integer_promotion`] — C17 §6.3.1.1.  Bool/char/short → int.
//! * [`usual_arithmetic_conversions`] — C17 §6.3.1.8.  The binary
//!   operator workhorse: decides the common type of two operands.
//! * [`are_compatible`] / [`composite_type`] — C17 §6.2.7.  Comparing
//!   and merging types across declarations and prototypes.

use std::collections::HashMap;
use std::fmt;

// =========================================================================
// ID newtypes for user-defined tag types
// =========================================================================

/// Opaque identifier for a `struct` type.  Actual layout lives in a
/// [`TypeContext`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct StructTypeId(pub u32);

/// Opaque identifier for a `union` type.  Actual layout lives in a
/// [`TypeContext`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct UnionTypeId(pub u32);

/// Opaque identifier for an `enum` type.  Actual layout lives in a
/// [`TypeContext`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumTypeId(pub u32);

// =========================================================================
// Ancillary type data
// =========================================================================

/// The C17 signedness of a `char`.
///
/// Plain `char` is a *distinct* type from `signed char` and `unsigned
/// char` even though it aliases one of them at the ABI level.  Whether
/// plain `char` is signed is implementation-defined (see
/// [`TargetInfo::char_is_signed`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Signedness {
    /// Plain `char` — ABI-dependent signedness.
    Plain,
    /// Explicit `signed char`.
    Signed,
    /// Explicit `unsigned char`.
    Unsigned,
}

/// The size component of an array type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArraySize {
    /// `int arr[10]` — compile-time constant size.
    Fixed(u64),
    /// `int arr[n]` — variable-length array (size expression tracked
    /// elsewhere for codegen).
    Variable,
    /// `int arr[]` — unknown size, used by `extern` arrays and flexible
    /// array members.
    Incomplete,
    /// `int arr[*]` — prototype-scope VLA placeholder.
    Star,
}

/// A single parameter in a function type.
///
/// Names are purely cosmetic — they do not participate in type
/// compatibility but are carried for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamType {
    /// The declared parameter name, if any.
    pub name: Option<String>,
    /// The parameter's qualified type.
    pub ty: QualType,
    /// `true` when the source spelled the parameter as `T arr[static N]`
    /// (C99/C17 §6.7.6.3p7).  The pointer type is unchanged; this flag
    /// merely records the caller-side "at least N elements" guarantee
    /// so later phases can exploit it.
    pub has_static_size: bool,
}

// =========================================================================
// Type
// =========================================================================

/// A C17 type, unqualified.
///
/// See [`QualType`] for the qualifier-carrying wrapper actually used
/// everywhere in sema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// `void` — always incomplete.
    Void,
    /// `_Bool` — 0 or 1.
    Bool,
    /// `char` / `signed char` / `unsigned char`.
    Char {
        /// Whether the char was written as plain, signed, or unsigned.
        signedness: Signedness,
    },
    /// `short` / `unsigned short`.
    Short {
        /// `true` if `unsigned short`.
        is_unsigned: bool,
    },
    /// `int` / `unsigned int`.
    Int {
        /// `true` if `unsigned int`.
        is_unsigned: bool,
    },
    /// `long` / `unsigned long`.
    Long {
        /// `true` if `unsigned long`.
        is_unsigned: bool,
    },
    /// `long long` / `unsigned long long`.
    LongLong {
        /// `true` if `unsigned long long`.
        is_unsigned: bool,
    },
    /// `float`.
    Float,
    /// `double`.
    Double,
    /// `long double`.
    LongDouble,
    /// `T *` — pointer to a possibly-qualified type.
    Pointer {
        /// The pointee, with its own qualifiers.
        pointee: Box<QualType>,
    },
    /// `T[N]` / `T[]` / `T[*]`.
    Array {
        /// The element type (qualified).
        element: Box<QualType>,
        /// Size shape — fixed, VLA, incomplete, or `*`.
        size: ArraySize,
    },
    /// A function type.  Not a value type — functions always appear
    /// behind a pointer, except as an operand of the address-of
    /// operator.
    Function {
        /// The return type.
        return_type: Box<QualType>,
        /// Parameters, in source order.
        params: Vec<ParamType>,
        /// `true` for `...` trailing.
        is_variadic: bool,
        /// `false` for old-style `int f()` declarations with no
        /// prototype.  Compatibility uses this flag.
        is_prototype: bool,
    },
    /// A named or anonymous `struct` type, referenced by ID.
    Struct(StructTypeId),
    /// A named or anonymous `union` type, referenced by ID.
    Union(UnionTypeId),
    /// A named or anonymous `enum` type, referenced by ID.
    Enum(EnumTypeId),
}

// =========================================================================
// QualType
// =========================================================================

/// A C type annotated with its qualifiers and optional explicit
/// alignment.
///
/// Constructed through [`QualType::unqualified`] or by direct field
/// assignment.  The `Display` impl renders the type in a C-ish syntax
/// useful for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualType {
    /// The unqualified underlying type.
    pub ty: Type,
    /// `const T`.
    pub is_const: bool,
    /// `volatile T`.
    pub is_volatile: bool,
    /// `restrict T` (only legal on pointer types in C, but we carry it
    /// uniformly).
    pub is_restrict: bool,
    /// `_Atomic T`.
    pub is_atomic: bool,
    /// Explicit `_Alignas` / `__attribute__((aligned(N)))` override.
    pub explicit_align: Option<u64>,
}

impl QualType {
    /// Wrap `ty` without any qualifiers or explicit alignment.
    pub fn unqualified(ty: Type) -> Self {
        Self {
            ty,
            is_const: false,
            is_volatile: false,
            is_restrict: false,
            is_atomic: false,
            explicit_align: None,
        }
    }

    /// Return `self` with `is_const` set.  Other qualifiers are
    /// preserved.
    pub fn with_const(mut self) -> Self {
        self.is_const = true;
        self
    }

    /// Return the underlying unqualified type by value.
    ///
    /// The returned [`Type`] shares no mutable state with `self`.
    pub fn strip_qualifiers(&self) -> Type {
        self.ty.clone()
    }

    /// `true` if any of `const`, `volatile`, `restrict`, `_Atomic` is
    /// set.  Explicit alignment does not count as a qualifier.
    pub fn has_any_qualifier(&self) -> bool {
        self.is_const || self.is_volatile || self.is_restrict || self.is_atomic
    }

    /// Pretty-print the type in C syntax, using `ctx` to resolve
    /// struct / union / enum tag names.  Preferred over `to_string()`
    /// for user-facing diagnostics.
    pub fn to_c_string(&self, ctx: &TypeContext) -> String {
        let mut out = String::new();
        write_qualtype(self, &mut out, Some(ctx));
        out
    }

    fn qualifier_mask(&self) -> u8 {
        (u8::from(self.is_const))
            | (u8::from(self.is_volatile) << 1)
            | (u8::from(self.is_restrict) << 2)
            | (u8::from(self.is_atomic) << 3)
    }
}

// =========================================================================
// Type predicates
// =========================================================================

impl Type {
    /// `true` if this is the `void` type.
    pub fn is_void(&self) -> bool {
        matches!(self, Type::Void)
    }

    /// `true` if this is `_Bool`.
    pub fn is_bool(&self) -> bool {
        matches!(self, Type::Bool)
    }

    /// `true` if this is any integer type (including `_Bool`, `char`,
    /// and `enum`).
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Type::Bool
                | Type::Char { .. }
                | Type::Short { .. }
                | Type::Int { .. }
                | Type::Long { .. }
                | Type::LongLong { .. }
                | Type::Enum(_)
        )
    }

    /// `true` if this is a floating-point type.
    pub fn is_floating(&self) -> bool {
        matches!(self, Type::Float | Type::Double | Type::LongDouble)
    }

    /// `true` if this is an arithmetic type (integer or floating-point).
    pub fn is_arithmetic(&self) -> bool {
        self.is_integer() || self.is_floating()
    }

    /// `true` if this is a scalar type (arithmetic or pointer).
    pub fn is_scalar(&self) -> bool {
        self.is_arithmetic() || self.is_pointer()
    }

    /// `true` if this is a pointer type.
    pub fn is_pointer(&self) -> bool {
        matches!(self, Type::Pointer { .. })
    }

    /// `true` if this is an array type.
    pub fn is_array(&self) -> bool {
        matches!(self, Type::Array { .. })
    }

    /// `true` if this is a function type.
    pub fn is_function(&self) -> bool {
        matches!(self, Type::Function { .. })
    }

    /// `true` if this is a `struct` type.
    pub fn is_struct(&self) -> bool {
        matches!(self, Type::Struct(_))
    }

    /// `true` if this is a `union` type.
    pub fn is_union(&self) -> bool {
        matches!(self, Type::Union(_))
    }

    /// `true` if this is a `struct` or `union` type.
    pub fn is_struct_or_union(&self) -> bool {
        self.is_struct() || self.is_union()
    }

    /// `true` if the type is *complete* — its storage size is known.
    ///
    /// `void`, incomplete-array types (`T[]`, `T[*]`), and functions
    /// are always incomplete.  Struct / union completeness is delegated
    /// to `ctx`.
    pub fn is_complete(&self, ctx: &TypeContext) -> bool {
        match self {
            Type::Void => false,
            Type::Function { .. } => false,
            Type::Array { size, element } => match size {
                ArraySize::Incomplete | ArraySize::Star => false,
                _ => element.ty.is_complete(ctx),
            },
            Type::Struct(id) => ctx.is_struct_complete(*id),
            Type::Union(id) => ctx.is_union_complete(*id),
            _ => true,
        }
    }

    /// `true` if this integer type is unsigned.
    ///
    /// `_Bool` is treated as unsigned per C17 §6.2.5.  Plain `char` is
    /// *not* reported as unsigned even when the ABI chooses that
    /// signedness — use the target-aware overload for that.
    pub fn is_unsigned(&self) -> bool {
        match self {
            Type::Bool => true,
            Type::Char {
                signedness: Signedness::Unsigned,
            } => true,
            Type::Short { is_unsigned }
            | Type::Int { is_unsigned }
            | Type::Long { is_unsigned }
            | Type::LongLong { is_unsigned } => *is_unsigned,
            _ => false,
        }
    }

    /// The C17 integer conversion rank (§6.3.1.1).
    ///
    /// Returns `0` for non-integer types — callers should gate on
    /// [`Type::is_integer`] first.
    pub fn integer_rank(&self) -> u8 {
        match self {
            Type::Bool => 0,
            Type::Char { .. } => 1,
            Type::Short { .. } => 2,
            Type::Int { .. } | Type::Enum(_) => 3,
            Type::Long { .. } => 4,
            Type::LongLong { .. } => 5,
            _ => 0,
        }
    }

    /// Size in bytes, or `None` for incomplete / sizeless types.
    pub fn size_of(&self, target: &TargetInfo, ctx: &TypeContext) -> Option<u64> {
        match self {
            Type::Void => None,
            Type::Function { .. } => None,
            Type::Bool => Some(target.bool_size),
            Type::Char { .. } => Some(target.char_size),
            Type::Short { .. } => Some(target.short_size),
            Type::Int { .. } => Some(target.int_size),
            Type::Long { .. } => Some(target.long_size),
            Type::LongLong { .. } => Some(target.long_long_size),
            Type::Float => Some(target.float_size),
            Type::Double => Some(target.double_size),
            Type::LongDouble => Some(target.long_double_size),
            Type::Pointer { .. } => Some(target.pointer_size),
            Type::Array { element, size } => {
                let ArraySize::Fixed(n) = size else {
                    return None;
                };
                let elem = element.ty.size_of(target, ctx)?;
                elem.checked_mul(*n)
            }
            Type::Struct(id) => ctx.struct_size(*id),
            Type::Union(id) => ctx.union_size(*id),
            Type::Enum(id) => ctx.enum_size(*id).or(Some(target.int_size)),
        }
    }

    /// Alignment in bytes, or `None` for incomplete / alignless types.
    pub fn align_of(&self, target: &TargetInfo, ctx: &TypeContext) -> Option<u64> {
        match self {
            Type::Void => None,
            Type::Function { .. } => None,
            Type::Bool => Some(target.bool_size),
            Type::Char { .. } => Some(target.char_size),
            Type::Short { .. } => Some(target.short_size),
            Type::Int { .. } => Some(target.int_size),
            Type::Long { .. } => Some(target.long_size),
            Type::LongLong { .. } => Some(target.long_long_size),
            Type::Float => Some(target.float_size),
            Type::Double => Some(target.double_size),
            Type::LongDouble => Some(target.long_double_align),
            Type::Pointer { .. } => Some(target.pointer_align),
            Type::Array { element, .. } => element.ty.align_of(target, ctx),
            Type::Struct(id) => ctx.struct_align(*id),
            Type::Union(id) => ctx.union_align(*id),
            Type::Enum(id) => ctx.enum_align(*id).or(Some(target.int_size)),
        }
    }
}

/// Determine whether an expression with type `expr_type` and
/// (optionally known) integer value `const_value` forms a *null pointer
/// constant* per C17 §6.3.2.3.
///
/// The rule is narrow: an integer constant expression with the value
/// 0, or that expression cast to `void *`.
pub fn is_null_pointer_constant(expr_type: &Type, const_value: Option<i64>) -> bool {
    match expr_type {
        t if t.is_integer() => const_value == Some(0),
        Type::Pointer { pointee } if matches!(pointee.ty, Type::Void) => const_value == Some(0),
        _ => false,
    }
}

// =========================================================================
// TargetInfo
// =========================================================================

/// ABI-specific sizes and alignments.
///
/// The type system queries this struct whenever a question about "how
/// big is an `int`?" arises.  The only target provided today is
/// [`TargetInfo::x86_64_linux`]; others will be added as code
/// generation grows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetInfo {
    /// Size of a pointer.
    pub pointer_size: u64,
    /// Alignment of a pointer.
    pub pointer_align: u64,
    /// `true` if plain `char` is signed on this target.
    pub char_is_signed: bool,
    /// Size of `_Bool`.
    pub bool_size: u64,
    /// Size of `char`.
    pub char_size: u64,
    /// Size of `short`.
    pub short_size: u64,
    /// Size of `int`.
    pub int_size: u64,
    /// Size of `long`.
    pub long_size: u64,
    /// Size of `long long`.
    pub long_long_size: u64,
    /// Size of `float`.
    pub float_size: u64,
    /// Size of `double`.
    pub double_size: u64,
    /// Size of `long double`.
    pub long_double_size: u64,
    /// Alignment of `long double` (often larger than its size on x86).
    pub long_double_align: u64,
    /// Maximum fundamental alignment (`alignof(max_align_t)`).
    pub max_align: u64,
}

impl TargetInfo {
    /// x86-64 Linux LP64 ABI.
    pub fn x86_64_linux() -> Self {
        Self {
            pointer_size: 8,
            pointer_align: 8,
            char_is_signed: true,
            bool_size: 1,
            char_size: 1,
            short_size: 2,
            int_size: 4,
            long_size: 8,
            long_long_size: 8,
            float_size: 4,
            double_size: 8,
            long_double_size: 16,
            long_double_align: 16,
            max_align: 16,
        }
    }

    /// The underlying integer type of `size_t` on this target.
    pub fn size_t_type(&self) -> Type {
        // On LP64 this is `unsigned long`.
        if self.long_size == self.pointer_size {
            Type::Long { is_unsigned: true }
        } else if self.long_long_size == self.pointer_size {
            Type::LongLong { is_unsigned: true }
        } else {
            Type::Int { is_unsigned: true }
        }
    }

    /// The underlying integer type of `ptrdiff_t` on this target.
    pub fn ptrdiff_t_type(&self) -> Type {
        if self.long_size == self.pointer_size {
            Type::Long { is_unsigned: false }
        } else if self.long_long_size == self.pointer_size {
            Type::LongLong { is_unsigned: false }
        } else {
            Type::Int { is_unsigned: false }
        }
    }

    /// The underlying integer type of `wchar_t`.  On x86-64 Linux this
    /// is `int`; on Windows it would be `unsigned short`.
    pub fn wchar_t_type(&self) -> Type {
        Type::Int { is_unsigned: false }
    }
}

// =========================================================================
// TypeContext and layouts
// =========================================================================

/// Per-translation-unit side table holding the definition of every
/// `struct`, `union`, and `enum` type.
///
/// The `Type` enum only stores opaque IDs; the real layout — member
/// offsets, bit-field packing, the enum's underlying integer type —
/// lives on the [`StructLayout`] / [`UnionLayout`] / [`EnumLayout`]
/// entries registered here.
#[derive(Default, Debug, Clone)]
pub struct TypeContext {
    structs: HashMap<u32, StructLayout>,
    unions: HashMap<u32, UnionLayout>,
    enums: HashMap<u32, EnumLayout>,
    next_struct_id: u32,
    next_union_id: u32,
    next_enum_id: u32,
}

/// Full layout information for a single `struct` type.
///
/// Starts life *incomplete* when the parser first sees the tag (so
/// self-referential types can resolve `struct node *`), and is replaced
/// in place once the member list is analysed.
#[derive(Default, Debug, Clone)]
pub struct StructLayout {
    /// Source tag, if any (`None` for anonymous compounds).
    pub tag: Option<String>,
    /// Members, in declaration order.  Empty for an incomplete
    /// forward-declared tag.
    pub members: Vec<MemberLayout>,
    /// Total size in bytes, padded to `alignment`.
    pub total_size: u64,
    /// Struct alignment in bytes.
    pub alignment: u64,
    /// `true` if `__attribute__((packed))` disables natural alignment
    /// padding.  Currently always `false`; hook for future work.
    pub is_packed: bool,
    /// `true` once the body has been laid out.  Incomplete tags have
    /// `is_complete = false`.
    pub is_complete: bool,
    /// `true` if the last member is a flexible array member
    /// (`T arr[];`).  Propagated to any struct that contains this one
    /// as its last member.
    pub has_flexible_array: bool,
}

/// Full layout information for a single `union` type.
///
/// All members start at offset 0; the union's size is the maximum of
/// its members' sizes, and its alignment is the maximum of theirs.
#[derive(Default, Debug, Clone)]
pub struct UnionLayout {
    /// Source tag, if any (`None` for anonymous compounds).
    pub tag: Option<String>,
    /// Members (all at offset 0).
    pub members: Vec<MemberLayout>,
    /// Total size in bytes, padded to `alignment`.
    pub total_size: u64,
    /// Union alignment in bytes.
    pub alignment: u64,
    /// `true` once the body has been laid out.
    pub is_complete: bool,
}

/// Full layout information for a single `enum` type.
#[derive(Default, Debug, Clone)]
pub struct EnumLayout {
    /// Source tag, if any (`None` for anonymous enums).
    pub tag: Option<String>,
    /// Enumerator (name, value) pairs in declaration order.
    pub constants: Vec<(String, i64)>,
    /// Underlying standard integer type chosen to contain every value.
    /// `None` while the enum is incomplete (forward-declared).
    pub underlying_type: Option<Type>,
    /// `true` once the enumerator body has been analysed.
    pub is_complete: bool,
}

/// One member of a struct or union layout.
#[derive(Clone, Debug)]
pub struct MemberLayout {
    /// Member name.  `None` for anonymous bit-fields (`int : 5;`) and
    /// for C11 anonymous struct/union members.
    pub name: Option<String>,
    /// Member type (possibly qualified).
    pub ty: QualType,
    /// Byte offset from the start of the enclosing struct/union.  For
    /// union members this is always 0.
    pub offset: u64,
    /// `Some` if this member is a bit-field.
    pub bit_field: Option<BitFieldLayout>,
    /// `Some` if this member is a C11 anonymous struct/union whose
    /// fields are transparently accessible on the outer type.
    pub anon_members: Option<AnonMemberMap>,
}

/// Flattened view of the members contributed by a C11 anonymous
/// struct/union member.
///
/// Stored alongside the anonymous member in [`MemberLayout::anon_members`]
/// so that ordinary member-access can jump straight to an inner field
/// without walking the nested layout at every use.
#[derive(Clone, Debug, Default)]
pub struct AnonMemberMap {
    /// name → (offset_from_outer_container, member_type)
    pub fields: HashMap<String, (u64, QualType)>,
}

/// Bit-field placement within a storage unit.
#[derive(Clone, Copy, Debug)]
pub struct BitFieldLayout {
    /// Number of bits occupied by the bit-field.
    pub width: u32,
    /// Bit offset within the containing storage unit (LSB-first on the
    /// System V AMD64 ABI we support).
    pub bit_offset: u32,
    /// Size of the storage unit that holds this bit-field, in bytes.
    pub storage_unit_size: u32,
}

/// How a `sizeof` expression should be lowered.
///
/// Populated by expression analysis (Phase 4.4) and consumed by Phase
/// 5 IR lowering.  Forward-declared here so the context struct that
/// stores it can reference the type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizeofKind {
    /// The result is a compile-time constant — lower directly to the
    /// given `u64`.
    Constant(u64),
    /// The operand referred to a VLA; its size is computed at runtime
    /// from the listed expression nodes (one per VLA dimension).
    RuntimeVla {
        /// Expression `NodeId`s whose evaluated products make up the
        /// final byte count at runtime.
        expr_nodes: Vec<forge_parser::NodeId>,
    },
}

impl TypeContext {
    /// Allocate a fresh, unused [`StructTypeId`].  The new id has no
    /// layout info registered yet — call [`Self::set_struct`] once the
    /// struct body is resolved.
    pub fn fresh_struct_id(&mut self) -> StructTypeId {
        let id = StructTypeId(self.next_struct_id);
        self.next_struct_id += 1;
        id
    }

    /// Allocate a fresh, unused [`UnionTypeId`].
    pub fn fresh_union_id(&mut self) -> UnionTypeId {
        let id = UnionTypeId(self.next_union_id);
        self.next_union_id += 1;
        id
    }

    /// Allocate a fresh, unused [`EnumTypeId`].
    pub fn fresh_enum_id(&mut self) -> EnumTypeId {
        let id = EnumTypeId(self.next_enum_id);
        self.next_enum_id += 1;
        id
    }

    /// Insert or replace a struct layout.
    pub fn set_struct(&mut self, id: StructTypeId, layout: StructLayout) {
        self.structs.insert(id.0, layout);
    }

    /// Insert or replace a union layout.
    pub fn set_union(&mut self, id: UnionTypeId, layout: UnionLayout) {
        self.unions.insert(id.0, layout);
    }

    /// Insert or replace an enum layout.
    pub fn set_enum(&mut self, id: EnumTypeId, layout: EnumLayout) {
        self.enums.insert(id.0, layout);
    }

    /// Borrow the full struct layout, if one is registered.
    pub fn struct_layout(&self, id: StructTypeId) -> Option<&StructLayout> {
        self.structs.get(&id.0)
    }

    /// Borrow the full union layout, if one is registered.
    pub fn union_layout(&self, id: UnionTypeId) -> Option<&UnionLayout> {
        self.unions.get(&id.0)
    }

    /// Borrow the full enum layout, if one is registered.
    pub fn enum_layout(&self, id: EnumTypeId) -> Option<&EnumLayout> {
        self.enums.get(&id.0)
    }

    /// Struct size in bytes, or `None` if incomplete / unknown.
    pub fn struct_size(&self, id: StructTypeId) -> Option<u64> {
        self.structs
            .get(&id.0)
            .filter(|s| s.is_complete)
            .map(|s| s.total_size)
    }

    /// Struct alignment in bytes, or `None` if incomplete / unknown.
    pub fn struct_align(&self, id: StructTypeId) -> Option<u64> {
        self.structs
            .get(&id.0)
            .filter(|s| s.is_complete)
            .map(|s| s.alignment)
    }

    /// Union size in bytes, or `None` if incomplete / unknown.
    pub fn union_size(&self, id: UnionTypeId) -> Option<u64> {
        self.unions
            .get(&id.0)
            .filter(|u| u.is_complete)
            .map(|u| u.total_size)
    }

    /// Union alignment in bytes, or `None` if incomplete / unknown.
    pub fn union_align(&self, id: UnionTypeId) -> Option<u64> {
        self.unions
            .get(&id.0)
            .filter(|u| u.is_complete)
            .map(|u| u.alignment)
    }

    /// Enum size in bytes, or `None` to fall back to `int`.
    pub fn enum_size(&self, id: EnumTypeId) -> Option<u64> {
        self.enums
            .get(&id.0)
            .and_then(|e| e.underlying_type.as_ref())
            .and_then(|ty| match ty {
                Type::Int { .. } => Some(4),
                Type::Long { .. } => Some(8),
                Type::LongLong { .. } => Some(8),
                Type::Short { .. } => Some(2),
                Type::Char { .. } => Some(1),
                _ => None,
            })
    }

    /// Enum alignment in bytes, or `None` to fall back to `int`.
    pub fn enum_align(&self, id: EnumTypeId) -> Option<u64> {
        self.enum_size(id)
    }

    /// The underlying integer type chosen for an enum, or `None` if the
    /// enum is incomplete.  Callers using this to promote an enum lvalue
    /// fall back to `int` when the enum is not yet complete.
    pub fn enum_underlying(&self, id: EnumTypeId) -> Option<&Type> {
        self.enums
            .get(&id.0)
            .and_then(|e| e.underlying_type.as_ref())
    }

    /// `true` if the struct with this ID has been laid out.
    pub fn is_struct_complete(&self, id: StructTypeId) -> bool {
        self.structs.get(&id.0).is_some_and(|s| s.is_complete)
    }

    /// `true` if the union with this ID has been laid out.
    pub fn is_union_complete(&self, id: UnionTypeId) -> bool {
        self.unions.get(&id.0).is_some_and(|u| u.is_complete)
    }

    /// `true` if the enum with this ID has enumerators resolved.
    pub fn is_enum_complete(&self, id: EnumTypeId) -> bool {
        self.enums.get(&id.0).is_some_and(|e| e.is_complete)
    }

    /// Looked-up tag name, or `None` for anonymous / unknown.
    pub fn struct_tag(&self, id: StructTypeId) -> Option<&str> {
        self.structs.get(&id.0).and_then(|s| s.tag.as_deref())
    }

    /// Looked-up tag name, or `None` for anonymous / unknown.
    pub fn union_tag(&self, id: UnionTypeId) -> Option<&str> {
        self.unions.get(&id.0).and_then(|u| u.tag.as_deref())
    }

    /// Looked-up tag name, or `None` for anonymous / unknown.
    pub fn enum_tag(&self, id: EnumTypeId) -> Option<&str> {
        self.enums.get(&id.0).and_then(|e| e.tag.as_deref())
    }
}

// =========================================================================
// ImplicitConversion
// =========================================================================

/// The kind of implicit conversion applied at an expression site.
///
/// Sema attaches one of these to the AST on every implicit conversion
/// it inserts — array-to-pointer decay, integer promotions, usual
/// arithmetic conversions, etc.  Later phases use the enum both for
/// codegen and for emitting lint diagnostics ("implicit narrowing",
/// "comparison of signed and unsigned").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImplicitConversion {
    /// Reading the value of an lvalue into an rvalue.
    LvalueToRvalue,
    /// `T[N]` → `T *` decay.
    ArrayToPointer,
    /// Function-to-pointer decay.
    FunctionToPointer,
    /// Small integer → `int` (C17 §6.3.1.1).
    IntegerPromotion {
        /// The promoted-to type.
        to: Type,
    },
    /// Arithmetic balancing via the usual arithmetic conversions
    /// (C17 §6.3.1.8).
    ArithmeticConversion {
        /// The common type.
        to: Type,
    },
    /// Integer → floating-point.
    IntToFloat {
        /// Destination floating type.
        to: Type,
    },
    /// Floating-point → integer.
    FloatToInt {
        /// Destination integer type.
        to: Type,
    },
    /// Floating type → wider / narrower floating type.
    FloatConversion {
        /// Destination floating type.
        to: Type,
    },
    /// Pointer → `_Bool` in a Boolean context.
    PointerToBoolean,
    /// Integer 0 (or `(void*)0`) → null pointer of some type.
    NullPointerConversion,
    /// Integer → pointer (explicit cast lowered to implicit — unusual).
    IntegerToPointer,
    /// Pointer → integer (same as above).
    PointerToInteger,
    /// Adding qualifiers (`T *` → `const T *`).
    QualificationConversion,
    /// Reading a bit-field as its promoted integer type.
    BitFieldToInt,
}

// =========================================================================
// Integer promotion and usual arithmetic conversions
// =========================================================================

/// Apply C17 §6.3.1.1 integer promotion to `ty`.
///
/// Non-integer types pass through unchanged so callers can invoke this
/// unconditionally on operand types.
pub fn integer_promotion(ty: &Type, target: &TargetInfo) -> Type {
    match ty {
        Type::Bool | Type::Char { .. } | Type::Short { is_unsigned: false } | Type::Enum(_) => {
            Type::Int { is_unsigned: false }
        }
        Type::Short { is_unsigned: true } => {
            // `unsigned short` fits in `int` iff int strictly larger.
            if target.int_size > target.short_size {
                Type::Int { is_unsigned: false }
            } else {
                Type::Int { is_unsigned: true }
            }
        }
        _ => ty.clone(),
    }
}

/// C17 §6.3.1.8 usual arithmetic conversions.
///
/// Determine the common type of two arithmetic operands.  Non-arithmetic
/// input is defensively returned unchanged on the left, matching what
/// most callers want for error recovery.
pub fn usual_arithmetic_conversions(lhs: &Type, rhs: &Type, target: &TargetInfo) -> Type {
    // Step 1 — long double / double / float hierarchy.
    if matches!(lhs, Type::LongDouble) || matches!(rhs, Type::LongDouble) {
        return Type::LongDouble;
    }
    if matches!(lhs, Type::Double) || matches!(rhs, Type::Double) {
        return Type::Double;
    }
    if matches!(lhs, Type::Float) || matches!(rhs, Type::Float) {
        return Type::Float;
    }

    // Step 2 — apply integer promotions to both operands.
    let lhs = integer_promotion(lhs, target);
    let rhs = integer_promotion(rhs, target);

    // Step 3 — same type after promotion.
    if lhs == rhs {
        return lhs;
    }

    // Any non-integer leftover: we can't pick a common type; return lhs.
    if !lhs.is_integer() || !rhs.is_integer() {
        return lhs;
    }

    let lhs_rank = lhs.integer_rank();
    let rhs_rank = rhs.integer_rank();
    let lhs_u = lhs.is_unsigned();
    let rhs_u = rhs.is_unsigned();

    // Step 4 — same signedness: higher rank wins.
    if lhs_u == rhs_u {
        return if lhs_rank >= rhs_rank { lhs } else { rhs };
    }

    // Different signedness — arrange (u = unsigned operand, s = signed operand).
    let (u, u_rank, s, s_rank) = if lhs_u {
        (&lhs, lhs_rank, &rhs, rhs_rank)
    } else {
        (&rhs, rhs_rank, &lhs, lhs_rank)
    };

    // Step 5 — unsigned rank ≥ signed rank: unsigned wins.
    if u_rank >= s_rank {
        return u.clone();
    }

    // Step 6 — signed can represent all unsigned values: signed wins.
    if signed_can_represent_unsigned(s, u, target) {
        return s.clone();
    }

    // Step 7 — fall back to the unsigned version of the signed type.
    to_unsigned(s)
}

/// `true` if the `signed_ty` operand can represent every value of the
/// `unsigned_ty` operand on `target`.  Used by rule 6 of the usual
/// arithmetic conversions.
fn signed_can_represent_unsigned(
    signed_ty: &Type,
    unsigned_ty: &Type,
    target: &TargetInfo,
) -> bool {
    let s_bits = integer_bit_width(signed_ty, target);
    let u_bits = integer_bit_width(unsigned_ty, target);
    // Signed type representable range is 2^(s_bits-1) − 1; unsigned is
    // 2^u_bits − 1.  Representation property reduces to s_bits − 1 ≥
    // u_bits, which is strict: signed must have MORE bits.
    s_bits > u_bits
}

fn integer_bit_width(ty: &Type, target: &TargetInfo) -> u64 {
    match ty {
        Type::Bool => target.bool_size * 8,
        Type::Char { .. } => target.char_size * 8,
        Type::Short { .. } => target.short_size * 8,
        Type::Int { .. } | Type::Enum(_) => target.int_size * 8,
        Type::Long { .. } => target.long_size * 8,
        Type::LongLong { .. } => target.long_long_size * 8,
        _ => 0,
    }
}

fn to_unsigned(ty: &Type) -> Type {
    match ty {
        Type::Int { .. } => Type::Int { is_unsigned: true },
        Type::Long { .. } => Type::Long { is_unsigned: true },
        Type::LongLong { .. } => Type::LongLong { is_unsigned: true },
        Type::Short { .. } => Type::Short { is_unsigned: true },
        other => other.clone(),
    }
}

// =========================================================================
// Compatibility and composite types
// =========================================================================

/// C17 §6.2.7 type compatibility including qualifiers.
///
/// Two qualified types are compatible if their unqualified types are
/// compatible *and* their top-level qualifier sets match.  For the
/// qualifier-agnostic variant use [`are_compatible_unqualified`].
pub fn are_compatible(a: &QualType, b: &QualType, ctx: &TypeContext) -> bool {
    if a.qualifier_mask() != b.qualifier_mask() {
        return false;
    }
    types_compatible(&a.ty, &b.ty, ctx)
}

/// Qualifier-ignoring variant of [`are_compatible`].
pub fn are_compatible_unqualified(a: &QualType, b: &QualType, ctx: &TypeContext) -> bool {
    types_compatible(&a.ty, &b.ty, ctx)
}

fn types_compatible(a: &Type, b: &Type, ctx: &TypeContext) -> bool {
    match (a, b) {
        (Type::Void, Type::Void) => true,

        // Enum ↔ its (default) underlying int type.  Must fire before
        // the arithmetic arm below — otherwise that arm shadows this
        // rule with strict structural equality and the two cases would
        // never match.
        (Type::Enum(_), Type::Int { is_unsigned: false })
        | (Type::Int { is_unsigned: false }, Type::Enum(_)) => true,

        // Arithmetic: structural equality is enough.
        (x, y) if x.is_arithmetic() && y.is_arithmetic() => x == y,

        (Type::Pointer { pointee: ap }, Type::Pointer { pointee: bp }) => {
            // Qualifier match on the pointee is required for full
            // compatibility; callers wanting laxer rules should strip
            // qualifiers themselves.
            are_compatible(ap, bp, ctx)
        }

        (
            Type::Array {
                element: ae,
                size: asz,
            },
            Type::Array {
                element: be,
                size: bsz,
            },
        ) => {
            if !are_compatible(ae, be, ctx) {
                return false;
            }
            match (asz, bsz) {
                (ArraySize::Fixed(n), ArraySize::Fixed(m)) => n == m,
                (ArraySize::Incomplete, _) | (_, ArraySize::Incomplete) => true,
                (ArraySize::Variable, _) | (_, ArraySize::Variable) => true,
                (ArraySize::Star, ArraySize::Star) => true,
                _ => false,
            }
        }

        (
            Type::Function {
                return_type: ar,
                params: ap,
                is_variadic: av,
                is_prototype: apro,
            },
            Type::Function {
                return_type: br,
                params: bp,
                is_variadic: bv,
                is_prototype: bpro,
            },
        ) => {
            if !are_compatible(ar, br, ctx) {
                return false;
            }
            match (*apro, *bpro) {
                (true, true) => {
                    av == bv
                        && ap.len() == bp.len()
                        && ap
                            .iter()
                            .zip(bp.iter())
                            .all(|(x, y)| are_compatible(&x.ty, &y.ty, ctx))
                }
                (false, false) => true,
                (true, false) | (false, true) => {
                    let (proto_params, proto_variadic) = if *apro { (ap, *av) } else { (bp, *bv) };
                    !proto_variadic && proto_params.iter().all(|p| is_default_promoted(&p.ty.ty))
                }
            }
        }

        (Type::Struct(a), Type::Struct(b)) => a == b,
        (Type::Union(a), Type::Union(b)) => a == b,
        (Type::Enum(a), Type::Enum(b)) => a == b,

        _ => false,
    }
}

/// A type matches what default argument promotions would produce — so
/// an unprototyped declaration may be compatible with a prototyped one
/// that uses this type.
fn is_default_promoted(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int { .. }
            | Type::Long { .. }
            | Type::LongLong { .. }
            | Type::Double
            | Type::LongDouble
            | Type::Pointer { .. }
    )
}

/// Composite of two compatible types, per C17 §6.2.7.
///
/// The caller is responsible for ensuring `are_compatible(a, b, ctx)`
/// holds (or can be proven loosely); if it doesn't, `a` is returned
/// unchanged so the caller can still produce a diagnostic based on it.
///
/// `ctx` is threaded through for parity with [`are_compatible`] and so
/// that future extensions (merging struct/union layouts, for example)
/// have it available without a signature change.
#[allow(clippy::only_used_in_recursion)]
pub fn composite_type(a: &QualType, b: &QualType, ctx: &TypeContext) -> QualType {
    match (&a.ty, &b.ty) {
        (
            Type::Array {
                element: ae,
                size: asz,
            },
            Type::Array {
                element: be,
                size: bsz,
            },
        ) => {
            let elem = composite_type(ae, be, ctx);
            let size = match (asz, bsz) {
                (ArraySize::Fixed(n), _) | (_, ArraySize::Fixed(n)) => ArraySize::Fixed(*n),
                (ArraySize::Variable, _) | (_, ArraySize::Variable) => ArraySize::Variable,
                _ => asz.clone(),
            };
            QualType {
                ty: Type::Array {
                    element: Box::new(elem),
                    size,
                },
                is_const: a.is_const,
                is_volatile: a.is_volatile,
                is_restrict: a.is_restrict,
                is_atomic: a.is_atomic,
                explicit_align: a.explicit_align.or(b.explicit_align),
            }
        }
        (
            Type::Function {
                is_prototype: true, ..
            },
            Type::Function {
                is_prototype: false,
                ..
            },
        ) => a.clone(),
        (
            Type::Function {
                is_prototype: false,
                ..
            },
            Type::Function {
                is_prototype: true, ..
            },
        ) => b.clone(),
        _ => a.clone(),
    }
}

// =========================================================================
// Display implementation
// =========================================================================

impl fmt::Display for QualType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write_qualtype(self, &mut out, None);
        f.write_str(&out)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Wrap in a QualType with no qualifiers so we reuse the same
        // printer.  This keeps the recursion in one place.
        let qt = QualType::unqualified(self.clone());
        let mut out = String::new();
        write_qualtype(&qt, &mut out, None);
        f.write_str(&out)
    }
}

fn write_qualtype(qt: &QualType, out: &mut String, ctx: Option<&TypeContext>) {
    match &qt.ty {
        // Pointer → write the pointee then " *" and any outer quals.
        Type::Pointer { pointee } => match &pointee.ty {
            Type::Function {
                return_type,
                params,
                is_variadic,
                ..
            } => {
                write_qualtype(return_type, out, ctx);
                out.push_str(" (*");
                write_outer_quals(qt, out);
                out.push_str(")(");
                write_params(params, *is_variadic, out, ctx);
                out.push(')');
            }
            Type::Array { element, size } => {
                write_qualtype(element, out, ctx);
                out.push_str(" (*");
                write_outer_quals(qt, out);
                out.push(')');
                write_array_size(size, out);
            }
            _ => {
                write_qualtype(pointee, out, ctx);
                out.push_str(" *");
                write_outer_quals(qt, out);
            }
        },

        Type::Array { element, size } => {
            write_qualtype(element, out, ctx);
            write_array_size(size, out);
        }

        Type::Function {
            return_type,
            params,
            is_variadic,
            ..
        } => {
            write_qualtype(return_type, out, ctx);
            out.push('(');
            write_params(params, *is_variadic, out, ctx);
            out.push(')');
        }

        _ => {
            write_qual_prefix(qt, out);
            write_base(&qt.ty, out, ctx);
        }
    }
}

fn write_qual_prefix(qt: &QualType, out: &mut String) {
    if qt.is_const {
        out.push_str("const ");
    }
    if qt.is_volatile {
        out.push_str("volatile ");
    }
    if qt.is_restrict {
        out.push_str("restrict ");
    }
    if qt.is_atomic {
        out.push_str("_Atomic ");
    }
}

fn write_outer_quals(qt: &QualType, out: &mut String) {
    let mut first = true;
    let emit = |s: &str, out: &mut String, first: &mut bool| {
        if *first {
            *first = false;
        } else {
            out.push(' ');
        }
        out.push_str(s);
    };
    if qt.is_const {
        emit("const", out, &mut first);
    }
    if qt.is_volatile {
        emit("volatile", out, &mut first);
    }
    if qt.is_restrict {
        emit("restrict", out, &mut first);
    }
    if qt.is_atomic {
        emit("_Atomic", out, &mut first);
    }
}

fn write_array_size(size: &ArraySize, out: &mut String) {
    out.push('[');
    match size {
        ArraySize::Fixed(n) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{n}");
        }
        ArraySize::Variable => out.push_str("<vla>"),
        ArraySize::Incomplete => {}
        ArraySize::Star => out.push('*'),
    }
    out.push(']');
}

fn write_params(
    params: &[ParamType],
    is_variadic: bool,
    out: &mut String,
    ctx: Option<&TypeContext>,
) {
    if params.is_empty() && !is_variadic {
        return;
    }
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_qualtype(&p.ty, out, ctx);
    }
    if is_variadic {
        if !params.is_empty() {
            out.push_str(", ");
        }
        out.push_str("...");
    }
}

fn write_base(ty: &Type, out: &mut String, ctx: Option<&TypeContext>) {
    match ty {
        Type::Void => out.push_str("void"),
        Type::Bool => out.push_str("_Bool"),
        Type::Char {
            signedness: Signedness::Plain,
        } => out.push_str("char"),
        Type::Char {
            signedness: Signedness::Signed,
        } => out.push_str("signed char"),
        Type::Char {
            signedness: Signedness::Unsigned,
        } => out.push_str("unsigned char"),
        Type::Short { is_unsigned: false } => out.push_str("short"),
        Type::Short { is_unsigned: true } => out.push_str("unsigned short"),
        Type::Int { is_unsigned: false } => out.push_str("int"),
        Type::Int { is_unsigned: true } => out.push_str("unsigned int"),
        Type::Long { is_unsigned: false } => out.push_str("long"),
        Type::Long { is_unsigned: true } => out.push_str("unsigned long"),
        Type::LongLong { is_unsigned: false } => out.push_str("long long"),
        Type::LongLong { is_unsigned: true } => out.push_str("unsigned long long"),
        Type::Float => out.push_str("float"),
        Type::Double => out.push_str("double"),
        Type::LongDouble => out.push_str("long double"),
        Type::Struct(id) => {
            out.push_str("struct ");
            write_tag(ctx.and_then(|c| c.struct_tag(*id)), id.0, out);
        }
        Type::Union(id) => {
            out.push_str("union ");
            write_tag(ctx.and_then(|c| c.union_tag(*id)), id.0, out);
        }
        Type::Enum(id) => {
            out.push_str("enum ");
            write_tag(ctx.and_then(|c| c.enum_tag(*id)), id.0, out);
        }
        // Covered by the wrapping path in `write_qualtype`.
        Type::Pointer { .. } | Type::Array { .. } | Type::Function { .. } => {}
    }
}

fn write_tag(tag: Option<&str>, id: u32, out: &mut String) {
    match tag {
        Some(name) => out.push_str(name),
        None => {
            use std::fmt::Write as _;
            let _ = write!(out, "#{id}");
        }
    }
}
