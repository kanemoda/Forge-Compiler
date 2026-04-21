//! Translation-unit analysis.
//!
//! This module owns the outermost sema entry point: [`analyze_translation_unit`]
//! builds a fresh [`SemaContext`] and [`SymbolTable`], pre-seeds the
//! handful of builtin typedefs that the rest of the crate relies on,
//! walks every [`ExternalDeclaration`], and finally promotes every
//! file-scope tentative definition to a real definition per C17
//! §6.9.2.  The resulting context — diagnostics, resolved types, and
//! symbol table by way of the returned handle — is what downstream
//! phases consume.

use forge_diagnostics::FileId;
use forge_lexer::Span;
use forge_parser::ast::{ExternalDeclaration, TranslationUnit};

use crate::context::SemaContext;
use crate::declare::{analyze_declaration, analyze_static_assert};
use crate::scope::{Linkage, StorageClass, Symbol, SymbolKind, SymbolTable};
use crate::stmt::analyze_function_def;
use crate::types::{ParamType, QualType, Signedness, TargetInfo, Type};

// =========================================================================
// Public entry point
// =========================================================================

/// Analyse a complete translation unit.
///
/// Returns `(SemaContext, SymbolTable)` so callers (the driver, tests)
/// retain access to both the type / conversion side tables and the
/// final symbol table.  Diagnostics live on the returned [`SemaContext`]
/// in source order.
pub fn analyze_translation_unit(
    tu: &TranslationUnit,
    target: &TargetInfo,
) -> (SemaContext, SymbolTable) {
    let mut ctx = SemaContext::new();
    let mut table = SymbolTable::new();

    seed_builtin_typedefs(&mut table, &mut ctx);

    for ext in &tu.declarations {
        match ext {
            ExternalDeclaration::FunctionDef(fd) => {
                analyze_function_def(fd, &mut table, target, &mut ctx);
            }
            ExternalDeclaration::Declaration(d) => {
                analyze_declaration(d, &mut table, target, &mut ctx);
            }
            ExternalDeclaration::StaticAssert(sa) => {
                analyze_static_assert(sa, &mut table, target, &mut ctx);
            }
        }
    }

    promote_tentative_definitions(&mut table);

    (ctx, table)
}

// =========================================================================
// Builtin typedef seeding
// =========================================================================

/// Pre-seed the builtin typedefs and builtin function declarations that
/// sema relies on regardless of which system headers were included.
///
/// The goal is simply to make system-header uses of these names
/// type-check cleanly — accuracy of the seeded types need only be good
/// enough for type-compatibility in sema.  The x86-64 codegen backend
/// is expected to replace `__builtin_va_list`, `__builtin_va_start`,
/// and similar names with the real ABI-specific lowering later on.
///
/// Four buckets live here:
///
/// 1. Variadic helper typedefs: `__builtin_va_list`.
/// 2. C11 Unicode character typedefs: `char16_t`, `char32_t`.
/// 3. GCC / ISO TS 18661-3 extended float typedefs: `_Float16`,
///    `_Float32`, `_Float64`, `_Float128`, and their `x`-suffixed kin.
/// 4. Builtin function declarations: `__builtin_va_start`,
///    `__builtin_va_end`, `__builtin_va_copy`, plus the non-varargs
///    builtins `__builtin_trap`, `__builtin_unreachable`,
///    `__builtin_expect`, `__builtin_constant_p`.
fn seed_builtin_typedefs(table: &mut SymbolTable, ctx: &mut SemaContext) {
    seed_builtin_typedef_names(table, ctx);
    seed_builtin_functions(table, ctx);
}

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

fn typedef_symbol(name: &str, ty: QualType) -> Symbol {
    Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Typedef,
        storage: StorageClass::None,
        linkage: Linkage::None,
        span: Span::new(FileId::INVALID, 0, 0),
        is_defined: true,
        is_inline: false,
        is_noreturn: false,
        has_noreturn_attr: false,
    }
}

fn function_symbol(name: &str, ty: QualType, is_noreturn: bool) -> Symbol {
    Symbol {
        id: 0,
        name: name.to_string(),
        ty,
        kind: SymbolKind::Function,
        storage: StorageClass::Extern,
        linkage: Linkage::External,
        span: Span::new(FileId::INVALID, 0, 0),
        is_defined: false,
        is_inline: false,
        is_noreturn,
        has_noreturn_attr: is_noreturn,
    }
}

fn plain_char_ptr() -> QualType {
    QualType::unqualified(Type::Pointer {
        pointee: Box::new(QualType::unqualified(Type::Char {
            signedness: Signedness::Plain,
        })),
    })
}

fn unqualified_int() -> QualType {
    QualType::unqualified(Type::Int { is_unsigned: false })
}

fn unqualified_long() -> QualType {
    QualType::unqualified(Type::Long { is_unsigned: false })
}

fn unqualified_void() -> QualType {
    QualType::unqualified(Type::Void)
}

fn void_ptr_to(pointee: QualType) -> QualType {
    QualType::unqualified(Type::Pointer {
        pointee: Box::new(pointee),
    })
}

fn nameless_param(ty: QualType) -> ParamType {
    ParamType {
        name: None,
        ty,
        has_static_size: false,
    }
}

fn function_type(return_type: QualType, params: Vec<ParamType>, is_variadic: bool) -> QualType {
    QualType::unqualified(Type::Function {
        return_type: Box::new(return_type),
        params,
        is_variadic,
        is_prototype: true,
    })
}

// -------------------------------------------------------------------------
// Typedef names
// -------------------------------------------------------------------------

fn seed_builtin_typedef_names(table: &mut SymbolTable, ctx: &mut SemaContext) {
    // `__builtin_va_list` maps to `char *` on every target we currently
    // support — real x86-64 SysV uses a struct, but the lightweight
    // typedef is enough for sema to accept uses of the name.
    let _ = table.declare(typedef_symbol("__builtin_va_list", plain_char_ptr()), ctx);

    // C11 Unicode character typedefs.  Standard headers (<uchar.h>,
    // <wchar.h>) usually define these, but we seed them as builtins so
    // code that predeclares them or uses the names directly is still
    // accepted.
    let _ = table.declare(
        typedef_symbol(
            "char16_t",
            QualType::unqualified(Type::Short { is_unsigned: true }),
        ),
        ctx,
    );
    let _ = table.declare(
        typedef_symbol(
            "char32_t",
            QualType::unqualified(Type::Int { is_unsigned: true }),
        ),
        ctx,
    );

    // ISO/IEC TS 18661-3 extended floating-point types.  These appear
    // in glibc's <math.h> on some targets.  We approximate each with a
    // C type of matching size:
    //
    // * `_Float16` → `float` (approximate — 2-byte vs 4-byte, but sema
    //   only needs a scalar)
    // * `_Float32` → `float`
    // * `_Float64` → `double`
    // * `_Float128` → `long double`
    // * `_FloatNx` (32x / 64x / 128x) → same as their base sibling
    for (name, ty) in [
        ("_Float16", QualType::unqualified(Type::Float)),
        ("_Float32", QualType::unqualified(Type::Float)),
        ("_Float32x", QualType::unqualified(Type::Double)),
        ("_Float64", QualType::unqualified(Type::Double)),
        ("_Float64x", QualType::unqualified(Type::LongDouble)),
        ("_Float128", QualType::unqualified(Type::LongDouble)),
        ("_Float128x", QualType::unqualified(Type::LongDouble)),
    ] {
        let _ = table.declare(typedef_symbol(name, ty), ctx);
    }

    // GCC __int128 / __int128_t / __uint128_t.  We don't have a real
    // 128-bit integer type, so we approximate with `long long`.  This
    // is wrong for size/alignment and will need a dedicated variant
    // before codegen touches these, but it's enough to get sema to
    // accept declarations that use the name.
    let _ = table.declare(
        typedef_symbol(
            "__int128",
            QualType::unqualified(Type::LongLong { is_unsigned: false }),
        ),
        ctx,
    );
    let _ = table.declare(
        typedef_symbol(
            "__int128_t",
            QualType::unqualified(Type::LongLong { is_unsigned: false }),
        ),
        ctx,
    );
    let _ = table.declare(
        typedef_symbol(
            "__uint128_t",
            QualType::unqualified(Type::LongLong { is_unsigned: true }),
        ),
        ctx,
    );
}

// -------------------------------------------------------------------------
// Builtin functions
// -------------------------------------------------------------------------

fn seed_builtin_functions(table: &mut SymbolTable, ctx: &mut SemaContext) {
    // `va_list` here is modelled as `char *` (see
    // `seed_builtin_typedef_names`).  Real implementations of the
    // varargs builtins take the va_list argument by reference-like
    // semantics, but sema only cares about surface type-compatibility.

    // `void __builtin_va_start(va_list ap, ...);`
    // The second argument is a plain identifier (the last named
    // parameter) and is varargs-punned here.
    let va_start_ty = function_type(
        unqualified_void(),
        vec![nameless_param(plain_char_ptr())],
        /* is_variadic = */ true,
    );
    let _ = table.declare(
        function_symbol("__builtin_va_start", va_start_ty, false),
        ctx,
    );

    // `void __builtin_va_end(va_list ap);`
    let va_end_ty = function_type(
        unqualified_void(),
        vec![nameless_param(plain_char_ptr())],
        false,
    );
    let _ = table.declare(function_symbol("__builtin_va_end", va_end_ty, false), ctx);

    // `void __builtin_va_copy(va_list dst, va_list src);`
    let va_copy_ty = function_type(
        unqualified_void(),
        vec![
            nameless_param(plain_char_ptr()),
            nameless_param(plain_char_ptr()),
        ],
        false,
    );
    let _ = table.declare(function_symbol("__builtin_va_copy", va_copy_ty, false), ctx);

    // `void __builtin_trap(void) __attribute__((noreturn));`
    let trap_ty = function_type(unqualified_void(), Vec::new(), false);
    let _ = table.declare(function_symbol("__builtin_trap", trap_ty, true), ctx);

    // `void __builtin_unreachable(void) __attribute__((noreturn));`
    let unreachable_ty = function_type(unqualified_void(), Vec::new(), false);
    let _ = table.declare(
        function_symbol("__builtin_unreachable", unreachable_ty, true),
        ctx,
    );

    // `long __builtin_expect(long, long);`
    let expect_ty = function_type(
        unqualified_long(),
        vec![
            nameless_param(unqualified_long()),
            nameless_param(unqualified_long()),
        ],
        false,
    );
    let _ = table.declare(function_symbol("__builtin_expect", expect_ty, false), ctx);

    // `int __builtin_constant_p(...);` — really a compile-time test
    // that takes any single expression; modelling it as variadic
    // returning `int` is the simplest way to let arbitrary argument
    // types pass without triggering a prototype mismatch.
    let const_p_ty = function_type(unqualified_int(), Vec::new(), true);
    let _ = table.declare(
        function_symbol("__builtin_constant_p", const_p_ty, false),
        ctx,
    );

    // `__builtin_object_size(void *, int)` — returns a `size_t`.  We
    // approximate with `unsigned long` on LP64.
    let object_size_ty = function_type(
        QualType::unqualified(Type::Long { is_unsigned: true }),
        vec![
            nameless_param(void_ptr_to(unqualified_void())),
            nameless_param(unqualified_int()),
        ],
        false,
    );
    let _ = table.declare(
        function_symbol("__builtin_object_size", object_size_ty, false),
        ctx,
    );

    // glibc's <byteswap.h> and <endian.h> pepper headers with these;
    // model them as byte-swaps on the matching fixed-width type.
    let u16_ty = QualType::unqualified(Type::Short { is_unsigned: true });
    let u32_ty = QualType::unqualified(Type::Int { is_unsigned: true });
    let u64_ty = QualType::unqualified(Type::Long { is_unsigned: true });
    let bswap16_ty = function_type(u16_ty.clone(), vec![nameless_param(u16_ty)], false);
    let _ = table.declare(function_symbol("__builtin_bswap16", bswap16_ty, false), ctx);
    let bswap32_ty = function_type(u32_ty.clone(), vec![nameless_param(u32_ty)], false);
    let _ = table.declare(function_symbol("__builtin_bswap32", bswap32_ty, false), ctx);
    let bswap64_ty = function_type(u64_ty.clone(), vec![nameless_param(u64_ty)], false);
    let _ = table.declare(function_symbol("__builtin_bswap64", bswap64_ty, false), ctx);

    // `void __builtin_abort(void) __attribute__((noreturn));`
    let abort_ty = function_type(unqualified_void(), Vec::new(), false);
    let _ = table.declare(function_symbol("__builtin_abort", abort_ty, true), ctx);

    // `__builtin_offsetof` and `__builtin_types_compatible_p` are not
    // seeded here: the parser lowers them to dedicated AST nodes
    // (`Expr::BuiltinOffsetof` / `Expr::BuiltinTypesCompatibleP`) and
    // sema evaluates each directly in `expr.rs` / `const_eval.rs`, so
    // they never go through ordinary name lookup.
}

// =========================================================================
// Tentative-definition promotion (§6.9.2)
// =========================================================================

/// Promote every surviving tentative definition to a real definition at
/// end of translation unit (C17 §6.9.2).
///
/// A file-scope object declaration without an initialiser and without
/// `extern` storage is *tentative*; if by end-of-TU no non-tentative
/// definition has overridden it, the object is implicitly defined with
/// a zero initialiser.  We surface that by flipping `is_defined` to
/// `true` so downstream phases can treat the declaration uniformly.
fn promote_tentative_definitions(table: &mut SymbolTable) {
    let promote: Vec<_> = table
        .all_symbols()
        .iter()
        .filter(|s| is_tentative_candidate(s))
        .map(|s| s.id)
        .collect();
    for id in promote {
        table.mark_defined(id);
    }
}

fn is_tentative_candidate(sym: &Symbol) -> bool {
    // Only undefined *variables* with external / internal linkage are
    // candidates.  Functions are never tentative, `extern` variables
    // with no initialiser remain declarations only, and block-scope
    // auto objects are marked defined at declaration time already.
    matches!(sym.kind, SymbolKind::Variable)
        && !sym.is_defined
        && !matches!(sym.storage, StorageClass::Extern)
        && matches!(sym.linkage, Linkage::External | Linkage::Internal)
}
