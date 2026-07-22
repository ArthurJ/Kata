use std::path::{Path, PathBuf};

use clap::Parser;
use kata_codegen::{TestWrapper, jit_compile_tests, jit_eval};
use kata_core::ty::Ty;
use kata_inference::infer_module;
use kata_lexer::lex;
use kata_monomorph::monomorphize;
use kata_optimizer::optimize;
use kata_parser::parse;
use kata_resolution::{ResolvedModule, load_prelude, resolve};
use kata_rt as rt;
use kata_tree_shaking::tree_shake;

mod aot;
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
        } => aot::cmd_build(&file, output.as_deref(), dynamic),
        Command::Repl => repl::cmd_repl(),
    }
}

// ── Conversão de erros para miette::Report ──────────────────

/// Converte um erro que implementa `miette::Diagnostic` em `miette::Report`.
pub(crate) trait IntoReport {
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
    // Unit de retorno de `main` não carrega informação — o output do
    // programa já foi produzido via echo!/_print!. Suprimir o `()`.
    if !matches!(result.ty, Ty::Unit) {
        display::print_result(result.raw, &result.ty);
    }
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

        // Tree shaking preservando testes — remove actions não alcançadas
        // (ex: `echo` original type-erased após monomorfização) mas mantém
        // TypedTestSpec nas actions alcançadas para que jit_compile_tests
        // gere os wrappers `__kata_test_*`.
        let typed = kata_monomorph::MonoModule::from(kata_tree_shaking::tree_shake_preserve_tests(
            typed.inner,
        ));

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

    // 6a. Tree shaking — remove funções/actions não alcançados.
    //     Necessário para descartar Actions polimórficas originais
    //     (ex: `echo :: SHOW`) após o monomorphizador instanciar
    //     versões concretas (ex: `echo_SHOW_Int`). Sem isso, o codegen
    //     tenta compilar o body type-erased e falha.
    let mono = kata_monomorph::MonoModule::from(tree_shake(mono.inner));

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

    let mut refines_registry = prelude.refines_registry;
    refines_registry.merge(user.refines_registry);

    // Functions/actions: concatenar prelude + user, removendo duplicatas
    // por nome (user sobrescreve prelude quando redefine). Sem isso,
    // funções Kata da stdlib (mod, and) têm assinatura no DispatchTable mas
    // não geram TypedFunction — codegen falha com UnsupportedNode.
    let mut functions = prelude.functions;
    let user_fn_names: std::collections::HashSet<&str> =
        user.functions.iter().map(|f| f.name.as_str()).collect();
    functions.retain(|f| !user_fn_names.contains(f.name.as_str()));
    functions.extend(user.functions);

    let mut actions = prelude.actions;
    let user_action_names: std::collections::HashSet<&str> =
        user.actions.iter().map(|a| a.name.as_str()).collect();
    actions.retain(|a| !user_action_names.contains(a.name.as_str()));
    actions.extend(user.actions);

    ResolvedModule {
        type_env,
        signatures,
        enum_registry,
        struct_registry,
        refined_decls,
        enum_pred_decls,
        interface_registry,
        refines_registry,
        functions,
        actions,
    }
}

// ── Helpers ────────────────────────────────────────────────

/// Lê o conteúdo de um arquivo.
pub(crate) fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
