// RUN: forge check %s
//
// Extended system-header smoke test.  The baseline `headers_smoke.c`
// pulls in the four headers every C program needs; this one widens
// coverage to eight canonical C17 library headers and calls at least
// one function from each so sema must walk every prototype the host
// libc exposes through them.  A regression in declarator resolution,
// type compatibility, or the preprocessor surfaces here first.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>
#include <ctype.h>
#include <errno.h>
#include <time.h>

int main(int argc, char **argv) {
    (void)argc;
    (void)argv;

    // stdio.h — printf and fprintf with varargs.
    printf("extended smoke: %s\n", "ok");
    fprintf(stderr, "argc=%d\n", argc);

    // stdlib.h — conversion and allocation helpers.  free(0) is a
    // defined no-op and keeps the test hermetic.
    int parsed = atoi("42");
    free((void *)0);

    // string.h — length, copy, compare.
    const char *msg = "forge";
    size_t n = strlen(msg);
    char buf[16];
    memcpy(buf, msg, n);
    buf[n] = '\0';
    int eq = strcmp(buf, "forge");

    // stdint.h — fixed-width integer types must coexist with native
    // integer operations without compatibility errors.
    int32_t  i32 = 7;
    uint64_t u64 = (uint64_t)i32 * 3ull;

    // stddef.h — ptrdiff_t and size_t appear in pointer arithmetic.
    ptrdiff_t diff = &buf[n] - &buf[0];
    size_t    len  = (size_t)diff;

    // ctype.h — classification and case mapping.
    int is_alpha = isalpha((int)'A');
    int upper    = toupper((int)'a');

    // errno.h — errno is a modifiable lvalue reachable via its macro.
    errno = 0;
    int saved = errno;

    // time.h — time(NULL) returns a time_t; difftime returns a double.
    time_t now = time((time_t *)0);
    double elapsed = difftime(now, now);

    return (parsed + (int)n + eq + (int)i32 + (int)u64
          + (int)len + is_alpha + upper + saved
          + (int)now + (int)elapsed) & 0;
}
