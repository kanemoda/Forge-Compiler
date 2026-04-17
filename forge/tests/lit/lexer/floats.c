// RUN: forge check %s
//
// Decimal and hexadecimal floating-point literals.  Every value below is
// exactly representable in f64, so the Debug-printed value in the token
// line is stable across platforms.
//
// Bare literals are not a valid C translation unit; `forge check`
// runs the parser after the lexer, so parser errors are expected.
// The `// ERROR:` directive lets the lit runner accept the non-zero
// exit while the CHECK directives above still validate the lexer's
// token output on stdout.
// ERROR: expected
//
// Decimal floats: ordinary "int.frac", leading-dot, trailing-dot, and
// exponent-only forms all take the float path.
// CHECK: FloatLiteral { value: 1.5, suffix: None }
// CHECK: FloatLiteral { value: 0.5, suffix: None }
// CHECK: FloatLiteral { value: 100000.0, suffix: None }
// CHECK: FloatLiteral { value: 2.0, suffix: None }
//
// `1.` with nothing after the dot is still a float (value 1.0).
// CHECK: FloatLiteral { value: 1.0, suffix: None }
//
// Hex floats: binary exponent is mandatory.
// 0x1.8p1 == (1 + 8/16) * 2^1 == 3.0
// 0x1p3   == 1 * 2^3           == 8.0
// 0x.8p2  == 0.5 * 2^2          == 2.0
// 0x1p-1  == 1 * 2^-1           == 0.5
// CHECK: FloatLiteral { value: 3.0, suffix: None }
// CHECK: FloatLiteral { value: 8.0, suffix: None }
// CHECK: FloatLiteral { value: 2.0, suffix: None }
// CHECK: FloatLiteral { value: 0.5, suffix: None }
//
// Float suffixes: `f`/`F` → FloatSuffix::F, `l`/`L` → FloatSuffix::L.
// CHECK: FloatLiteral { value: 1.5, suffix: F }
// CHECK: FloatLiteral { value: 1.5, suffix: L }
// CHECK: FloatLiteral { value: 3.0, suffix: F }
// CHECK: FloatLiteral { value: 8.0, suffix: L }

1.5
.5
1e5
2.0E0
1.
0x1.8p1
0x1p3
0x.8p2
0x1p-1
1.5f
1.5L
0x1.8p1F
0x1p3l
