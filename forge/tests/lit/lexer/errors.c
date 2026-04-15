// RUN: forge check %s
//
// Lexer error recovery: each source line below triggers a diagnostic.
// Files carrying at least one `// ERROR:` directive are expected-failure
// tests — the runner accepts a non-zero exit status and only requires
// the named substrings to appear somewhere in stderr.
//
// Unterminated string literal — a raw newline closes the line before
// the matching `"`.
// ERROR: unterminated string literal
//
// Unterminated character constant — same rule for `'`.
// ERROR: unterminated character constant
//
// Invalid octal digit — `9` is not a valid digit in base 8.
// ERROR: invalid digit in octal literal
//
// Hex integer literal with no digits after the `0x` prefix.
// ERROR: hex integer literal has no digits
//
// Hex float without the mandatory `p` / `P` binary exponent.
// ERROR: hex float missing binary exponent
//
// Empty character constant — `''` has zero characters between quotes.
// ERROR: empty character constant

"never closed
'x
099
0x
0x1.5
''
