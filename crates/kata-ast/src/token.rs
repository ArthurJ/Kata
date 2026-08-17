//! `Token` — terminais produzidos pelo lexer e consumidos pelo parser.
//!
//! A notação prefixa (I1) elimina ambiguidade léxica: `+1` é número
//! positivo, `+ 1` é a função `+` aplicada a `1`. Operadores não são
//! especiais — são identificadores como qualquer outro.

use crate::span::Span;

/// Um token do lexer com seu span.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenWithSpan {
    pub token: Token,
    pub span: Span,
}

/// Tokens terminais da linguagem Kata.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Literais ─────────────────────────────────────────
    /// Inteiro. O texto bruto é preservado para BigInt/SMI no runtime.
    /// Suporta decimal, hex (`0x`), octal (`0o`), binário (`0b`), decimal
    /// explícito (`0d`), e separador visual `_` (descartado léxicamente).
    IntLit(String),
    /// Float. Notação decimal ou científica.
    FloatLit(String),
    /// String de aspas duplas, simples, ou tripla crua.
    /// O conteúdo já unescaped (as aspas foram consumidas pelo lexer).
    TextLit(String),
    /// Byte string literal — `b"Hello"`, `b"\x00\xFF"`.
    /// Conteúdo já unescaped (bytes crus).
    BytesLit(Vec<u8>),

    /// Texto bruto de um literal que será interpretado como Rational
    /// (ex: `3.14` em `3.14::Rational`). O lexer não sabe de Rational —
    /// produz FloatLit e o parser reinterpreta via ascription.
    /// (Não é um token separado — Rational usa FloatLit + `::Rational`.)

    // ── Identificadores ─────────────────────────────────
    /// Identificador — qualquer símbolo não-reservado.
    /// Inclui `+`, `-`, `*`, `/`, `<`, `>`, `=`, `$` — todos são identificadores
    /// válidos usados como nomes de função (consequência da notação prefixa).
    Ident(String),

    // ── Palavras-chave ─────────────────────────────────
    /// `let` — binding imutável
    Let,
    /// `constant` — declara constante de módulo (top-level only)
    Constant,
    /// `var` — binding mutável (exclusivo de Actions, )
    Var,
    /// `data` — declara tipo produto
    Data,
    /// `enum` — declara tipo soma
    Enum,
    /// `alias` — cria newtype
    Alias,
    /// `action` — declara Action
    Action,
    /// `lambda` / `λ` — declara função anônima
    Lambda,
    /// `import` — importa módulo
    Import,
    /// `export` — exporta itens
    Export,
    /// `as` — alias de import ou de tipo
    As,
    /// `interface` — declara contrato de tipo
    Interface,
    /// `implements` — implementa interface
    Implements,
    /// `refines` — delega interface ao tipo base (tipos refined)
    Refines,
    /// `final` — bloqueia extensão de enum
    Final,
    /// `extends` — herda variantes de enum base
    Extends,
    /// `with` — bloco bottom-up ao final de lambda
    With,
    /// `match` — pattern matching
    Match,
    /// `return` — early return em Actions
    Return,
    /// `loop` — laço infinito (exclusivo de Actions, )
    Loop,
    /// `break` — sai de `loop` (exclusivo de Actions, )
    Break,
    /// `continue` — próxima iteração de `loop` (exclusivo de Actions, )
    Continue,
    /// `otherwise` — fallback em guards
    Otherwise,
    /// `for` — iteração via ITERABLE (exclusivo de Actions, )
    For,
    /// `in` — separador em `for x in coll` e operador binário de membership
    In,
    /// `select` — multiplexação de canais CSP
    Select,
    /// `timeout` — cláusula de `select`
    Timeout,
    /// `type` — introspecção compile-time (`type!(expr)`)
    Type,
    /// `directive` — declara diretiva customizada
    Directive,

    // ── Operadores e pontuação ──────────────────────────
    /// `:=` — operador de binding (exclusivo para `let` e `var`)
    BindAssign,
    /// `::` — etiqueta de tipo (assinatura, campo, type param, variante, ascription)
    DoubleColon,
    /// `=>` — declaração de assinatura (separa args de retorno)
    FatArrow,
    /// `->` — tipo de função como valor
    ThinArrow,
    /// `|` — fallback local (coalescência de erro)
    Pipe,
    /// `|>` — pipeline (transformação)
    PipeForward,
    /// `?` — delegação/fail-fast (exclusivo de Actions)
    Question,
    /// `!` — sufixo de chamada de Action
    Bang,
    /// `<!` — envio por canal CSP ("valor entra no canal")
    SendArrow,
    /// `!>` — recebimento por canal CSP ("valor sai do canal")
    RecvArrow,
    /// `$` — spread/aplicação explícita (identificador interceptado pelo typeck)
    /// (Não é keyword — é Ident("$"). O lexer produz Ident para `$`.)

    // ── Delimitadores ───────────────────────────────────
    /// `(` — abre parênteses
    LParen,
    /// `)` — fecha parênteses
    RParen,
    /// `[` — abre colchetes
    LBracket,
    /// `]` — fecha colchetes
    RBracket,
    /// `{` — abre chaves
    LBrace,
    /// `}` — fecha chaves
    RBrace,

    // ── Separadores ────────────────────────────────────
    /// `,` — separador de tupla
    Comma,
    /// `.` — acesso de campo / indexação
    Dot,
    /// `..` — separador de componentes em Range: `[a..s..b]`
    DotDot,
    /// `..=` — separador de end inclusivo em Range: `[a..s..=b]`
    DotDotEq,
    /// `;` — terminador de statement (Actions) ou separador de dimensão (tensores)
    Semicolon,
    /// `:` — separa guard/pattern do corpo
    Colon,

    // ── Diretivas ──────────────────────────────────────
    /// `@nome` — diretiva prefixando item de topo
    /// O lexer produz `At` e o parser constrói `Directive`.
    At,

    // ── Tokens sintéticos (indent-sensitive) ───────────
    /// Indica aumento de indentação
    Indent,
    /// Indica diminuição de indentação
    Dedent,
    /// Separador de statements (quebra de linha em contexto de statement)
    StmtSep,

    // ──EOF ─────────────────────────────────────────────
    /// Fim do arquivo
    Eof,
}

impl Token {
    /// Verifica se o token é um literal.
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Token::IntLit(_) | Token::FloatLit(_) | Token::TextLit(_) | Token::BytesLit(_)
        )
    }

    /// Verifica se o token é uma palavra-chave.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            Token::Let
                | Token::Constant
                | Token::Var
                | Token::Data
                | Token::Enum
                | Token::Alias
                | Token::Action
                | Token::Lambda
                | Token::Import
                | Token::Export
                | Token::As
                | Token::Interface
                | Token::Implements
                | Token::Refines
                | Token::Final
                | Token::Extends
                | Token::With
                | Token::Match
                | Token::Return
                | Token::Loop
                | Token::Break
                | Token::Continue
                | Token::Otherwise
                | Token::For
                | Token::In
                | Token::Select
                | Token::Timeout
                | Token::Type
                | Token::Directive
        )
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::IntLit(s) | Token::FloatLit(s) | Token::TextLit(s) => write!(f, "{s}"),
            Token::BytesLit(bytes) => write!(
                f,
                "b\"{}\"",
                bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            ),
            Token::Ident(s) => write!(f, "{s}"),
            Token::Let => write!(f, "let"),
            Token::Constant => write!(f, "constant"),
            Token::Var => write!(f, "var"),
            Token::Data => write!(f, "data"),
            Token::Enum => write!(f, "enum"),
            Token::Alias => write!(f, "alias"),
            Token::Action => write!(f, "action"),
            Token::Lambda => write!(f, "lambda"),
            Token::Import => write!(f, "import"),
            Token::Export => write!(f, "export"),
            Token::As => write!(f, "as"),
            Token::Interface => write!(f, "interface"),
            Token::Implements => write!(f, "implements"),
            Token::Refines => write!(f, "refines"),
            Token::Final => write!(f, "final"),
            Token::Extends => write!(f, "extends"),
            Token::With => write!(f, "with"),
            Token::Match => write!(f, "match"),
            Token::Return => write!(f, "return"),
            Token::Loop => write!(f, "loop"),
            Token::Break => write!(f, "break"),
            Token::Continue => write!(f, "continue"),
            Token::Otherwise => write!(f, "otherwise"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::Select => write!(f, "select"),
            Token::Timeout => write!(f, "timeout"),
            Token::Type => write!(f, "type"),
            Token::Directive => write!(f, "directive"),
            Token::BindAssign => write!(f, ":="),
            Token::DoubleColon => write!(f, "::"),
            Token::FatArrow => write!(f, "=>"),
            Token::ThinArrow => write!(f, "->"),
            Token::Pipe => write!(f, "|"),
            Token::PipeForward => write!(f, "|>"),
            Token::Question => write!(f, "?"),
            Token::Bang => write!(f, "!"),
            Token::SendArrow => write!(f, "<!"),
            Token::RecvArrow => write!(f, "!>"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::Comma => write!(f, ","),
            Token::Dot => write!(f, "."),
            Token::DotDot => write!(f, ".."),
            Token::DotDotEq => write!(f, "..="),
            Token::Semicolon => write!(f, ";"),
            Token::Colon => write!(f, ":"),
            Token::At => write!(f, "@"),
            Token::Indent => write!(f, "<INDENT>"),
            Token::Dedent => write!(f, "<DEDENT>"),
            Token::StmtSep => write!(f, "<STMT_SEP>"),
            Token::Eof => write!(f, "<EOF>"),
        }
    }
}
