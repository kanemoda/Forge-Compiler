// RUN: forge -E %s
//
// `#include "..."` resolves relative to the directory of the current
// file.  The included header's macros must be visible in the including
// file after the directive is consumed.

#include "fixtures/values.h"

int v = PI;
const char *g = GREETING;
int d = DOUBLE(5);

// CHECK: int v = 314;
// CHECK: const char *g = "hello, world";
// CHECK: int d = ((5) + (5));
