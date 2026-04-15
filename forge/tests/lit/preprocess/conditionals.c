// RUN: forge -E %s
//
// `#if` / `#else` / `#endif` must compile-out the dead branch so it
// never appears in the output token stream.

#define DEBUG 1

#if DEBUG
int debug_only = 1;
#else
int release_only = 1;
#endif

#ifdef DEBUG
int debug_marker = 2;
#endif

#ifndef NOT_DEFINED
int ndef_marker = 3;
#endif

// CHECK: int debug_only = 1;
// CHECK: int debug_marker = 2;
// CHECK: int ndef_marker = 3;
