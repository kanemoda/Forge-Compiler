// RUN: forge parse %s
//
// A small but non-trivial C17 translation unit that exercises many
// parser paths at once: file-scope typedefs, struct/union/enum,
// pointer + array declarators, a function declaration prototype, a
// function definition with parameters, every statement shape the
// parser knows (if/else, while, do-while, for, switch/case/default,
// break/continue, return, labelled goto, compound, expression), a
// local struct initialiser with designators, compound assignment,
// `sizeof`, and `_Generic`.  If any one of those paths breaks, this
// file stops passing.
//
// The AST-dump format is not a stable contract — these CHECK lines
// anchor on robust substrings (node kind + declarator name) rather
// than exact layout, so adding fields to AST nodes does not break
// the test.

// CHECK: TranslationUnit
// CHECK: Declaration
// CHECK: Specifiers [Typedef

typedef unsigned long size_t;

// CHECK: Specifiers [Struct(Point)]
// CHECK: Field
struct Point {
    int x;
    int y;
};

// CHECK: Specifiers [Enum(Color)]
enum Color {
    RED = 0,
    GREEN = 1,
    BLUE = 2,
};

// A union with a designated initialiser target below.
union Value {
    int   as_int;
    float as_float;
};

// Function prototype.
// CHECK: Declarator: square
int square(int n);

// Variadic function prototype — tests ellipsis in parameter list.
int sum(int count, ...);

// Function definition exercising the full statement dispatcher.
// CHECK: FunctionDef
// CHECK: Declarator: run
int run(int argc, char **argv) {
    size_t i = 0;
    int    result = 0;

    // Local struct + designated initialiser.
    struct Point origin = { .x = 0, .y = 0 };

    // Compound assignment operators.
    result += origin.x;
    result -= origin.y;
    result *= 2;

    // sizeof on both expression and type-name.
    size_t a = sizeof result;
    size_t b = sizeof(struct Point);
    result += (int)(a + b);

    // if / else-if / else chain.
    if (argc == 0) {
        result = -1;
    } else if (argc == 1) {
        result = 0;
    } else {
        result = argc;
    }

    // while loop with break.
    while (i < (size_t)argc) {
        if (argv[i] == 0) break;
        i += 1;
    }

    // do / while.
    do {
        result += 1;
    } while (result < 10);

    // for with declaration-init and continue.
    for (int k = 0; k < argc; k += 1) {
        if (k == 2) continue;
        result += k;
    }

    // switch / case / default / labelled-goto / fallthrough.
    switch (result & 0x3) {
        case 0:
            goto done;
        case 1:
        case 2:
            result += 1;
            break;
        default:
            result = 0;
            break;
    }

done:
    // _Generic type-selection.
    result += _Generic((int)0,
                      int:   1,
                      long:  2,
                      default: 0);

    return result;
}

// Top-level entry point.
// CHECK: Declarator: main
int main(int argc, char **argv) {
    return run(argc, argv);
}
