use crate::parser::ast::{Directive, Expr, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Error" => Some(Self::Error),
            "Warn" => Some(Self::Warn),
            "Info" => Some(Self::Info),
            "Debug" => Some(Self::Debug),
            "Trace" => Some(Self::Trace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackpressurePolicy {
    Block,
    Drop,
}

impl BackpressurePolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "block" | "Block" => Some(Self::Block),
            "drop" | "Drop" => Some(Self::Drop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CacheStrategyType {
    LFU, // TODO default, usando a lib Ristretto do Rust
    LRU,
    RR,
    FIFO,
    MRU,
}

impl CacheStrategyType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "lru" => Some(Self::LRU),
            "lfu" => Some(Self::LFU),
            "rr" | "random" => Some(Self::RR),
            "fifo" => Some(Self::FIFO),
            "mru" => Some(Self::MRU),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KataDirective {
    Log {
        level: LogLevel,
        msg: Option<String>,
        topic: Option<String>,
        on_full: BackpressurePolicy,
    },
    Test {
        desc: String,
        expects: Option<String>,
    },
    Ffi(String),
    Commutative,
    Associative(Option<Expr>),
    Comptime,
    Parallel,
    Restart(String),
    CacheStrategy {
        strategy: CacheStrategyType,
        size: Option<usize>,
        ttl: Option<usize>,
    },
}

pub fn validate_and_parse_directives(
    dirs: &[crate::parser::ast::Spanned<Directive>],
) -> (Vec<crate::parser::ast::Spanned<KataDirective>>, Vec<(crate::errors::KataError, Span)>) {
    let mut parsed = Vec::new();
    let mut errors = Vec::new();

    let valid_names = ["log", "test", "ffi", "commutative", "associative", "comptime", "parallel", "restart"];

    for (dir, span) in dirs {
        if !valid_names.contains(&dir.name.as_str()) {
            errors.push((
                crate::errors::KataError::SyntaxError(format!("Diretiva desconhecida ou invalida: @{}. Diretivas suportadas: {:?}", dir.name, valid_names)),
                span.clone(),
            ));
            continue;
        }

        match dir.name.as_str() {
            "log" => {
                if match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.is_empty(), _ => false } {
                    errors.push((crate::errors::KataError::SyntaxError("Diretiva @log exige argumentos: @log(Level, \"Mensagem\", \"Topic\", \"OnFull\"). Exemplo: @log(Info, \"Start\", \"metrics\", \"drop\")".to_string()), span.clone()));
                    continue;
                }

                let level_arg = &match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[0], _ => unreachable!() };
                let level = if let Expr::Ident(l) = &level_arg.0 {
                    if let Some(lvl) = LogLevel::from_str(l) {
                        lvl
                    } else {
                        errors.push((crate::errors::KataError::SyntaxError(format!("Diretiva @log: Level '{}' invalido. Use Error, Warn, Info, Debug ou Trace.", l)), level_arg.1.clone()));
                        continue;
                    }
                } else {
                    errors.push((crate::errors::KataError::SyntaxError("Diretiva @log: O primeiro argumento deve ser o Identificador do Nivel (ex: Info, Warn).".to_string()), level_arg.1.clone()));
                    continue;
                };

                let msg = if match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.len(), _ => 0 } > 1 {
                    if let Expr::String(s) = &match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[1], _ => unreachable!() }.0 {
                        Some(s.clone())
                    } else {
                        errors.push((crate::errors::KataError::SyntaxError("Diretiva @log: O segundo argumento (msg) deve ser uma String literal.".to_string()), match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[1], _ => unreachable!() }.1.clone()));
                        None
                    }
                } else { None };

                let topic = if match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.len(), _ => 0 } > 2 {
                    if let Expr::String(s) = &match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[2], _ => unreachable!() }.0 {
                        Some(s.clone())
                    } else {
                        errors.push((crate::errors::KataError::SyntaxError("Diretiva @log: O terceiro argumento (topic) deve ser uma String literal.".to_string()), match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[2], _ => unreachable!() }.1.clone()));
                        None
                    }
                } else { None };

                let on_full = if match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.len(), _ => 0 } > 3 {
                    if let Expr::String(s) = &match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[3], _ => unreachable!() }.0 {
                        if let Some(policy) = BackpressurePolicy::from_str(s.as_str()) {
                            policy
                        } else {
                            errors.push((crate::errors::KataError::SyntaxError(format!("Diretiva @log: A politica on_full '{}' e invalida. Use \"block\" ou \"drop\".", s)), match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[3], _ => unreachable!() }.1.clone()));
                            BackpressurePolicy::Block
                        }
                    } else {
                        errors.push((crate::errors::KataError::SyntaxError("Diretiva @log: O quarto argumento (on_full) deve ser uma String literal (\"block\" ou \"drop\").".to_string()), match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[3], _ => unreachable!() }.1.clone()));
                        BackpressurePolicy::Block
                    }
                } else {
                    BackpressurePolicy::Block // Default
                };

                parsed.push((KataDirective::Log { level, msg, topic, on_full }, span.clone()));
            }
            "test" => {
                match &dir.args {
                    crate::parser::ast::DirectiveArgs::Positional(p) => {
                        let desc = if let Some((Expr::String(s), _)) = p.first() {
                            s.clone()
                        } else {
                            "Sem descricao".to_string()
                        };
                        parsed.push((KataDirective::Test { desc, expects: None }, span.clone()));
                    }
                    crate::parser::ast::DirectiveArgs::Named(n) => {
                        let mut desc = "Sem descricao".to_string();
                        let mut expects = None;

                        for (k, v) in n {
                            match k.as_str() {
                                "desc" => {
                                    if let Expr::String(s) = &v.0 { desc = s.clone(); }
                                    else { errors.push((crate::errors::KataError::SyntaxError("O argumento 'desc' deve ser uma String literal.".to_string()), v.1.clone())); }
                                }
                                "expects" => {
                                    if let Expr::String(s) = &v.0 { expects = Some(s.clone()); }
                                    else { errors.push((crate::errors::KataError::SyntaxError("O argumento 'expects' deve ser uma String literal (ex: \"CompileError\", \"Panic\").".to_string()), v.1.clone())); }
                                }
                                _ => errors.push((crate::errors::KataError::SyntaxError(format!("Argumento nomeado '{}' desconhecido para @test.", k)), v.1.clone())),
                            }
                        }

                        if expects.as_deref() != Some("CompileError") && expects.as_deref() != Some("Panic") && expects.is_some() {
                            errors.push((crate::errors::KataError::SyntaxError(format!("Valor invalido para 'expects': {:?}. Valores permitidos: \"CompileError\", \"Panic\".", expects)), span.clone()));
                        }

                        parsed.push((KataDirective::Test { desc, expects }, span.clone()));
                    }
                }
            }
            "ffi" => {
                if match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.is_empty(), _ => false } {
                    errors.push((crate::errors::KataError::SyntaxError("Diretiva @ffi exige o nome do simbolo externo como argumento em formato texto (ex: @ffi(\"kata_rt_print\"))".to_string()), span.clone()));
                } else if let Expr::String(s) = &match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[0], _ => unreachable!() }.0 {
                    parsed.push((KataDirective::Ffi(s.clone()), span.clone()));
                } else {
                    errors.push((crate::errors::KataError::SyntaxError("O argumento da diretiva @ffi deve ser uma String literal.".to_string()), match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => &p[0], _ => unreachable!() }.1.clone()));
                }
            }
            "commutative" => {
                parsed.push((KataDirective::Commutative, span.clone()));
            }
            "associative" => {
                let id = if let Some((expr, _)) = match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.first(), _ => None } {
                    Some(expr.clone() /* type known */)
                } else { None };
                parsed.push((KataDirective::Associative(id), span.clone()));
            }
            "comptime" => {
                parsed.push((KataDirective::Comptime, span.clone()));
            }
            "parallel" => {
                parsed.push((KataDirective::Parallel, span.clone()));
            }
            "restart" => {
                let policy = if let Some((Expr::String(s), _)) = match &dir.args { crate::parser::ast::DirectiveArgs::Positional(p) => p.first(), _ => None } {
                    s.clone()
                } else {
                    "always".to_string()
                };
                parsed.push((KataDirective::Restart(policy), span.clone()));
            }
            "cache_strategy" => {
                let mut strategy = CacheStrategyType::LRU; // Default
                let mut size = None;
                let mut ttl = None;

                match &dir.args {
                    crate::parser::ast::DirectiveArgs::Named(args) => {
                        for (k, v) in args {
                            match k.as_str() {
                                "strategy" => {
                                    if let Expr::String(s) = &v.0 {
                                        if let Some(strat) = CacheStrategyType::from_str(s) {
                                            strategy = strat;
                                        } else {
                                            errors.push((crate::errors::KataError::SyntaxError(format!("Estrategia de cache '{}' desconhecida. Use 'lru', 'lfu', 'rr', 'fifo' ou 'mru'.", s)), v.1.clone()));
                                        }
                                    }
                                    else { errors.push((crate::errors::KataError::SyntaxError("O argumento 'strategy' deve ser uma String literal.".to_string()), v.1.clone())); }
                                }
                                "size" => {
                                    if let Expr::Int(s) = &v.0 { size = s.parse().ok(); }
                                    else { errors.push((crate::errors::KataError::SyntaxError("O argumento 'size' deve ser um Inteiro literal.".to_string()), v.1.clone())); }
                                }
                                "ttl" => {
                                    if let Expr::Int(s) = &v.0 { ttl = s.parse().ok(); }
                                    else { errors.push((crate::errors::KataError::SyntaxError("O argumento 'ttl' deve ser um Inteiro literal.".to_string()), v.1.clone())); }
                                }
                                _ => errors.push((crate::errors::KataError::SyntaxError(format!("Argumento nomeado '{}' desconhecido para @cache_strategy.", k)), v.1.clone())),
                            }
                        }
                    }
                    _ => {
                        errors.push((crate::errors::KataError::SyntaxError("Diretiva @cache_strategy exige argumentos nomeados. Ex: @cache_strategy{strategy: \"lru\", size: 1000}".to_string()), span.clone()));
                    }
                }
                parsed.push((KataDirective::CacheStrategy { strategy, size, ttl }, span.clone()));
            }
            _ => {}
        }
    }

    (parsed, errors)
}