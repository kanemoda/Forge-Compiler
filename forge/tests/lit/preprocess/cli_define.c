// RUN: forge -E -D CUSTOM_VAL=777 -D SQR(x)=((x)*(x)) -U LEGACY %s
//
// Command-line `-D` flags must be visible to the source as if they had
// been `#define`d at the top of the file.  The `-U` flag undoes a prior
// `-D` or built-in definition before the source is preprocessed.

int v = CUSTOM_VAL;
int s = SQR(9);

#ifdef LEGACY
int legacy_marker;
#else
int modern_marker = 1;
#endif

// CHECK: int v = 777;
// CHECK: int s = ((9)*(9));
// CHECK: int modern_marker = 1;
