//! AOT build — compilação de programa Kata para executável nativo.
//!
//! Este módulo contém o pipeline de AOT (`cmd_build`), a conversão de
//! tipos para tags do runtime (`ty_to_type_tag`), e o linker C
//! (`link` + `find_linker`). O restante do driver (CLI dispatch, pipeline
//! JIT, test runner) vive em `main.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use kata_codegen::aot_emit;
use kata_comptime::run_comptime_pass;
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{load_prelude, resolve};
use kata_rt as rt;
use kata_tree_shaking::tree_shake;

use crate::type_table;

use crate::imports::{load_module_imports, merge_imports};
use crate::{IntoReport, merge_resolved, read_source};

/// Executa o subcomando `kata build`.
///
/// Pipeline: lex → parse → resolve → infer → monomorph → optimize →
/// tree_shake → aot_emit → link. O resultado é um executável nativo.
pub(crate) fn cmd_build(file: &str, output: Option<&str>, dynamic: bool) -> miette::Result<()> {
    // Determinar path de saída — default: nome do arquivo sem extensão no cwd.
    let output_path = match output {
        Some(p) => PathBuf::from(p),
        None => {
            let p = Path::new(file);
            let stem = p
                .file_stem()
                .ok_or_else(|| miette::Report::msg(format!("arquivo sem nome: `{file}`")))?
                .to_string_lossy()
                .into_owned();
            PathBuf::from(stem)
        }
    };

    // Pipeline até TypedModule.
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(IntoReport::into_report)?;
    let module = parse(tokens).map_err(IntoReport::into_report)?;

    // Carregar módulos importados (se houver)
    let imports = load_module_imports(file, &module)?;

    let prelude = load_prelude()
        .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
    let user =
        resolve(&module).map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
    let mut resolved = merge_resolved(prelude, user);
    merge_imports(&mut resolved, &imports);
    let typed = infer_module(&module, &resolved).map_err(IntoReport::into_report)?;

    // Monomorph + optimize.
    let mono = monomorphize(typed);
    let mono = optimize(mono);

    // Tree shaking — remove @test e funções não alcançadas (só AOT).
    let shaken = tree_shake(mono.inner);

    // Comptime pass — avalia expressões @comptime em compile-time e
    // substitui por literais/snapshots antes do codegen AOT.
    let shaken = run_comptime_pass(shaken, &resolved.enum_registry)
        .map_err(|e| miette::Report::msg(format!("erro de comptime: {e}")))?;

    // Type table — registra TypeShapes no runtime para to_bytes/from_bytes.
    let mono = kata_monomorph::MonoModule::from(shaken);
    let type_id_map: HashMap<Ty, i64> =
        type_table::build_and_register_type_table(&mono, &mono.struct_registry, &resolved.enum_registry);

    // AOT emit — produz object file (.o) bytes.
    let object_bytes = aot_emit(&mono, &type_id_map)
        .map_err(|e| miette::Report::msg(format!("erro de codegen AOT: {e:?}")))?;

    // Determinar o tipo de retorno do entry point para o tag de display.
    let ret_ty = mono.entry.node.ty.clone();
    let type_tag = ty_to_type_tag(&ret_ty);

    // Link — produz executável.
    link(&object_bytes, &output_path, dynamic, type_tag)
        .map_err(|e| miette::Report::msg(format!("erro de link: {e}")))?;

    eprintln!("compilado: {} → {}", file, output_path.display());
    Ok(())
}

/// Converte `Ty` do entry point para o tag serializável do runtime.
fn ty_to_type_tag(ty: &Ty) -> i32 {
    use kata_core::ty::PrimTy;
    match ty {
        Ty::Prim(PrimTy::Int) => rt::TYPE_INT,
        Ty::Prim(PrimTy::Float) => rt::TYPE_FLOAT,
        Ty::Prim(PrimTy::Text) => rt::TYPE_TEXT,
        Ty::Prim(PrimTy::Rational) => rt::TYPE_RATIONAL,
        Ty::Sum(name) if name == "Boolean" => rt::TYPE_BOOLEAN,
        Ty::Unit => rt::TYPE_UNIT,
        _ => rt::TYPE_OTHER,
    }
}

/// Linka um object file (.o) do Cranelift com libkata_rt e um shim C
/// para produzir um executável nativo.
///
/// O shim C chama `__kata_entry` e `kata_rt_print_result` — display
/// vive no runtime, não há duplicação de lógica.
fn link(object_bytes: &[u8], output: &Path, dynamic: bool, type_tag: i32) -> Result<(), String> {
    // Workspace root (definido por build.rs).
    let build_root = env!("KATA_BUILD_ROOT");
    let target_dir = Path::new(build_root).join("target");

    // Profile: se o binário do driver está em target/debug, usamos debug;
    // se está em target/release, usamos release. Heurística: checar qual
    // libkata_rt.a existe. Default: debug.
    let profile_dir = if target_dir.join("release").join("libkata_rt.a").exists()
        && !target_dir.join("debug").join("libkata_rt.a").exists()
    {
        "release"
    } else {
        "debug"
    };
    let lib_dir = target_dir.join(profile_dir);

    // Descobrir linker (cc, gcc, clang — primeiro disponível).
    let cc = find_linker()
        .ok_or_else(|| "linker não encontrado: instale cc, gcc ou clang".to_string())?;

    // Diretório temporário para o shim e o .o do Cranelift.
    let tmp = std::env::temp_dir().join(format!("kata-build-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)
        .map_err(|e| format!("não foi possível criar dir temporário: {e}"))?;

    // Escrever o .o do Cranelift.
    let cranelift_o = tmp.join("kata_module.o");
    std::fs::write(&cranelift_o, object_bytes)
        .map_err(|e| format!("não foi possível escrever .o: {e}"))?;

    // Gerar shim C que chama __kata_entry + kata_rt_print_result.
    //
    // Float é especial: __kata_entry retorna f64 via XMM0 (SystemV ABI),
    // não i64 via RAX. O shim declara o retorno correto conforme o type_tag.
    // Para Float, declara `double __kata_entry(void)` e bitcasta para i64
    // antes de passar para kata_rt_print_result (que faz from_bits).
    let shim_c = tmp.join("kata_shim.c");
    let entry_decl = if type_tag == rt::TYPE_FLOAT {
        "double __kata_entry(void)"
    } else {
        "int64_t __kata_entry(void)"
    };
    let call_and_print = if type_tag == rt::TYPE_FLOAT {
        format!(
            r#"    double result_f64 = __kata_entry();
    // bitcast double → int64_t para kata_rt_print_result (que faz from_bits)
    int64_t result;
    __builtin_memcpy(&result, &result_f64, sizeof(result));
    kata_rt_print_result(result, {type_tag});"#
        )
    } else {
        format!(
            r#"    int64_t result = __kata_entry();
    kata_rt_print_result(result, {type_tag});"#
        )
    };
    let shim_source = format!(
        r#"#include <stdint.h>

extern {entry_decl};
extern void kata_rt_print_result(int64_t raw, int32_t type_tag);

int main(void) {{
{call_and_print}
    return 0;
}}
"#,
    );
    std::fs::write(&shim_c, &shim_source)
        .map_err(|e| format!("não foi possível escrever shim C: {e}"))?;

    // Compilar shim C → .o
    let shim_o = tmp.join("kata_shim.o");
    let status = std::process::Command::new(&cc)
        .args(["-c", "-o"])
        .arg(&shim_o)
        .arg(&shim_c)
        .status()
        .map_err(|e| format!("falha ao invocar {cc}: {e}"))?;
    if !status.success() {
        return Err(format!("falha ao compilar shim C (cc retornou {status})"));
    }

    // Linkar: cc -o <output> <shim.o> <cranelift.o> -L<lib_dir> -lkata_rt -lm -lpthread
    let mut cmd = std::process::Command::new(&cc);
    cmd.args(["-o"]).arg(output).arg(&shim_o).arg(&cranelift_o);

    if dynamic {
        // Link dinâmico: -lkata_rt resolve contra libkata_rt.so
        cmd.arg(format!("-L{}", lib_dir.display()));
        cmd.arg("-lkata_rt");
        cmd.args(["-lm", "-lpthread"]);
        // rpath para encontrar libkata_rt.so em runtime
        cmd.arg(format!("-Wl,-rpath,{}", lib_dir.display()));
    } else {
        // Link estático: linka libkata_rt.a diretamente
        let static_lib = lib_dir.join("libkata_rt.a");
        cmd.arg(&static_lib);
        cmd.args(["-lm", "-lpthread"]);
    }

    let status = cmd
        .status()
        .map_err(|e| format!("falha ao invocar linker {cc}: {e}"))?;
    if !status.success() {
        return Err(format!("falha ao linkar (cc retornou {status})"));
    }

    // Limpeza do diretório temporário.
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(())
}

/// Encontra um linker disponível: cc, gcc, ou clang.
fn find_linker() -> Option<String> {
    for name in &["cc", "gcc", "clang"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}
