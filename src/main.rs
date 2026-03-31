mod cli;
mod codegen;
mod lexer;
mod optimizer;
mod parser;
mod repl;
mod type_checker;

// Importa a biblioteca kata_rt compilada separadamente
use kata_rt;

use clap::Parser;
use cli::{Cli, Commands};

fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    // Configurar o logger baseado na verbosidade (-v, -vv, -vvv)
    let log_level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        2 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    };

    env_logger::Builder::new().filter_level(log_level).init();

    log::info!("Kata Compiler iniciado.");

    // Informar se flags de depuração estão ativas
    if cli.dump_tokens {
        log::info!("Flag ativada: --dump-tokens");
    }
    if cli.dump_ast {
        log::info!("Flag ativada: --dump-ast");
    }
    if cli.dump_tast {
        log::info!("Flag ativada: --dump-tast");
    }

    match &cli.command {
        Commands::Build {
            entrypoint,
            output,
            release,
        } => {
            log::info!("Comando: BUILD");
            log::info!("Entrypoint: {}", entrypoint);
            log::info!("Output: {:?}", output);
            log::info!("Release mode: {}", release);

            // Tenta ler o arquivo
            let source = std::fs::read_to_string(&entrypoint)
                .map_err(|e| miette::miette!("Falha ao ler o arquivo {}: {}", entrypoint, e))?;

            // 1. Lexer
            let tokens = match lexer::lex(&source, lexer::LexMode::File) {
                Ok(t) => t,
                Err(errs) => {
                    for err in errs {
                        let msg = err.to_string();
                        ariadne::Report::build(ariadne::ReportKind::Error, (entrypoint.clone(), err.span()))
                            .with_message("Erro Léxico")
                            .with_label(
                                ariadne::Label::new((entrypoint.clone(), err.span()))
                                    .with_message(&msg)
                                    .with_color(ariadne::Color::Red),
                            )
                            .finish()
                            .eprint((entrypoint.clone(), ariadne::Source::from(&source)))
                            .unwrap();
                    }
                    return Err(miette::miette!("Falha na análise léxica."));
                }
            };

            if cli.dump_tokens {
                println!("--- TOKENS ---");
                for (t, s) in &tokens {
                    println!("{:?} {:?}", t, s);
                }
            }

            // 2. Parser
            let source_len = source.chars().count();
            let module = match parser::parse_module(tokens.clone(), source_len) {
                Ok(m) => m,
                Err(errs) => {
                    for err in errs {
                        let msg = if let chumsky::error::SimpleReason::Unexpected = err.reason() {
                            format!(
                                "Token inesperado: {}",
                                err.found()
                                    .map(|t| t.to_string())
                                    .unwrap_or_else(|| "fim de arquivo".to_string())
                            )
                        } else {
                            err.to_string()
                        };

                        ariadne::Report::build(ariadne::ReportKind::Error, (entrypoint.clone(), err.span()))
                            .with_message(&msg)
                            .with_label(
                                ariadne::Label::new((entrypoint.clone(), err.span()))
                                    .with_message(msg.clone())
                                    .with_color(ariadne::Color::Red),
                            )
                            .finish()
                            .eprint((entrypoint.clone(), ariadne::Source::from(&source)))
                            .unwrap();
                    }
                    return Err(miette::miette!("Falha na análise sintática."));
                }
            };

            if cli.dump_ast {
                println!("--- AST PLANA ---");
                println!("{:#?}", module);
            }

            // 3. Type Checker (Arity Resolution & Types)
            let mut checker = type_checker::Checker::new();
            
            // Carrega os modulos do core (Prelude)
            let core_files = [
                ("types", "src/core/types.kata"), 
                ("io", "src/core/io.kata"), 
                ("csp", "src/core/csp.kata"), 
                ("assert", "src/core/assert.kata"), 
                ("prelude", "src/core/prelude.kata")
            ];
            let mut prelude_modules = Vec::new();
            for (name, file) in core_files {
                match std::fs::read_to_string(file) {
                    Ok(src) => {
                        match lexer::lex(&src, lexer::LexMode::File) {
                            Ok(toks) => {
                                match parser::parse_module(toks, src.len()) {
                                    Ok(m) => prelude_modules.push((name, m)),
                                    Err(e) => log::error!("Erro ao parsear o prelude {}: {:?}", file, e),
                                }
                            }
                            Err(e) => log::error!("Erro ao fazer lex no prelude {}: {:?}", file, e),
                        }
                    }
                    Err(e) => log::error!("Erro ao ler o arquivo {}: {:?}", file, e),
                }
            }
            checker.load_prelude(&prelude_modules);

            checker.check_module(&module);

            if !checker.errors.is_empty() {
                log::error!("Erros Semanticos detectados na Fase 3:");
                for e in &checker.errors {
                    log::error!("{}", e.0);
                }
            }

            if cli.dump_tast {
                println!("--- TAST (RESOLVIDA) ---");
                println!("{:#?}", checker.tast);
            }

            let mut opt = optimizer::Optimizer::new(&checker.env);
            let optimized_tast = opt.optimize(checker.tast, *release);

            if !opt.errors.is_empty() {
                log::error!("Erros de Otimizacao detectados na Fase 4:");
                for e in &opt.errors {
                    log::error!("{}", e.message);
                }
                return Err(miette::miette!("Falha na otimizacao."));
            }

            if cli.dump_tast {
                println!("--- TAST (OTIMIZADA) ---");
                println!("{:#?}", optimized_tast);
            }

            // codegen::run_stub();
            let out_bin = output.clone().unwrap_or_else(|| {
                std::path::Path::new(&entrypoint)
                    .file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string()
            });

            if let Err(e) = codegen::compile_and_link(optimized_tast, &out_bin) {
                log::error!("Erro de Compilacao/Codegen: {}", e);
                return Err(miette::miette!("Falha na geracao de codigo nativo."));
            }
            log::info!("Binario gerado com sucesso: {}", out_bin);
        }
        Commands::Run {
            entrypoint,
            release,
        } => {
            log::info!("Comando: RUN");
            log::info!("Entrypoint: {}", entrypoint);
            log::info!("Release mode: {}", release);

            // parser::run_stub();
            // type_checker::run_stub();
            // kata_rt::init_stub();
        }
        Commands::Test { path } => {
            log::info!("Comando: TEST");
            log::info!("Path: {}", path);

            // Tenta ler o arquivo
            let source = std::fs::read_to_string(&path)
                .map_err(|e| miette::miette!("Falha ao ler o arquivo de teste {}: {}", path, e))?;

            // 1. Lexer
            let tokens = match lexer::lex(&source, lexer::LexMode::File) {
                Ok(t) => t,
                Err(_) => return Err(miette::miette!("Falha na análise léxica do teste.")),
            };

            // 2. Parser
            let source_len = source.chars().count();
            let module = match parser::parse_module(tokens.clone(), source_len) {
                Ok(m) => m,
                Err(_) => return Err(miette::miette!("Falha na análise sintática do teste.")),
            };

            // 3. Type Checker
            let mut checker = type_checker::Checker::new();
            let core_files = [
                ("types", "src/core/types.kata"), 
                ("io", "src/core/io.kata"), 
                ("csp", "src/core/csp.kata"), 
                ("assert", "src/core/assert.kata"), 
                ("prelude", "src/core/prelude.kata")
            ];
            let mut prelude_modules = Vec::new();
            for (name, file) in core_files {
                if let Ok(src) = std::fs::read_to_string(file) {
                    if let Ok(toks) = lexer::lex(&src, lexer::LexMode::File) {
                        if let Ok(m) = parser::parse_module(toks, src.len()) {
                            prelude_modules.push((name, m));
                        }
                    }
                }
            }
            checker.load_prelude(&prelude_modules);
            checker.check_module(&module);

            println!("--- TESTES ENCONTRADOS ---");
            if checker.tests.is_empty() {
                println!("Nenhum bloco de teste anotado com @test encontrado.");
            } else {
                for (i, t) in checker.tests.iter().enumerate() {
                    let tipo = if t.is_action { "[Action / Impuro]" } else { "[Função / Puro]" };
                    println!("{}) {} {} - \"{}\"", i + 1, tipo, t.name, t.description);
                }
                println!("\nPronto para gerar o Entrypoint Sintético para {} teste(s) via Cranelift (Fase 4/6).", checker.tests.len());
            }
        }
        Commands::Repl => {
            log::info!("Comando: REPL");
            repl::start_stub();
        }
    }

    Ok(())
}
