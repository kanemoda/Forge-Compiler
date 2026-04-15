// RUN: forge -E %s
//
// End-to-end exercise combining several preprocessor features: object-like
// and function-like macros, nested invocations, `#if`/`#else`, and the
// `#ifdef` / `#undef` cycle.

#define LEVEL 2
#define SQR(x) ((x) * (x))
#define PICK(flag, a, b) ((flag) ? (a) : (b))

#if LEVEL > 1
int n = SQR(LEVEL);
#else
int n = 0;
#endif

#define TOGGLE 1
#ifdef TOGGLE
int m = PICK(TOGGLE, SQR(3), SQR(4));
#endif

#undef TOGGLE
#ifdef TOGGLE
int should_not_appear;
#else
int after_undef = 1;
#endif

// CHECK: int n = ((2) * (2));
// CHECK: int m = ((1) ? (((3) * (3))) : (((4) * (4))));
// CHECK: int after_undef = 1;
