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
    let result = run_pipeline_with_file(&source, Some(file))?;
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

        // Carregar módulos importados (se houver)
        let imports = load_module_imports(&file.to_string_lossy(), &module)?;

        let prelude = load_prelude()
            .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
        let user = resolve(&module)
            .map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
        let mut resolved = merge_resolved(prelude, user);
        merge_imports(&mut resolved, &imports);
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
    run_pipeline_with_file(source, None)
}

/// Executa o pipeline completo com caminho do arquivo (para resolver imports).
fn run_pipeline_with_file(source: &str, file_path: Option<&str>) -> miette::Result<ExecResult> {
    // 1. Lex
    let tokens = lex(source).map_err(IntoReport::into_report)?;

    // 2. Parse
    let module = parse(tokens).map_err(IntoReport::into_report)?;

    // 2a. Carregar módulos importados (se houver)
    let imports = if let Some(file) = file_path {
        load_module_imports(file, &module)?
    } else {
        Vec::new()
    };

    // 3. Resolve (prelude + módulo do usuário)
    let prelude = load_prelude()
        .map_err(|e| miette::Report::msg(format!("erro ao carregar prelude: {e:?}")))?;
    let user =
        resolve(&module).map_err(|e| miette::Report::msg(format!("erro de resolução: {e:?}")))?;
    let mut resolved = merge_resolved(prelude, user);

    // 3a. Merge imports (itens seletivos no escopo direto)
    merge_imports(&mut resolved, &imports);

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
/// Delega para `kata_resolution::merge_two` (compartilhado com ModuleLoader).
pub(crate) fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    kata_resolution::merge_two(prelude, user)
}

/// Mergeia módulos importados no ResolvedModule (prelude + user já mergeados).
///
/// Para cada `ImportedModule`:
/// - `Selective { items }`: traz itens nomeados para o escopo direto (sem prefixo).
/// - `WholeModule { prefix }`: registra cada item exportado com nome qualificado
///   `prefix.item` nas signatures/functions/actions. O inference resolve
///   `mod.fn` como `DotAccess { Ident("mod"), Field("fn") }` procurando
///   `mod.fn` no DispatchTable.
/// - `WholeModuleAliased { alias }`: mesmo que WholeModule mas com prefixo alias.
pub(crate) fn merge_imports(
    merged: &mut ResolvedModule,
    imports: &[kata_resolution::ImportedModule],
) {
    for imported in imports {
        match &imported.import_kind {
            kata_resolution::ImportKind::Selective { items } => {
                // Import seletivo: trazer itens nomeados para o escopo direto.
                // Cada item pode ter alias: `dobrar as d` → registra como `d`.
                for imp_item in items {
                    let target_name = imp_item.alias.as_ref().unwrap_or(&imp_item.name);
                    // Signatures
                    if let Some(sig) = imported
                        .resolved
                        .signatures
                        .iter()
                        .find(|s| s.name == imp_item.name)
                    {
                        if !merged.signatures.iter().any(|s| s.name == *target_name) {
                            let mut renamed = sig.clone();
                            renamed.name = target_name.clone();
                            merged.signatures.push(renamed);
                        }
                    }
                    // Functions
                    if let Some(func) = imported
                        .resolved
                        .functions
                        .iter()
                        .find(|f| f.name == imp_item.name)
                    {
                        if !merged.functions.iter().any(|f| f.name == *target_name) {
                            let mut renamed = func.clone();
                            renamed.name = target_name.clone();
                            merged.functions.push(renamed);
                        }
                    }
                    // Actions
                    if let Some(action) = imported
                        .resolved
                        .actions
                        .iter()
                        .find(|a| a.name == imp_item.name)
                    {
                        if !merged.actions.iter().any(|a| a.name == *target_name) {
                            let mut renamed = action.clone();
                            renamed.name = target_name.clone();
                            merged.actions.push(renamed);
                        }
                    }
                }
            }
            kata_resolution::ImportKind::WholeModule { prefix } => {
                // Módulo inteiro: registrar cada item exportado com nome
                // qualificado `prefix.item`. O inference resolve DotAccess
                // { Ident("mod"), Field("fn") } procurando `mod.fn` no
                // DispatchTable.
                register_qualified(merged, prefix, &imported.resolved);
            }
            kata_resolution::ImportKind::WholeModuleAliased { alias } => {
                register_qualified(merged, alias, &imported.resolved);
            }
        }
    }
}

/// Registra itens de um módulo importado com nome qualificado `prefix.item`.
///
/// Renomeia signatures, functions e actions com o prefixo qualificado.
/// Isso garante consistência em todos os passes:
/// - DispatchTable: signature.name = "mod.fn"
/// - TypedFunction: func.name = "mod.fn" (infer_module usa func_def.name)
/// - symbol_table/kata_ids: chave = ("mod.fn", params, ret)
/// - tree_shaking: fn_names e reached_fns usam "mod.fn"
fn register_qualified(
    merged: &mut ResolvedModule,
    prefix: &str,
    resolved: &kata_resolution::ResolvedModule,
) {
    for sig in &resolved.signatures {
        let qual_name = format!("{prefix}.{}", sig.name);
        if !merged.signatures.iter().any(|s| s.name == qual_name) {
            let mut qual_sig = sig.clone();
            qual_sig.name = qual_name;
            merged.signatures.push(qual_sig);
        }
    }
    for func in &resolved.functions {
        let qual_name = format!("{prefix}.{}", func.name);
        if !merged.functions.iter().any(|f| f.name == qual_name) {
            let mut qual_func = func.clone();
            qual_func.name = qual_name;
            merged.functions.push(qual_func);
        }
    }
    for action in &resolved.actions {
        let qual_name = format!("{prefix}.{}", action.name);
        if !merged.actions.iter().any(|a| a.name == qual_name) {
            let mut qual_action = action.clone();
            qual_action.name = qual_name;
            merged.actions.push(qual_action);
        }
    }
}

// ── Helpers ────────────────────────────────────────────────

/// Carrega módulos importados por um arquivo.
///
/// Cria um `ModuleLoader` com search paths = diretório do arquivo + stdlib.
/// Retorna a lista de `ImportedModule` (vazia se não há imports).
pub(crate) fn load_module_imports(
    file: &str,
    module: &kata_ast::Module,
) -> miette::Result<Vec<kata_resolution::ImportedModule>> {
    use kata_resolution::ModuleLoader;

    let entry_dir = Path::new(file)
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    // stdlib dir: relativo ao CARGO_MANIFEST_DIR do kata-driver.
    // O kata-driver está em crates/kata-driver/, stdlib em stdlib/.
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../stdlib")
        .canonicalize()
        .unwrap_or_else(|_| Path::new("../../stdlib").to_path_buf());

    let search_paths = vec![entry_dir, stdlib_dir];
    let mut loader = ModuleLoader::new(search_paths);
    loader
        .load_imports(module)
        .map_err(|e| miette::Report::msg(format!("erro ao carregar imports: {e:?}")))
}

/// Lê o conteúdo de um arquivo.
pub(crate) fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
