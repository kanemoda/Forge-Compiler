// RUN: forge -E %s
//
// Function-like macros must expand at every invocation with the
// arguments substituted into the replacement list.  Nested calls are
// fully rescanned so nested expansions appear.

#define SQR(x) ((x) * (x))
#define MAX(a, b) ((a) > (b) ? (a) : (b))

int s = SQR(7);
int m = MAX(3, 5);
int n = MAX(SQR(2), SQR(3));

// CHECK: int s = ((7) * (7));
// CHECK: int m = ((3) > (5) ? (3) : (5));
// CHECK: int n = ((((2) * (2))) > (((3) * (3))) ? (((2) * (2))) : (((3) * (3))));
