pub fn link_executable(object_file: &str, output_bin: &str) -> Result<(), String> {
    log::info!("Iniciando Linker: conectando {} com kata-rt", object_file);

    let main_c_path = "kata_entry.c";
    let main_c_content = r#"
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
void* kata_rt_alloc_shared(long size, long align) { return malloc(size + 16) + 16; /* dummy ARC struct offset */ }
void kata_rt_decref(void* ptr) { 
    if (ptr) {
        free(ptr - 16); 
    }
}
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
    char* buf = malloc(64);
    snprintf(buf, 64, "Object@%p", a);
    return buf;
}

char* kata_rt_repr_text(char* a) {
    if (!a) return "null";
    char* buf = malloc(1024);
    snprintf(buf, 1024, "\"%s\"", a);
    return buf;
}

char* kata_rt_bool_to_str(char a) {
    return a ? "True" : "False";
}

char* kata_rt_type(void* a) {
    return "UnknownType";
}

char* kata_rt_format(char* template, void* arg) {
    return "formatted";
}

void* kata_rt_fields(void* a) {
    return NULL;
}

char* kata_rt_concat_text(char* a, char* b) {
    if (!a && !b) return "";
    if (!a) return b;
    if (!b) return a;
    size_t len_a = 0; while(a[len_a]) len_a++;
    size_t len_b = 0; while(b[len_b]) len_b++;
    char* buf = malloc(len_a + len_b + 1);
    for(size_t i=0; i<len_a; i++) buf[i] = a[i];
    for(size_t i=0; i<len_b; i++) buf[len_a + i] = b[i];
    buf[len_a + len_b] = '\0';
    return buf;
}

char kata_rt_eq_generic(void* a, void* b) {
    return a == b;
}

char kata_rt_eq_enum(char* a, char* b) {
    if (!a || !b) return 0;
    return *a == *b;
}

void kata_rt_print_str(const char* ptr) {
    if (ptr) {
        printf("%s\n", ptr);
    } else {
        printf("null\n");
    }
}

int main(int argc, char** argv) {
    kata_rt_boot(kata_main);
    return 0;
}
"#;
    std::fs::write(main_c_path, main_c_content).map_err(|e| format!("Falha ao gerar o entrypoint C: {}", e))?;

    let status = std::process::Command::new("cc")
        .arg(main_c_path)
        .arg(object_file)
        .arg("-o")
        .arg(output_bin)
        .status()
        .map_err(|e| format!("Falha ao invocar o compilador C (cc): {}", e))?;

    if !status.success() {
        return Err(format!("Linker falhou com status: {}", status));
    }

    log::info!("Linkagem concluída com sucesso. Executável: {}", output_bin);

    Ok(())
}
