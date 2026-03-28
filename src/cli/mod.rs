use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kata")]
#[command(version = "0.1.0")]
#[command(about = "O compilador da linguagem Kata", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Aumenta o nível de verbosidade do compilador (ex: -v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Imprime a saída linear produzida pelo Lexer
    #[arg(long, global = true)]
    pub dump_tokens: bool,

    /// Imprime a Árvore Sintática Bruta (Plana / Sequences) produzida pelo Parser
    #[arg(long, global = true)]
    pub dump_ast: bool,

    /// Imprime a Árvore Tipada Resolvida (TAST) produzida pelo Type Checker
    #[arg(long, global = true)]
    pub dump_tast: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compila o módulo principal gerando um binário executável nativo (AOT)
    Build {
        /// Caminho para o arquivo .kata inicial
        entrypoint: String,

        /// Caminho/nome do binário gerado (padrão: nome do arquivo de entrada)
        #[arg(short, long)]
        output: Option<String>,

        /// Habilita otimizações para produção (Cranelift e Tree-Shaking)
        #[arg(long)]
        release: bool,
    },
    /// Compila e executa o código imediatamente (JIT / Temporário)
    Run {
        /// Caminho para o arquivo .kata inicial
        entrypoint: String,
        
        /// Habilita otimizações para produção (Cranelift e Tree-Shaking)
        #[arg(long)]
        release: bool,
    },
    /// Vasculha o diretório ou arquivo específico executando blocos marcados com @test
    Test {
        /// Caminho opcional (padrão é o diretório atual '.')
        #[arg(default_value = ".")]
        path: String,
    },
    /// Inicia o ambiente de execução interativa (REPL JIT)
    Repl,
}