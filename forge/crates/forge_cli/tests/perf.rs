//! Wall-clock performance gates for the Phase 4 sema pipeline.
//!
//! Two tests:
//!
//! * **Test A — trivial:** `int main(void) { return 0; }` must
//!   complete a full lex → preprocess → parse → sema pass in well
//!   under the phase's budget.  This is the smoke gate; a regression
//!   here signals a structural problem.
//! * **Test B — medium:** `tests/lit/sema/headers_smoke_extended.c`
//!   plus the `realworld.c` body appended in, pulling in eight system
//!   headers and exercising the wide feature cross-section the real-
//!   world lit test covers.  This is the real budget check.
//!
//! Budgets are chosen from the Phase 4 acceptance criteria:
//!
//! | test  | release  | debug    |
//! |-------|----------|----------|
//! | A     |   80 ms  |  300 ms  |
//! | B     |  120 ms  |  500 ms  |
//!
//! The right budget is picked at compile time via
//! `cfg!(debug_assertions)` so a plain `cargo test` and
//! `cargo test --release` each enforce their matching target.  A
//! generous 3× safety margin is *not* applied — if this starts
//! flaking on slow CI, raise the budgets intentionally rather than
//! masking the regression.
//!
//! These tests skip themselves when the host has no discoverable
//! toolchain (same mechanism as `system_headers.rs`) because the
//! medium test needs `#include <stdio.h>` etc. to resolve.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

const FORGE_BIN: &str = env!("CARGO_BIN_EXE_forge");

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "forge_perf_{}_{}_{}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir { path }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.path.join(name);
        fs::write(&p, contents).expect("write temp file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn host_has_system_headers() -> bool {
    Command::new("cc")
        .args(["-E", "-v", "-x", "c", "/dev/null"])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Millisecond budget for a given test, chosen at compile time from
/// the current cargo profile.
const fn budget_ms(release: u64, debug: u64) -> u64 {
    if cfg!(debug_assertions) {
        debug
    } else {
        release
    }
}

fn run_forge_check(source: &std::path::Path) -> (std::time::Duration, bool, String, String) {
    let start = Instant::now();
    let out = Command::new(FORGE_BIN)
        .arg("check")
        .arg(source)
        .output()
        .expect("spawn forge check");
    let elapsed = start.elapsed();
    let ok = out.status.success();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    (elapsed, ok, stdout, stderr)
}

/// Trivial program — the structural smoke gate.
///
/// Budget: 80 ms release / 300 ms debug.
#[test]
fn perf_a_trivial_program_under_budget() {
    let tmp = TempDir::new("a_trivial");
    let src = tmp.file("main.c", "int main(void) { return 0; }\n");

    let (elapsed, ok, stdout, stderr) = run_forge_check(&src);
    assert!(
        ok,
        "forge check failed\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
        stdout.chars().take(2_000).collect::<String>()
    );

    let budget = budget_ms(80, 300);
    let actual_ms = elapsed.as_millis() as u64;
    assert!(
        actual_ms <= budget,
        "perf A: trivial program took {actual_ms} ms, budget {budget} ms \
         (cfg!(debug_assertions) = {})",
        cfg!(debug_assertions)
    );
    eprintln!("perf A trivial: {actual_ms} ms (budget {budget} ms)");
}

/// Medium program — eight system headers plus a substantial real body.
///
/// Uses the existing `headers_smoke_extended.c` content inlined into a
/// temp file so we are not coupled to any single lit path's identity,
/// and so the medium test is self-contained.
///
/// Budget: 120 ms release / 500 ms debug.
#[test]
fn perf_b_medium_program_under_budget() {
    if !host_has_system_headers() {
        eprintln!("skipping: no toolchain detected on host");
        return;
    }

    let tmp = TempDir::new("b_medium");
    let src = tmp.file(
        "main.c",
        r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stddef.h>
#include <ctype.h>
#include <errno.h>
#include <time.h>

enum Color { RED = 1, GREEN = 2, BLUE = 4, ALL = 7 };
struct Node { int value; struct Node *next; };
typedef int (*compare_fn)(int, int);
struct Comparator { const char *name; compare_fn fn; };

static int cmp_less(int a, int b)    { return a < b; }
static int cmp_greater(int a, int b) { return a > b; }
static int cmp_equal(int a, int b)   { return a == b; }

static const struct Comparator TABLE[] = {
    { .name = "less",    .fn = cmp_less },
    { .name = "greater", .fn = cmp_greater },
    { .name = "equal",   .fn = cmp_equal },
};

_Static_assert(sizeof(int) >= 2, "int >= 16 bits");

int sum_variadic(int count, ...);

int main(int argc, char **argv) {
    (void)argv;
    printf("bench: %s\n", "ok");
    fprintf(stderr, "argc=%d\n", argc);
    free((void *)0);

    const char *msg = "forge";
    size_t n = strlen(msg);
    char buf[32];
    memcpy(buf, msg, n);
    buf[n] = '\0';
    int eq = strcmp(buf, "forge");

    int32_t i32 = 7;
    uint64_t u64 = (uint64_t)i32 * 3ull;
    ptrdiff_t diff = &buf[n] - &buf[0];
    size_t len = (size_t)diff;
    int is_alpha = isalpha((int)'A');
    int upper = toupper((int)'a');
    errno = 0;
    int saved = errno;
    time_t now = time((time_t *)0);
    double elapsed = difftime(now, now);

    int xs[8] = { 0, 1, 2, 3, 4, 5, 6, 7 };
    int *p = xs;
    int *q = p + 3;
    int total = *q + *(p + 5);

    struct Node n1 = { .value = 1, .next = 0 };
    struct Node n2 = { .value = 2, .next = &n1 };
    struct Node *head = &n2;
    int walk = 0;
    while (head != 0) { walk += head->value; head = head->next; }

    int k = TABLE[0].fn(3, 5) + TABLE[2].fn(7, 7);

    enum Color col = GREEN;
    int classified = -1;
    switch (col) {
        case RED:   classified = 10; break;
        case GREEN: classified = 20; break;
        case BLUE:  classified = 30; break;
        case ALL:   classified = 40; break;
        case 99:    classified = 50; break;
        default:    classified =  0; break;
    }

    int i, j;
    int acc = 0;
    for (i = 0, j = 7; i < 8 && j >= 0; i++, j--) {
        acc += xs[i] - xs[j];
    }

    int v_sum = sum_variadic(3, 10, 20, 30);
    _Static_assert(sizeof(long) >= sizeof(int), "long >= int");

    return (argc + (int)n + eq + (int)i32 + (int)u64 + (int)len
          + is_alpha + upper + saved + (int)now + (int)elapsed
          + total + walk + k + classified + acc + v_sum) & 0;
}
"#,
    );

    let (elapsed, ok, stdout, stderr) = run_forge_check(&src);
    assert!(
        ok,
        "forge check failed\n--- stdout ---\n{}\n--- stderr ---\n{stderr}",
        stdout.chars().take(2_000).collect::<String>()
    );

    let budget = budget_ms(120, 500);
    let actual_ms = elapsed.as_millis() as u64;
    assert!(
        actual_ms <= budget,
        "perf B: medium program took {actual_ms} ms, budget {budget} ms \
         (cfg!(debug_assertions) = {})",
        cfg!(debug_assertions)
    );
    eprintln!("perf B medium: {actual_ms} ms (budget {budget} ms)");
}
