//! `Ty` — tipo canônico do compilador.
//!
//! O typeck produz `Ty` em cada `TypedExpr.ty`. O lowering mapeia `Ty`
//! direto para ABI do Cranelift (Int→I64, Float→F64, Text/Struct/Sum→Ptr).
//!
//! `PrimTy` é o mapeamento de representação FFI (não tipo da linguagem).
//! Os tipos da linguagem (`Int`, `Float`, `Text`, `Rational`) são `data`
//! com `@ffi` no prelude. `Boolean` é `enum` no prelude.

use std::collections::{HashMap, HashSet};

/// Tipo canônico do compilador.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// Int, Float, Text, Rational — tipos opacos do prelude com @ffi.
    Prim(PrimTy),
    /// `()` — unit.
    Unit,
    /// Tipo struct (produto) — `data Pessoa (nome::Text idade::Int)`.
    Struct(String),
    /// Tipo soma (enum) — `enum Boolean { True, False }`.
    Sum(String),
    /// Tipo função — `(A B -> C)`.
    Function(Vec<Ty>, Box<Ty>),
    /// Antecipado. Sem field access, sem .N — só tipo estrutural
    /// para suportar tuple patterns em match/lambda.
    Tuple(Vec<Ty>),
    /// Variável de inferência — preenchida pelo typeck.
    InferVar(u32),
    /// Variável de tipo nomeada pelo usuário (ex: `T`, `E` em `Result::(T, E)`).
    /// Distinta de InferVar — Var é parâmetro de tipo nomeado, InferVar é
    /// gerada internamente pelo typeck.
    Var(String),
    /// Tipo genérico instanciado (ex: `Result<Int, Text>`).
    /// Carrega o nome do enum + os argumentos de tipo concretos.
    Generic(String, Vec<Ty>),
    /// Interface (ex: `NUM`, `ORD`, `EQ`, `SHOW`).
    /// Usada como tipo de parâmetro em assinaturas para indicar
    /// "qualquer tipo que implementa esta interface". Não é um tipo
    /// concreto — o dispatch resolve para o tipo real no call site.
    Interface(String),
    /// Lista persistente: `[T]` — Cons cell encadeada.
    List(Box<Ty>),
    /// Array contíguo: `{T}` — bloco imutável.
    Array(Box<Ty>),
    /// Range lazy: `[a..s..b]` — start, step, end. Genérico sobre A.
    Range(Box<Ty>),
    /// Sender de canal — `Sender::T`. Pode fazer `!>`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Sender(Box<Ty>),
    /// Receiver de canal — `Receiver::T`. Pode fazer `<!`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Receiver(Box<Ty>),
    /// Fábrica de receivers para broadcast — `ReceiverFactory::T`.
    /// Chamada como action produz `Receiver::T`.
    ReceiverFactory(Box<Ty>),
}

/// Mapeamento de representação FFI.
/// Não é tipo da linguagem — é como o codegen mapeia `Ty → ABI`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    /// Int → i64 (BigInt/SMI no runtime).
    Int,
    /// Float → f64.
    Float,
    /// Text → ponteiro para kata_rt_string.
    Text,
    /// Rational → ponteiro para kata_rt_rat.
    Rational,
}

impl PrimTy {
    /// Nome do símbolo FFI de representação.
    pub fn ffi_repr(self) -> &'static str {
        match self {
            PrimTy::Int => "i64",
            PrimTy::Float => "f64",
            PrimTy::Text => "kata_rt_string",
            PrimTy::Rational => "kata_rt_rat",
        }
    }

    /// Constrói PrimTy a partir do nome de símbolo FFI.
    pub fn from_ffi(s: &str) -> Option<PrimTy> {
        match s {
            "i64" => Some(PrimTy::Int),
            "f64" => Some(PrimTy::Float),
            "kata_rt_string" => Some(PrimTy::Text),
            "kata_rt_rat" => Some(PrimTy::Rational),
            _ => None,
        }
    }
}

impl Ty {
    /// `Ty::Prim(PrimTy::Int)` — conveniência.
    pub fn int() -> Ty {
        Ty::Prim(PrimTy::Int)
    }

    /// `Ty::Prim(PrimTy::Float)` — conveniência.
    pub fn float() -> Ty {
        Ty::Prim(PrimTy::Float)
    }

    /// `Ty::Prim(PrimTy::Text)` — conveniência.
    pub fn text() -> Ty {
        Ty::Prim(PrimTy::Text)
    }

    /// `Ty::Prim(PrimTy::Rational)` — conveniência.
    pub fn rational() -> Ty {
        Ty::Prim(PrimTy::Rational)
    }

    /// `Ty::Sum("Boolean")` — conveniência para Boolean.
    pub fn boolean() -> Ty {
        Ty::Sum("Boolean".into())
    }
}

/// Árvore de escopos para name resolution.
///
/// Populada no resolution (Pass 0+1) e consumida no inference (Pass 2).
/// Não sobrevive além do typeck — a TAST já carrega os tipos resolvidos.
#[derive(Debug, Clone)]
pub struct TypeEnv {
    bindings: HashMap<String, Ty>,
    parent: Option<Box<TypeEnv>>,
    /// Nomes declarados como mutáveis (`var`) neste escopo.
    /// Necessário para validar reatribuição (`x := 42` só é válido se `x`
    /// foi declarado com `var`, não `let`).
    mutables: HashSet<String>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: None,
            mutables: HashSet::new(),
        }
    }

    pub fn with_parent(parent: TypeEnv) -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: Some(Box::new(parent)),
            mutables: HashSet::new(),
        }
    }

    /// Define um nome no escopo atual (binding imutável por default).
    pub fn define(&mut self, name: &str, ty: Ty) {
        self.bindings.insert(name.to_string(), ty);
    }

    /// Define um nome mutável no escopo atual (`var`).
    /// Marca o nome como mutável para validação de reatribuição.
    pub fn define_mutable(&mut self, name: &str, ty: Ty) {
        self.bindings.insert(name.to_string(), ty);
        self.mutables.insert(name.to_string());
    }

    /// Verifica se um nome foi declarado como mutável (`var`).
    /// Percorre a cadeia de escopos.
    pub fn is_mutable(&self, name: &str) -> bool {
        if self.mutables.contains(name) {
            return true;
        }
        self.parent.as_deref().is_some_and(|p| p.is_mutable(name))
    }

    /// Procura um nome na cadeia de escopos.
    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        if let Some(ty) = self.bindings.get(name) {
            return Some(ty);
        }
        self.parent.as_deref().and_then(|p| p.lookup(name))
    }

    /// Cria um escopo filho.
    pub fn push_scope(&self) -> TypeEnv {
        TypeEnv::with_parent(self.clone())
    }

    /// Drena os bindings (e mutables) de `other` para este escopo.
    ///
    /// Usado em `merge_resolved` para combinar o `type_env` do user module
    /// com o escopo filho do prelude. `other` fica vazio após a chamada.
    pub fn merge_bindings_from(&mut self, other: &mut TypeEnv) {
        for (name, ty) in other.bindings.drain() {
            self.bindings.insert(name, ty);
        }
        for name in other.mutables.drain() {
            self.mutables.insert(name);
        }
    }
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Display — sintaxe que o usuário escreve, não Debug de Rust.
// ---------------------------------------------------------------------------

impl std::fmt::Display for PrimTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimTy::Int => f.write_str("Int"),
            PrimTy::Float => f.write_str("Float"),
            PrimTy::Text => f.write_str("Text"),
            PrimTy::Rational => f.write_str("Rational"),
        }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Prim(p) => write!(f, "{p}"),
            Ty::Unit => f.write_str("()"),
            Ty::Var(name) => f.write_str(name),
            Ty::Sum(name) | Ty::Struct(name) | Ty::Interface(name) => f.write_str(name),
            Ty::Generic(name, args) => {
                f.write_str(name)?;
                f.write_str("::(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                f.write_str(")")
            }
            Ty::Tuple(elements) => {
                f.write_str("(")?;
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{elem}")?;
                }
                f.write_str(")")
            }
            Ty::Function(params, ret) => {
                f.write_str("(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{p}")?;
                }
                f.write_str(") -> ")?;
                write!(f, "{ret}")
            }
            Ty::List(inner) => write!(f, "[{inner}]"),
            Ty::Array(inner) => write!(f, "{{{inner}}}"),
            Ty::Range(inner) => write!(f, "[..{inner}]"),
            Ty::Sender(inner) => write!(f, "Sender::{inner}"),
            Ty::Receiver(inner) => write!(f, "Receiver::{inner}"),
            Ty::ReceiverFactory(inner) => write!(f, "ReceiverFactory::{inner}"),
            Ty::InferVar(_) => f.write_str("?"),
        }
    }
}

/// Formata uma lista de tipos como `(T1, T2, T3)` — útil para mensagens
/// de erro que mostram `Vec<Ty>` (ex: params de uma função, args de uma
/// chamada). Usa `Display` em cada elemento.
pub fn ty_list_to_string(tys: &[Ty]) -> String {
    tys.iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
