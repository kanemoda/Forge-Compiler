//! Synthetic lexer throughput benchmark.
//!
//! Builds a realistic ~50 000-line C translation unit in memory, then times
//! a single pass of the lexer over it.  Prints total time, lines/sec,
//! bytes/sec, and tokens/sec so we can compare against the <100 ms budget
//! stated in the Phase-1 validation plan.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p forge_lexer --release --example perf
//! ```

use std::time::Instant;

use forge_lexer::Lexer;

fn build_source(target_lines: usize) -> String {
    // A compact but heterogeneous template that exercises every major token
    // class: keywords, identifiers, numeric literals (decimal, hex, octal,
    // float), character literals, string literals, punctuators, line
    // comments, and block comments.
    let template = r#"// line comment ---------------------------------------------
/* block comment spanning a single line */
static int g_counter_%N% = 0;
const char *g_name_%N% = "forge_bench_%N%";

static inline long add_%N%(long a, long b) {
    /* do a little work so the body is not empty */
    long sum = a + b;
    sum ^= 0xDEADBEEFu;
    sum += 0755;        /* octal */
    sum *= 3L;
    sum /= 2LL;
    return sum;
}

static double compute_%N%(double x) {
    double y = x * 1.5e-3 + .25;
    y += 0x1.8p1;
    if (y > 1.0e10 || y < -1.0e10) {
        y = 0.0;
    }
    return y;
}

struct Point_%N% {
    int   x, y, z;
    float w;
    char  tag[16];
};

int process_%N%(struct Point_%N% *p, const char *msg) {
    if (p == NULL) return -1;
    for (int i = 0; i < 16; ++i) {
        p->tag[i] = (char)(i & 0xFF);
    }
    /* mixed punctuators: && || << >> >>= <<= == != <= >= ... */
    int flags = (p->x << 2) | (p->y >> 1);
    flags &= ~0x7;
    flags ^= 0xA5;
    return flags + add_%N%(p->x, p->y);
}
"#;

    // Each expansion of the template is about ~40 lines, so ~1 250 expansions
    // give us the targeted ~50 000-line file.
    let template_lines = template.lines().count();
    let copies = target_lines.div_ceil(template_lines);

    let mut out = String::with_capacity(template.len() * copies);
    for i in 0..copies {
        // %N% substitution without pulling in `regex`.
        let block = template.replace("%N%", &i.to_string());
        out.push_str(&block);
    }
    out
}

fn main() {
    const TARGET_LINES: usize = 50_000;

    let source = build_source(TARGET_LINES);
    let actual_lines = source.lines().count();
    let bytes = source.len();

    // One warm-up pass so we are not measuring first-allocation overhead.
    {
        let mut lex = Lexer::new(&source);
        let _ = lex.tokenize();
    }

    // Measured pass.
    let start = Instant::now();
    let mut lex = Lexer::new(&source);
    let tokens = lex.tokenize();
    let diags = lex.take_diagnostics();
    let elapsed = start.elapsed();

    let ms = elapsed.as_secs_f64() * 1_000.0;
    let lines_per_sec = actual_lines as f64 / elapsed.as_secs_f64();
    let mb_per_sec = (bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();
    let tokens_per_sec = tokens.len() as f64 / elapsed.as_secs_f64();

    println!("forge_lexer synthetic perf test");
    println!("  input:          {actual_lines} lines, {bytes} bytes");
    println!("  tokens:         {}", tokens.len());
    println!("  diagnostics:    {}", diags.len());
    println!("  elapsed:        {ms:.2} ms");
    println!("  throughput:     {lines_per_sec:>10.0} lines/sec");
    println!("                  {mb_per_sec:>10.2} MiB/sec");
    println!("                  {tokens_per_sec:>10.0} tokens/sec");

    if ms < 100.0 {
        println!("  verdict:        PASS (< 100 ms target)");
    } else if ms < 500.0 {
        println!("  verdict:        SLOW ({ms:.2} ms — above 100 ms budget, below 500 ms)");
    } else {
        println!("  verdict:        FAIL ({ms:.2} ms — above 500 ms, investigate)");
    }
}
