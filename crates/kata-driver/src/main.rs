use std::path::{Path, PathBuf};

use clap::Parser;
use kata_codegen::{TestWrapper, aot_emit, jit_compile_tests, jit_eval};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_rt as rt;
use kata_tree_shaking::tree_shake;

mod display;
mod repl;

/// CLI do compilador Kata.
#[derive(Parser)]
#[command(name = "kata", version, about = "Compilador da linguagem Kata")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Análise léxica — imprime tokens com spans
    Lex { file: String },
    /// Análise sintática — imprime AST via Debug pretty-print
    Parse { file: String },
    /// Avalia expressão via JIT, imprime resultado
    Eval { expr: String },
    /// Compila e executa arquivo via JIT
    Run { file: String },
    /// Descobre e executa testes `@test` em arquivo ou diretório
    Test {
        /// Arquivo `.kata` ou diretório com `*.kata` (recursivo)
        path: String,
        /// Filtra testes por substring na descrição
        #[arg(long)]
        filter: Option<String>,
    },
    /// Compila programa Kata para executável nativo (AOT)
    Build {
        /// Arquivo `.kata` de entrada
        file: String,
        /// Path de saída (default: nome do arquivo sem extensão no cwd)
        #[arg(short, long)]
        output: Option<String>,
        /// Link dinâmico contra libkata_rt.so (default: estático)
        #[arg(long)]
        dynamic: bool,
    },
    /// REPL interativo — TypeEnv persistente + JIT fresco por expressão
    Repl,
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
        Command::Parse { file } => cmd_parse(&file),
        Command::Eval { expr } => cmd_eval(&expr),
        Command::Run { file } => cmd_run(&file),
        Command::Test { path, filter } => cmd_test(&path, filter.as_deref()),
        Command::Build {
            file,
            output,
            dynamic,
        } => cmd_build(&file, output.as_deref(), dynamic),
        Command::Repl => repl::cmd_repl(),
    }
}

// ── Conversão de erros para miette::Report ──────────────────

/// Converte um erro que implementa `miette::Diagnostic` em `miette::Report`.
trait IntoReport {
    fn into_report(self) -> miette::Report;
}

impl<E: miette::Diagnostic + Send + Sync + 'static> IntoReport for E {
    fn into_report(self) -> miette::Report {
        miette::Report::new_boxed(Box::new(self))
    }
}

// ── Comandos ───────────────────────────────────────────────

fn cmd_lex(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(IntoReport::into_report)?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn cmd_parse(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(IntoReport::into_report)?;
    let module = parse(tokens).map_err(IntoReport::into_report)?;
    println!("{module:#?}");
    Ok(())
}

fn cmd_eval(expr: &str) -> miette::Result<()> {
    let result = run_pipeline(expr)?;
    display::print_result(result.raw, &result.ty);
    Ok(())
}

fn cmd_run(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let result = run_pipeline(&source)?;
    display::print_result(result.raw, &result.ty);
    Ok(())
}

// ── Test runner ────────────────────────────────────────────

/// Resultado da execução de um único caso de teste.
enum TestOutcome {
    Pass,
    Timeout,
    Deadlock,
}

/// Executa o subcomando `kata test`.
///
/// Descobre arquivos `.kata` (arquivo único ou diretório recursivo),
/// compila cada um via `jit_compile_tests`, e executa os wrappers
/// `__kata_test_*` individualmente com scheduler fresco + timeout.
fn cmd_test(path: &str, filter: Option<&str>) -> miette::Result<()> {
    let files = discover_kata_files(path)?;
    if files.is_empty() {
        eprintln!("nenhum arquivo .kata encontrado em `{path}`");
        return Ok(());
    }

    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut total_skip = 0usize;

    for file in &files {
        let source = read_source(&file.to_string_lossy())?;
        let label = file.display();

        // Pipeline até jit_compile_tests.
        let tokens = lex(&source).map_err(IntoReport::into_report)?;
        let module = parse(tokens).map_err(IntoReport::into_report)?;
        let prelude = load_prelude()
            .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
        let user = resolve(&module)
            .map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
        let resolved = merge_resolved(prelude, user);
        let typed = infer_module(&module, &resolved).map_err(IntoReport::into_report)?;
        let typed = monomorphize(typed);
        let typed = optimize(typed);

        let (jit_module, wrappers) = jit_compile_tests(&typed)
            .map_err(|e| miette::Report::msg(format!("erro de codegen: {e:?}")))?;

        for w in &wrappers {
            let desc = w.spec.desc.as_deref().unwrap_or("(sem desc)");

            // Filtro por substring na descrição.
            if let Some(f) = filter
                && !desc.contains(f)
            {
                total_skip += 1;
                continue;
            }

            // Negativos CompileError não têm wrapper — não há nada para
            // executar. O driver compila o sub-módulo isolado.
            // Por ora, reporta como pendente.
            if w.spec
                .expects
                .as_deref()
                .is_some_and(|e| e.starts_with("CompileError:"))
            {
                println!("  [PENDENTE] {label}: {desc} (negativo CompileError)");
                total_skip += 1;
                continue;
            }

            let outcome = run_test_wrapper(&jit_module, w);

            match outcome {
                TestOutcome::Pass => {
                    println!("  [PASS] {label}: {desc}");
                    total_pass += 1;
                }
                TestOutcome::Timeout => {
                    println!("  [TIMEOUT] {label}: {desc}");
                    total_fail += 1;
                }
                TestOutcome::Deadlock => {
                    println!("  [DEADLOCK] {label}: {desc}");
                    total_fail += 1;
                }
            }
        }
    }

    println!(
        "\n{} passed, {} failed, {} skipped",
        total_pass, total_fail, total_skip
    );

    if total_fail > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Executa um wrapper de teste individualmente.
///
/// Cada teste roda em scheduler fresco: `reset_scheduler` +
/// `kata_rt_set_test_timeout(N)` + chamada do wrapper. O wrapper é
/// `() -> i64` com `CallConv::SystemV` — autossuficiente.
fn run_test_wrapper(module: &cranelift_jit::JITModule, w: &TestWrapper) -> TestOutcome {
    // Resetar estado global entre testes.
    rt::reset_scheduler();

    // Configurar timeout — opt-in. Sem `@test{timeout: N}`, o teste
    // roda até completar ou deadlock (TIMEOUT_EXPIRED fica false).
    if let Some(ms) = w.spec.timeout {
        rt::kata_rt_set_test_timeout(ms);
    }

    // Obter ponteiro do wrapper compilado.
    let code = module.get_finalized_function(w.func_id);

    // SAFETY: `code` é ponteiro válido após finalize_definitions. O wrapper
    // é `extern "C" fn() -> i64` — autossuficiente (faz scheduler_init +
    // spawn + run internamente).
    let result: i64 = unsafe { std::mem::transmute::<*const u8, extern "C" fn() -> i64>(code)() };

    if result == rt::TIMEOUT_SENTINEL {
        TestOutcome::Timeout
    } else if result == rt::DEADLOCK_SENTINEL {
        TestOutcome::Deadlock
    } else {
        // Sucesso — o valor retornado é o resultado da action.
        TestOutcome::Pass
    }
}

/// Descobre arquivos `.kata` — arquivo único ou diretório recursivo.
fn discover_kata_files(path: &str) -> miette::Result<Vec<PathBuf>> {
    let p = Path::new(path);
    if p.is_file() {
        return Ok(vec![p.to_path_buf()]);
    }
    if !p.is_dir() {
        return Err(miette::Report::msg(format!(
            "caminho não é arquivo nem diretório: `{path}`"
        )));
    }
    let mut files = Vec::new();
    collect_kata_files(p, &mut files);
    files.sort();
    Ok(files)
}

/// Coleta arquivos `.kata` recursivamente.
fn collect_kata_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_kata_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "kata") {
            out.push(path);
        }
    }
}

// ── Pipeline ───────────────────────────────────────────────

/// Resultado da execução — valor bruto + tipo para display.
pub(crate) struct ExecResult {
    pub raw: i64,
    pub ty: Ty,
}

/// Executa o pipeline completo: lex → parse → resolve → infer → optimize → codegen → JIT.
fn run_pipeline(source: &str) -> miette::Result<ExecResult> {
    // 1. Lex
    let tokens = lex(source).map_err(IntoReport::into_report)?;

    // 2. Parse
    let module = parse(tokens).map_err(IntoReport::into_report)?;

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = load_prelude()
        .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
    let user =
        resolve(&module).map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
    let resolved = merge_resolved(prelude, user);

    // 4. Infer (typeck + dispatch)
    let typed = infer_module(&module, &resolved).map_err(IntoReport::into_report)?;

    // 5. Monomorph (especializa call sites genéricos)
    let mono = monomorphize(typed);

    // 6. Optimize (TRMA + futuros passes)
    let mono = optimize(mono);

    // 7. Codegen + JIT + executar
    let jit =
        jit_eval(&mono).map_err(|e| miette::Report::msg(format!("erro de codegen: {e:?}")))?;

    Ok(ExecResult {
        raw: jit.raw,
        ty: jit.ty,
    })
}

/// Combina prelude + módulo do usuário em um ResolvedModule único.
pub(crate) fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    let mut signatures = prelude.signatures;
    signatures.extend(user.signatures);

    // TypeEnv: prelude é o escopo base, user é filho.
    let mut type_env = kata_core::ty::TypeEnv::with_parent(prelude.type_env);
    // Copia bindings do user (enums, structs declarados pelo usuário) para o escopo filho.
    let mut user_type_env = user.type_env;
    type_env.merge_bindings_from(&mut user_type_env);

    // Merge enum_registry: prelude + user (user enums sobrescrevem prelude).
    let mut enum_registry = prelude.enum_registry;
    enum_registry.merge(user.enum_registry);
    let mut struct_registry = prelude.struct_registry;
    struct_registry.merge(user.struct_registry);

    let mut refined_decls = prelude.refined_decls;
    refined_decls.extend(user.refined_decls);

    let mut enum_pred_decls = prelude.enum_pred_decls;
    enum_pred_decls.extend(user.enum_pred_decls);

    let mut interface_registry = prelude.interface_registry;
    interface_registry.merge(user.interface_registry);

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        functions: user.functions,
        actions: user.actions,
    }
}

// ── AOT Build ──────────────────────────────────────────────

/// Executa o subcomando `kata build`.
///
/// Pipeline: lex → parse → resolve → infer → monomorph → optimize →
/// tree_shake → aot_emit → link. O resultado é um executável nativo.
fn cmd_build(file: &str, output: Option<&str>, dynamic: bool) -> miette::Result<()> {
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
    let prelude = load_prelude()
        .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
    let user =
        resolve(&module).map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
    let resolved = merge_resolved(prelude, user);
    let typed = infer_module(&module, &resolved).map_err(IntoReport::into_report)?;

    // Monomorph + optimize.
    let mono = monomorphize(typed);
    let mono = optimize(mono);

    // Tree shaking — remove @test e funções não alcançadas (só AOT).
    let shaken = tree_shake(mono.inner);

    // AOT emit — produz object file (.o) bytes.
    let object_bytes = aot_emit(&shaken)
        .map_err(|e| miette::Report::msg(format!("erro de codegen AOT: {e:?}")))?;

    // Determinar o tipo de retorno do entry point para o tag de display.
    let ret_ty = shaken.entry.node.ty.clone();
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

// ── Helpers ────────────────────────────────────────────────

/// Lê o conteúdo de um arquivo.
fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
