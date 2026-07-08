use clap::Parser;

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
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file: _ } => {
            eprintln!("kata lex: ainda não implementado (Fio 1)");
        }
        Command::Parse { file: _ } => {
            eprintln!("kata parse: ainda não implementado (Fio 1)");
        }
        Command::Eval { expr: _ } => {
            eprintln!("kata eval: ainda não implementado (Fio 1)");
        }
        Command::Run { file: _ } => {
            eprintln!("kata run: ainda não implementado (Fio 1)");
        }
    }
}