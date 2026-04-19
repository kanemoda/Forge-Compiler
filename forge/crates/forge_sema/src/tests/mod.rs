//! Test root for `forge_sema`.
//!
//! Tests are grouped by the area of the type system they cover:
//! sizes, qualifiers, promotions, arithmetic conversions, Display,
//! compatibility, and composite types.  Shared constructors live in
//! [`helpers`].

mod helpers;

mod arithmetic_conversions;
mod attribute_aligned;
mod attribute_noreturn;
mod attribute_packed;
mod attribute_unknown_ignored;
mod builtin_constant_p;
mod builtin_float_typedefs;
mod builtin_int128;
mod builtin_offsetof;
mod builtin_typeof;
mod builtin_types_compatible;
mod builtin_va;
mod compatibility;
mod composite;
mod const_eval;
mod declarations;
mod declarator_resolution;
mod display;
mod expr_address_deref;
mod expr_alignof;
mod expr_arithmetic;
mod expr_assignment;
mod expr_call;
mod expr_cast;
mod expr_comma;
mod expr_comparison;
mod expr_compound_literal;
mod expr_generic;
mod expr_identifier;
mod expr_increment;
mod expr_literals;
mod expr_lvalue;
mod expr_member;
mod expr_pointer_arith;
mod expr_shift_logical;
mod expr_sizeof;
mod expr_subscript;
mod expr_ternary;
mod function_body;
mod initializers;
mod integer_promotion;
mod layout;
mod parameter_scope;
mod qual_type;
mod specifier_resolution;
mod stmt_break_continue;
mod stmt_conditions;
mod stmt_goto;
mod stmt_return;
mod stmt_static_assert;
mod stmt_switch;
mod stress;
mod symbol_table;
mod translation_unit;
mod type_sizes;
