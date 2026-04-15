// RUN: forge -E %s
//
// Object-like `#define` macros must be expanded at every reference and
// the `#define` directives themselves must not appear in the output.

#define N 42
#define MESSAGE "hello"
#define PI 314

int n = N;
const char *m = MESSAGE;
int p = PI;

// CHECK: int n = 42;
// CHECK: const char *m = "hello";
// CHECK: int p = 314;
