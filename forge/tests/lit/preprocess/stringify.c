// RUN: forge -E %s
//
// The `#` stringify operator must turn its argument's token sequence
// into a single string literal.  Adjacent whitespace collapses to a
// single space inside the resulting literal.

#define STR(x) #x

const char *greet = STR(hello world);
const char *expr  = STR(1 + 2);

// CHECK: const char *greet = "hello world";
// CHECK: const char *expr = "1 + 2";
