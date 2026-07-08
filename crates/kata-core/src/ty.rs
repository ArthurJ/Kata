//! `Ty` — tipo canônico do compilador.
//!
//! O typeck produz `Ty` em cada `TypedExpr.ty`. O lowering mapeia `Ty`
//! direto para ABI do Cranelift (Int→I64, Float→F64, Text/Struct/Sum→Ptr).
//!
//! `PrimTy` é o mapeamento de representação FFI (não tipo da linguagem).
//! Os tipos da linguagem (`Int`, `Float`, `Text`, `Rational`) são `data`
//! com `@ffi` no prelude. `Boolean` é `enum` no prelude.

use std::collections::HashMap;

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
    /// Antecipado de Fio 5. Sem field access, sem .N — só tipo estrutural
    /// para suportar tuple patterns em match/lambda.
    Tuple(Vec<Ty>),
    /// Variável de inferência — preenchida pelo typeck.
    InferVar(u32),
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
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: TypeEnv) -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    /// Define um nome no escopo atual.
    pub fn define(&mut self, name: &str, ty: Ty) {
        self.bindings.insert(name.to_string(), ty);
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
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}
