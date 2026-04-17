// RUN: forge check %s
//
// Every C17 punctuator must tokenise as its dedicated TokenKind variant.
// The CHECK lines anchor on the quoted source text (e.g. '+=') rather
// than the Debug-printed variant name, because the variant names have
// prefix collisions (`Plus` is a substring of `PlusPlus`/`PlusEqual`).
// The quoted form is unique per punctuator and sidesteps the problem.
//
// The bare punctuator sequence below is not a syntactically valid C
// translation unit — `forge check` now runs the parser after the
// lexer, so parser errors are expected.  Declaring `// ERROR:`
// directives lets the lit runner accept the non-zero exit while the
// CHECK directives above still validate the lexer's token output on
// stdout, which is what this file actually tests.
// ERROR: expected
//
// Brackets and groupings.
// CHECK: '('
// CHECK: ')'
// CHECK: '{'
// CHECK: '}'
// CHECK: '['
// CHECK: ']'
//
// Member access and the arrow operator.
// CHECK: '.'
// CHECK: '->'
//
// Increment and decrement.
// CHECK: '++'
// CHECK: '--'
//
// Single-character unary / binary operators.
// CHECK: '&'
// CHECK: '*'
// CHECK: '+'
// CHECK: '-'
// CHECK: '~'
// CHECK: '!'
// CHECK: '/'
// CHECK: '%'
//
// Shifts (longest-match: `<<` stays a single token, not two `<`s).
// CHECK: '<<'
// CHECK: '>>'
//
// Relational and equality operators.
// CHECK: '<'
// CHECK: '>'
// CHECK: '<='
// CHECK: '>='
// CHECK: '=='
// CHECK: '!='
//
// Bitwise and logical.
// CHECK: '^'
// CHECK: '|'
// CHECK: '&&'
// CHECK: '||'
//
// Ternary, statement punctuation, and ellipsis.
// CHECK: '?'
// CHECK: ':'
// CHECK: ';'
// CHECK: '...'
// CHECK: ','
//
// Simple and compound assignments.
// CHECK: '='
// CHECK: '*='
// CHECK: '/='
// CHECK: '%='
// CHECK: '+='
// CHECK: '-='
// CHECK: '<<='
// CHECK: '>>='
// CHECK: '&='
// CHECK: '^='
// CHECK: '|='
//
// (`#` and `##` are preprocessor-only tokens and are consumed by the
// preprocessor before the lexer output reaches this harness — their
// recognition is exercised from `tests/lit/preprocess/` instead.)

( ) { } [ ]
. ->
++ --
& * + - ~ ! / %
<< >>
< > <= >= == !=
^ | && ||
? : ; ... ,
= *= /= %= += -=
<<= >>= &= ^= |=
