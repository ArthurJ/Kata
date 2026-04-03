
#include <stdio.h>
#include <stdlib.h>

extern void kata_main();

void kata_rt_boot(void (*main_action)()) {
    main_action();
}

long kata_rt_add_int(long a, long b) { return a + b; }
long kata_rt_sub_int(long a, long b) { return a - b; }
long kata_rt_mul_int(long a, long b) { return a * b; }
long kata_rt_div_int(long a, long b) { return b == 0 ? 0 : a / b; }
long kata_rt_mod_int(long a, long b) { return b == 0 ? 0 : a % b; }
char kata_rt_eq_int(long a, long b) { return a == b; }
char kata_rt_gt_int(long a, long b) { return a > b; }
char kata_rt_ge_int(long a, long b) { return a >= b; }
char kata_rt_lt_int(long a, long b) { return a < b; }
char kata_rt_le_int(long a, long b) { return a <= b; }

double kata_rt_int_to_float(long a) { return (double)a; }
double kata_rt_add_flt(double a, double b) { return a + b; }
double kata_rt_sub_flt(double a, double b) { return a - b; }
double kata_rt_mul_flt(double a, double b) { return a * b; }
double kata_rt_real_div_flt(double a, double b) { return a / b; }
long kata_rt_div_flt(double a, double b) { return (long)(a / b); }
char kata_rt_eq_flt(double a, double b) { return a == b; }
char kata_rt_gt_flt(double a, double b) { return a > b; }
char kata_rt_ge_flt(double a, double b) { return a >= b; }
char kata_rt_lt_flt(double a, double b) { return a < b; }
char kata_rt_le_flt(double a, double b) { return a <= b; }

void* kata_rt_alloc_local(long size, long align) { return malloc(size); }
void* kata_rt_alloc_shared(long size, long align) { return malloc(size); }
void kata_rt_decref(void* ptr) { /* Stub for MVP arc */ }
void* kata_rt_cache_get(long hash) { return NULL; }
void kata_rt_cache_set(long hash, void* ptr) {}

char* kata_rt_int_to_str(long a) {
    char* buf = malloc(32);
    snprintf(buf, 32, "%ld", a);
    return buf;
}

char* kata_rt_flt_to_str(double a) {
    char* buf = malloc(64);
    snprintf(buf, 64, "%f", a);
    return buf;
}

char* kata_rt_default_repr(void* a) {
    return "repr";
}

char kata_rt_eq_generic(void* a, void* b) {
    return a == b;
}

char kata_rt_eq_enum(char* a, char* b) {
    if (!a || !b) return 0;
    return *a == *b;
}

void kata_rt_print_str(const char* ptr) {
    printf("%s\n", ptr);
}

int main(int argc, char** argv) {
    kata_rt_boot(kata_main);
    return 0;
}
