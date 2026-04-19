// RUN: forge check %s
//
// Real-world-ish Phase 4 acceptance sample.  Exercises a deliberately
// wide cross-section of C17 constructs that appear together in any
// non-trivial program:
//   * Multiple structs, one self-referential (a linked list node).
//   * An enum with explicit integer values.
//   * A function pointer typedef used as a struct member.
//   * An array of structs built with designated initializers.
//   * Pointer arithmetic on an array decay.
//   * Casts between compatible pointer types (void *, char *).
//   * sizeof and _Alignof used as the dimension of an array type.
//   * A switch statement with five cases plus default.
//   * A for-loop whose update step is a comma expression.
//   * A variadic function declaration AND a matching call.
//   * _Static_assert at file scope AND inside a block.
//
// Everything below must type-check with zero errors.

typedef unsigned long size_t;

// ------------------------- enum with explicit values
enum Color {
    COLOR_RED   = 1,
    COLOR_GREEN = 2,
    COLOR_BLUE  = 4,
    COLOR_ALL   = 7,
};

// ------------------------- self-referential struct (linked list)
struct Node {
    int           value;
    struct Node  *next;
};

// ------------------------- function pointer typedef in a struct
typedef int (*compare_fn)(int a, int b);

struct Comparator {
    const char *name;
    compare_fn  fn;
};

static int cmp_less(int a, int b)    { return a < b ? 1 : 0; }
static int cmp_greater(int a, int b) { return a > b ? 1 : 0; }
static int cmp_equal(int a, int b)   { return a == b ? 1 : 0; }

// ------------------------- array of structs with designated initializers
static const struct Comparator COMPARATORS[] = {
    { .name = "less",    .fn = cmp_less    },
    { .name = "greater", .fn = cmp_greater },
    { .name = "equal",   .fn = cmp_equal   },
};

// ------------------------- _Static_assert at FILE scope
_Static_assert(sizeof(int) >= 2, "C17 requires int to be at least 16 bits");
_Static_assert(COLOR_ALL == (COLOR_RED | COLOR_GREEN | COLOR_BLUE),
               "bitmask invariant broken");

// ------------------------- variadic function declaration
int sum_variadic(int count, ...);

// ------------------------- array sized by sizeof / _Alignof
static char scratch_by_size [sizeof(struct Node) * 4];
static char scratch_by_align[_Alignof(struct Node) * 8];

int main(int argc, char **argv) {
    (void)argv;

    // Pointer arithmetic on array decay.
    int xs[8] = { 0, 1, 2, 3, 4, 5, 6, 7 };
    int *p    = xs;
    int *q    = p + 3;
    int total = *q + *(p + 5);

    // void* / char* casts — compatible pointer conversions.
    void *v = scratch_by_size;
    char *c = (char *)v;
    c[0]    = 'F';

    // Self-referential struct traversal.
    struct Node n1 = { .value = 1, .next = 0 };
    struct Node n2 = { .value = 2, .next = &n1 };
    struct Node *head = &n2;
    int walk = 0;
    while (head != 0) {
        walk += head->value;
        head  = head->next;
    }

    // Comparator table lookup — array-of-struct indexing plus a call
    // through a function-pointer typedef member.
    int k = COMPARATORS[0].fn(3, 5) + COMPARATORS[2].fn(7, 7);

    // Switch with five cases plus default.
    enum Color col = COLOR_GREEN;
    int classified = -1;
    switch (col) {
        case COLOR_RED:   classified = 10; break;
        case COLOR_GREEN: classified = 20; break;
        case COLOR_BLUE:  classified = 30; break;
        case COLOR_ALL:   classified = 40; break;
        case 99:          classified = 50; break;
        default:          classified =  0; break;
    }

    // For loop with a comma expression in the update step.
    int i, j;
    int acc = 0;
    for (i = 0, j = 7; i < 8 && j >= 0; i++, j--) {
        acc += xs[i] - xs[j];
    }

    // Variadic call.
    int v_sum = sum_variadic(3, 10, 20, 30);

    // _Static_assert inside a block — uses sizeof(type) since
    // sizeof(expression) in constant expressions is not yet evaluable
    // (tracked for a later phase).
    _Static_assert(sizeof(long) >= sizeof(int),
                   "long must be at least as wide as int");

    return (argc + total + walk + k + classified + acc + v_sum) & 0;
}
