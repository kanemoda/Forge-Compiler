// RUN: forge check %s
//
// Character and string literals: every prefix and a representative set of
// escape sequences.  The Debug format of `StringLiteral { value: ... }`
// uses Rust's string-escaping so a source-level `\t` shows up as `\t` in
// the rendered value.
//
// Bare literals are not a valid C translation unit; `forge check`
// runs the parser after the lexer, so parser errors are expected.
// The `// ERROR:` directive lets the lit runner accept the non-zero
// exit while the CHECK directives above still validate the lexer's
// token output on stdout.
// ERROR: expected
//
// Unprefixed strings and escapes: \n \t \\ \" \0 and a hex escape.
// CHECK: StringLiteral { value: "hello", prefix: None }
// CHECK: StringLiteral { value: "a\tb", prefix: None }
// CHECK: StringLiteral { value: "line\nbreak", prefix: None }
// CHECK: StringLiteral { value: "quote\"inside", prefix: None }
// CHECK: StringLiteral { value: "null\0byte", prefix: None }
// CHECK: StringLiteral { value: "A = A", prefix: None }
//
// String prefixes.
// CHECK: StringLiteral { value: "wide", prefix: L }
// CHECK: StringLiteral { value: "utf8", prefix: Utf8 }
// CHECK: StringLiteral { value: "u16", prefix: U16 }
// CHECK: StringLiteral { value: "u32", prefix: U32 }
//
// Empty string is legal and has an empty value.
// CHECK: StringLiteral { value: "", prefix: None }
//
// Character literals: 'A' is ASCII 65, '\n' is 10, and the L/u/U
// prefixes produce CharPrefix::{L,U16,U32}.
// CHECK: CharLiteral { value: 65, prefix: None }
// CHECK: CharLiteral { value: 10, prefix: None }
// CHECK: CharLiteral { value: 0, prefix: None }
// CHECK: CharLiteral { value: 97, prefix: L }
// CHECK: CharLiteral { value: 98, prefix: U16 }
// CHECK: CharLiteral { value: 99, prefix: U32 }

"hello"
"a\tb"
"line\nbreak"
"quote\"inside"
"null\0byte"
"A = \x41"
L"wide"
u8"utf8"
u"u16"
U"u32"
""
'A'
'\n'
'\0'
L'a'
u'b'
U'c'
