use std::path::{Path, PathBuf};

use clap::Parser;
use kata_codegen::TestWrapper;
use kata_core::ty::Ty;
use kata_lexer::lex;
use kata_parser::parse;
use kata_resolution::ResolvedModule;
use kata_rt as rt;

mod aot;
mod display;
mod highlight;
mod imports;
mod pipeline;
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
    /// Inicia o servidor LSP (Language Server Protocol) em stdio
    Lsp,
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
        Command::Lsp => cmd_lsp(),
    }
}

// ── LSP ────────────────────────────────────────────────────

/// Inicia o servidor LSP em stdio. Delega para `kata_lsp::run_stdio`.
fn cmd_lsp() -> miette::Result<()> {
    kata_lsp::run_stdio();
    Ok(())
}

// ── Conversão de erros para miette::Report ──────────────────

/// Converte um erro que implementa `miette::Diagnostic` em `miette::Report`
/// com `NamedSource` anexado, habilitando source context no miette
/// (linha de código + indicador de posição).
pub(crate) trait IntoReport: miette::Diagnostic + Send + Sync + Sized + 'static {
    /// Cria um `Report` com `NamedSource` anexado, habilitando source
    /// context no miette (linha de código + indicador de posição).
    ///
    /// - `source`: código-fonte completo do arquivo onde o erro ocorreu.
    /// - `file`: path do arquivo (ou `None` para eval/REPL → `<eval>`).
    fn into_report_with_source(self, source: &str, file: Option<&str>) -> miette::Report {
        let name = file.unwrap_or("<eval>");
        let named = miette::NamedSource::new(name, source.to_string());
        miette::Report::new_boxed(Box::new(self)).with_source_code(named)
    }
}

impl<E: miette::Diagnostic + Send + Sync + 'static> IntoReport for E {}

/// Formata um `Vec` de erros como string legível (um por linha).
/// `Vec<ResolveError>` não implementa `Display`, então `{e:?}` mostraria
/// `Debug` — incluindo spans brutos. Este helper usa `Display` de cada
/// erro individual, que delega para `thiserror::Error::fmt`.
pub(crate) fn format_error_vec<E: std::fmt::Display>(errors: &[E]) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Converte `Vec<Report>` (erros de uma fase do pipeline) em um único
/// `miette::Report` para propagar em `miette::Result`.
///
/// - Se há exatamente 1 erro, retorna-o diretamente — o miette imprime
///   com o formato padrão (preservando snapshots existentes).
/// - Se há múltiplos erros, imprime cada um via `eprintln!` (sem o
///   prefixo `Error: ` que o `main` adicionaria) e retorna um `Report`
///   de resumo ("N erros encontrados").
pub(crate) fn print_pipeline_errors(errors: Vec<miette::Report>) -> miette::Report {
    if errors.len() == 1 {
        return errors.into_iter().next().unwrap();
    }
    let n = errors.len();
    for report in &errors {
        eprintln!("{report:?}");
    }
    miette::Report::msg(format!("{n} erro(s) encontrado(s)"))
}

// ── Comandos ───────────────────────────────────────────────

fn cmd_lex(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn cmd_parse(file: &str) -> miette::Result<()> {
    let source = read_source(file)?;
    let tokens = lex(&source).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
    let module = parse(tokens).map_err(|e| e.into_report_with_source(&source, Some(file)))?;
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
        let compiled = (|| -> Result<_, Vec<miette::Report>> {
            pipeline::Pipeline::new(&source)
                .with_file_path(&file.to_string_lossy())
                .lex()?
                .parse(pipeline::ParseMode::Single, Some(&file.to_string_lossy()))?
                .resolve(Some(&file.to_string_lossy()))?
                .desugar()
                .infer()?
                .monomorph()
                .optimize()
                .tree_shake(pipeline::ShakeMode::PreserveTests)?
                .comptime()?
                .build_type_table()
        })()
        .map_err(crate::print_pipeline_errors)?;

        let type_shapes = compiled.type_shapes.clone();
        let (jit_module, wrappers) = compiled.jit_tests()?;

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

            let outcome = run_test_wrapper(&jit_module, w, &type_shapes);

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
/// Cada teste roda em Runtime fresco: `reset_scheduler` +
/// `kata_rt_set_test_timeout(N)` + chamada do wrapper. O wrapper é
/// `(rt: i64) -> i64` com `CallConv::SystemV` — autossuficiente.
fn run_test_wrapper(
    module: &cranelift_jit::JITModule,
    w: &TestWrapper,
    type_shapes: &[kata_rt::TypeShape],
) -> TestOutcome {
    // Resetar estado global (timer + TLS periféricas) entre testes.
    rt::reset_scheduler();

    // Configurar timeout — opt-in. Sem `@test{timeout: N}`, o teste
    // roda até completar ou deadlock (TIMEOUT_EXPIRED fica false).
    if let Some(ms) = w.spec.timeout {
        rt::kata_rt_set_test_timeout(ms);
    }

    // A2: Alocar Runtime fresco para cada teste.
    let runtime = Box::new(rt::Runtime::new());
    let rt_ptr = Box::into_raw(runtime) as i64;

    // A2: Registrar type_shapes no Runtime (marshalling to_bytes/from_bytes).
    if !type_shapes.is_empty() {
        rt::register_type_table(rt_ptr, type_shapes.to_vec());
    }

    // Obter ponteiro do wrapper compilado.
    let code = module.get_finalized_function(w.func_id);

    // SAFETY: `code` é ponteiro válido após finalize_definitions. O wrapper
    // é `extern "C" fn(i64) -> i64` — autossuficiente (faz scheduler_init +
    // spawn + run internamente).
    let result: i64 = unsafe {
        std::mem::transmute::<*const u8, extern "C" fn(i64) -> i64>(code)(rt_ptr)
    };

    // A2: Descartar Runtime após a execução (Drop libera arenas).
    // SAFETY: rt_ptr foi alocado acima; a execução já terminou.
    unsafe { drop(Box::from_raw(rt_ptr as *mut rt::Runtime)) };

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

/// Executa o pipeline completo (JIT): lex → parse (two-pass) → resolve →
/// infer → monomorph → optimize → tree-shake → comptime → type-table →
/// codegen + execução.
fn run_pipeline(source: &str) -> miette::Result<ExecResult> {
    run_pipeline_with_file(source, None)
}

/// Executa o pipeline completo com caminho do arquivo (para resolver imports).
///
/// Delega a sequência de compilação ao `Pipeline` composicional (A1).
/// Cada passo existe uma vez — este wrapper só escolhe os modos (two-pass,
/// tree-shake default) e termina com `jit_eval`.
fn run_pipeline_with_file(source: &str, file_path: Option<&str>) -> miette::Result<ExecResult> {
    let compiled = (|| -> Result<_, Vec<miette::Report>> {
        pipeline::Pipeline::new(source)
            .with_file_path(file_path.unwrap_or("<eval>"))
            .lex()?
            .parse(pipeline::ParseMode::TwoPass, file_path)?
            .resolve(file_path)?
            .desugar()
            .infer()?
            .monomorph()
            .optimize()
            .tree_shake(pipeline::ShakeMode::Default)?
            .comptime()?
            .build_type_table()
    })()
    .map_err(crate::print_pipeline_errors)?;

    let ty = compiled.entry_ty();
    let raw = compiled.jit_eval()?;

    Ok(ExecResult { raw, ty })
}

/// Combina prelude + módulo do usuário em um ResolvedModule único.
/// Delega para `kata_resolution::merge_two` (compartilhado com ModuleLoader).
pub(crate) fn merge_resolved(prelude: ResolvedModule, user: ResolvedModule) -> ResolvedModule {
    kata_resolution::merge_two(prelude, user)
}

// ── Helpers ────────────────────────────────────────────────

/// Lê o conteúdo de um arquivo.
pub(crate) fn read_source(path: &str) -> miette::Result<String> {
    let path = Path::new(path);
    std::fs::read_to_string(path)
        .map_err(|e| miette::Report::msg(format!("não foi possível ler `{}`: {e}", path.display())))
}
