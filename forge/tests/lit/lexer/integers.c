// RUN: forge check %s
//
// Integer literals in every base Forge recognises, with and without
// suffixes.  The literal's parsed value and canonical suffix show up in
// the Debug-formatted IntegerLiteral variant printed by `forge check`.
//
// Bare literals are not a valid C translation unit; `forge check`
// runs the parser after the lexer, so parser errors are expected.
// The `// ERROR:` directive lets the lit runner accept the non-zero
// exit while the CHECK directives above still validate the lexer's
// token output on stdout.
// ERROR: expected
//
// Decimal zero and a small decimal integer.
// CHECK: IntegerLiteral { value: 0, suffix: None }
// CHECK: IntegerLiteral { value: 42, suffix: None }
//
// Octal: 010 == 8, 0755 == 493 (classic chmod bitmask).
// CHECK: IntegerLiteral { value: 8, suffix: None }
// CHECK: IntegerLiteral { value: 493, suffix: None }
//
// Hexadecimal in both cases: 0x1F == 31, 0XCAFEBABE == 3405691582.
// CHECK: IntegerLiteral { value: 31, suffix: None }
// CHECK: IntegerLiteral { value: 3405691582, suffix: None }
//
// u64::MAX — boundary of what we can represent without overflow.
// CHECK: IntegerLiteral { value: 18446744073709551615, suffix: None }
//
// Suffix combinations — each canonicalises to one of None/U/L/UL/LL/ULL.
// CHECK: IntegerLiteral { value: 1, suffix: U }
// CHECK: IntegerLiteral { value: 1, suffix: L }
// CHECK: IntegerLiteral { value: 1, suffix: UL }
// CHECK: IntegerLiteral { value: 1, suffix: LL }
// CHECK: IntegerLiteral { value: 1, suffix: ULL }

0
42
010
0755
0x1F
0XCAFEBABE
18446744073709551615
1u
1L
1UL
1ll
1LLU
