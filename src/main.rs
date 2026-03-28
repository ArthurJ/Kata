mod cli;
mod codegen;
mod kata_rt;
mod lexer;
mod parser;
mod repl;
mod type_checker;

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

            type_checker::run_stub();
            codegen::run_stub();
        }
        Commands::Run {
            entrypoint,
            release,
        } => {
            log::info!("Comando: RUN");
            log::info!("Entrypoint: {}", entrypoint);
            log::info!("Release mode: {}", release);

            parser::run_stub();
            type_checker::run_stub();
            kata_rt::init_stub();
        }
        Commands::Test { path } => {
            log::info!("Comando: TEST");
            log::info!("Path: {}", path);
        }
        Commands::Repl => {
            log::info!("Comando: REPL");
            repl::start_stub();
        }
    }

    Ok(())
}
