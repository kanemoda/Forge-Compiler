// RUN: forge -E %s
//
// The `##` paste operator must splice its left and right operands into
// a single token.  After pasting the result is rescanned, so pasted
// identifiers that are themselves macro names get expanded.

#define CAT(a, b) a##b
#define PREFIX_foo 1

int CAT(foo, bar) = 42;
int CAT(PREFIX_, foo) = 7;

// CHECK: int foobar = 42;
// CHECK: int 1 = 7;
