// RUN: forge check %s
//
// The Phase 4 system-header smoke test: if this file does not pass sema
// with zero errors, Phase 4 is not finished.  It pulls in the system
// headers that nearly every real-world C program begins with, declares
// a plausible `main` that exercises a representative set of standard
// library surfaces, and relies on the combination of our preprocessor,
// parser, and sema to walk the whole lot without complaint.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    // stdio — a handful of calls so the FILE*, printf, and fprintf
    // prototypes all get exercised.
    printf("hello, forge\n");
    fprintf(stderr, "%d arguments\n", argc);

    // string.h — memcpy / strlen / strcmp across a small buffer.
    char buf[32];
    const char *msg = "abc";
    size_t n = strlen(msg);
    memcpy(buf, msg, n);
    buf[n] = '\0';
    if (strcmp(buf, "abc") != 0) {
        return 1;
    }

    // stdint.h fixed-width types should declare cleanly and interop
    // with ordinary integer operations.
    int32_t i32 = 42;
    uint64_t u64 = 0;
    u64 = (uint64_t)i32;

    // stdlib.h — exit() is noreturn; make sure the noreturn attribute
    // doesn't derail sema.
    return 0;
}
