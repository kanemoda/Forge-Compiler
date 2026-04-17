// RUN: forge check %s
//
// Every C17 keyword must tokenise as its dedicated TokenKind variant,
// never as a plain Identifier.  The CHECK directives use `KIND span=` to
// avoid substring collisions — for example, a bare `Static` would also
// match the line emitted for `_Static_assert` (StaticAssert).
//
// The bare keyword list is not a valid C translation unit; `forge
// check` runs the parser after the lexer, so parser errors are
// expected.  The `// ERROR:` directive lets the lit runner accept the
// non-zero exit while the CHECK directives above still validate
// lexer-level output on stdout.
// ERROR: expected
//
// CHECK: Auto span=
// CHECK: Break span=
// CHECK: Case span=
// CHECK: Char span=
// CHECK: Const span=
// CHECK: Continue span=
// CHECK: Default span=
// CHECK: Do span=
// CHECK: Double span=
// CHECK: Else span=
// CHECK: Enum span=
// CHECK: Extern span=
// CHECK: Float span=
// CHECK: For span=
// CHECK: Goto span=
// CHECK: If span=
// CHECK: Inline span=
// CHECK: Int span=
// CHECK: Long span=
// CHECK: Register span=
// CHECK: Restrict span=
// CHECK: Return span=
// CHECK: Short span=
// CHECK: Signed span=
// CHECK: Sizeof span=
// CHECK: Static span=
// CHECK: Struct span=
// CHECK: Switch span=
// CHECK: Typedef span=
// CHECK: Union span=
// CHECK: Unsigned span=
// CHECK: Void span=
// CHECK: Volatile span=
// CHECK: While span=
// CHECK: Alignas span=
// CHECK: Alignof span=
// CHECK: Atomic span=
// CHECK: Bool span=
// CHECK: Complex span=
// CHECK: Generic span=
// CHECK: Imaginary span=
// CHECK: Noreturn span=
// CHECK: StaticAssert span=
// CHECK: ThreadLocal span=

auto break case char const continue default do double else enum extern
float for goto if inline int long register restrict return short signed
sizeof static struct switch typedef union unsigned void volatile while
_Alignas _Alignof _Atomic _Bool _Complex _Generic _Imaginary _Noreturn
_Static_assert _Thread_local
