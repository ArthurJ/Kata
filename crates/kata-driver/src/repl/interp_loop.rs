//! REPL loop interpretador — `kata repl --interp`.
//!
//! Mesma I/O do REPL JIT (rustyline, multiline heuristics, history),
//! mas usando `InterpReplSession` (interpretador tree-walking) em vez
//! de `ReplSession` (codegen Cranelift).

use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;

use crate::highlight::KataHelper;
use crate::repl::interp_session::InterpReplSession;

/// Executa o subcomando `kata repl --interp`.
pub(crate) fn cmd_repl_interp() -> miette::Result<()> {
    let mut session = InterpReplSession::new().map_err(miette::Report::msg)?;

    let config = rustyline::config::Builder::new()
        .color_mode(rustyline::config::ColorMode::Forced)
        .build();
    let mut rl = Editor::<KataHelper, DefaultHistory>::with_config(config)
        .map_err(|e| miette::Report::msg(format!("erro ao iniciar rustyline: {e}")))?;
    rl.set_helper(Some(KataHelper::default()));

    let _ = rl.load_history(&session.history_path);

    println!("Kata REPL (interp) — digite :help para comandos, :quit para sair");

    loop {
        let first = match rl.readline(">>> ") {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("erro de leitura: {e}");
                break;
            }
        };

        let trimmed = first.trim();
        if trimmed.starts_with(':') || trimmed.is_empty() {
            let _ = rl.add_history_entry(&first);
            match session.handle(&first) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => eprintln!("{e}"),
            }
            continue;
        }

        // Heurística multiline (igual ao REPL JIT).
        let first_trimmed = first.trim_end();
        let multiline_sig = first_trimmed.contains("::")
            && first_trimmed.contains("=>")
            && !first_trimmed.contains("@ffi");
        let multiline_action = first_trimmed.ends_with("=>");
        let first_token = first_trimmed.split_whitespace().next().unwrap_or("");
        let multiline_indent = matches!(first_token, "match" | "enum" | "implements" | "interface");

        let mut buffer = first.clone();
        let in_multiline = multiline_sig || multiline_action || multiline_indent;

        loop {
            if !in_multiline {
                if !InterpReplSession::is_input_incomplete(&buffer) {
                    break;
                }
            }
            match rl.readline("   ... ") {
                Ok(line) => {
                    if line.trim().is_empty() {
                        break;
                    }
                    buffer.push('\n');
                    buffer.push_str(&line);
                    if (multiline_sig || multiline_indent)
                        && !line.starts_with(' ')
                        && !line.starts_with('\t')
                        && !line.trim_start().starts_with("lambda")
                        && !line.trim_start().starts_with("λ")
                    {
                        break;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    buffer.clear();
                    break;
                }
                Err(ReadlineError::Eof) => break,
                Err(e) => {
                    eprintln!("erro de leitura: {e}");
                    break;
                }
            }
        }

        let _ = rl.add_history_entry(&buffer);
        match session.handle(&buffer) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => eprintln!("{e}"),
        }
    }

    let _ = rl.save_history(&session.history_path);
    Ok(())
}
