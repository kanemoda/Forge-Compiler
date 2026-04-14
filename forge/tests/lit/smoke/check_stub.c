// RUN: forge check %s
//
// Verify that `forge check` accepts a syntactically valid C file and exits 0.
// No CHECK or ERROR directives are needed here — a clean exit is sufficient.
// This test will grow more interesting once the lexer and parser are wired in.

int x = 1;
