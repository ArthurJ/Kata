//! `Ty` — tipo canônico do compilador.
//!
//! O typeck produz `Ty` em cada `TypedExpr.ty`. O lowering mapeia `Ty`
//! direto para ABI do Cranelift (Int→I64, Float→F64, Text/Struct/Sum→Ptr).
//!
//! `PrimTy` é o mapeamento de representação FFI (não tipo da linguagem).
//! Os tipos da linguagem (`Int`, `Float`, `Text`, `Rational`) são `data`
//! com `@ffi` no prelude. `Boolean` é `enum` no prelude.

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
    /// Action como valor first-class.
    /// params: tipos dos parâmetros (sem nomes).
    /// ret: tipo de retorno.
    /// Separada de Ty::Function porque as ABIs são semanticamente diferentes:
    /// Function: (captures_ptr, args) -> ret — pura, sem scheduler
    /// Action: (fiber_arena, caller_arena, args_ptr) -> i64 — impura, scheduler M:N
    Action(Vec<Ty>, Box<Ty>),
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
    /// Dict persistente: `Dict::(K, V)` — HAMT de pares chave-valor.
    Dict(Box<Ty>, Box<Ty>),
    /// Set persistente: `Set::T` — HAMT de chaves (sem values).
    Set(Box<Ty>),
    /// Sender de canal — `Sender::T`. Pode fazer `<!`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Sender(Box<Ty>),
    /// Receiver de canal — `Receiver::T`. Pode fazer `!>`.
    /// Funciona para Channel (rendezvous), Queue (buffered), Broadcast.
    Receiver(Box<Ty>),
    /// Fábrica de receivers para broadcast — `ReceiverFactory::T`.
    /// Chamada como action produz `Receiver::T`.
    ReceiverFactory(Box<Ty>),
    /// Byte — unidade de 8 bits (0x00-0xFF). Distinto de Int.
    /// SMI-tagged no runtime (como Int). Conversão explícita via `int()` / `byte()`.
    Byte,
    /// Bytes — sequência contígua de u8. Blob opaco para I/O e marshalling.
    /// Ponteiro na ABI (como Array, Text, Struct). Imutável.
    Bytes,
    /// File — handle opaco para arquivo aberto. Sem parametrização de tipo
    /// (encoding é determinado pela operação: read → Bytes, readline → Text).
    /// Ponteiro na ABI (i64 com tag). Generaliza para Socket no futuro via
    /// camada IoHandle comum no runtime.
    File,
    /// Socket — handle opaco para socket TCP/Unix aberto. Sem parametrização
    /// de tipo (encoding determinada pela operação: read → Bytes, write aceita
    /// Text ou Bytes). Ponteiro na ABI (i64). Pode ser Listener (passivo) ou
    /// Connected (ativo, full-duplex).
    Socket,
    /// Conjunto de overloads de uma Action referenciada como valor first-class
    /// sem hint de tipo esperado. Interno ao compilador — não exposto na sintaxe.
    /// Resolvido para `Ty::Action` concreto no call site (dispatch por args) ou
    /// quando um hint de tipo esperado desambigua.
    OverloadSet {
        name: String,
        overloads: Vec<(Vec<Ty>, Ty)>,
    },
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

    /// Verifica recursivamente se este tipo contém `Ty::Var` em qualquer
    /// profundidade. Usado pelo monomorphizador para distinguir templates
    /// genéricos (contêm `Var`) de instâncias concretas.
    ///
    /// `Ty::Generic("Result", [Var("T"), Var("E")])` → true (template)
    /// `Ty::Generic("Result", [Int, Sum("MeuErro")])`  → false (instância)
    /// `Ty::Sum("MeuErro")`                           → false (não-genérico)
    pub fn contains_var(&self) -> bool {
        match self {
            Ty::Var(_) => true,
            Ty::Generic(_, args) => args.iter().any(|a| a.contains_var()),
            Ty::Function(params, ret) => {
                params.iter().any(|p| p.contains_var()) || ret.contains_var()
            }
            Ty::Action(params, ret) => {
                params.iter().any(|p| p.contains_var()) || ret.contains_var()
            }
            Ty::Tuple(elements) => elements.iter().any(|e| e.contains_var()),
            Ty::List(inner)
            | Ty::Array(inner)
            | Ty::Range(inner)
            | Ty::Set(inner)
            | Ty::Sender(inner)
            | Ty::Receiver(inner)
            | Ty::ReceiverFactory(inner) => inner.contains_var(),
            Ty::Dict(k, v) => k.contains_var() || v.contains_var(),
            // Folhas sem Var: Prim, Unit, Struct, Sum, InferVar, Interface,
            // Byte, Bytes, File, Socket, OverloadSet.
            _ => false,
        }
    }

    /// Substitui todas as ocorrências de `Ty::Var("Self")` por `replacement`,
    /// recursivamente em tipos compostos. Usado no resolution para instanciar
    /// default methods de interface para um tipo concreto.
    pub fn substitute_self(&self, replacement: &Ty) -> Ty {
        match self {
            Ty::Var(name) if name == "Self" => replacement.clone(),
            Ty::Generic(name, args) => Ty::Generic(
                name.clone(),
                args.iter()
                    .map(|a| a.substitute_self(replacement))
                    .collect(),
            ),
            Ty::Function(params, ret) => Ty::Function(
                params
                    .iter()
                    .map(|p| p.substitute_self(replacement))
                    .collect(),
                Box::new(ret.substitute_self(replacement)),
            ),
            Ty::Action(params, ret) => Ty::Action(
                params
                    .iter()
                    .map(|p| p.substitute_self(replacement))
                    .collect(),
                Box::new(ret.substitute_self(replacement)),
            ),
            Ty::Tuple(elems) => Ty::Tuple(
                elems
                    .iter()
                    .map(|e| e.substitute_self(replacement))
                    .collect(),
            ),
            Ty::List(elem) => Ty::List(Box::new(elem.substitute_self(replacement))),
            Ty::Array(elem) => Ty::Array(Box::new(elem.substitute_self(replacement))),
            Ty::Range(elem) => Ty::Range(Box::new(elem.substitute_self(replacement))),
            Ty::Dict(k, v) => Ty::Dict(
                Box::new(k.substitute_self(replacement)),
                Box::new(v.substitute_self(replacement)),
            ),
            Ty::Set(elem) => Ty::Set(Box::new(elem.substitute_self(replacement))),
            Ty::Sender(elem) => Ty::Sender(Box::new(elem.substitute_self(replacement))),
            Ty::Receiver(elem) => Ty::Receiver(Box::new(elem.substitute_self(replacement))),
            Ty::ReceiverFactory(elem) => {
                Ty::ReceiverFactory(Box::new(elem.substitute_self(replacement)))
            }
            _ => self.clone(),
        }
    }
}

pub use crate::type_env::{TypeBinding, TypeEnv};

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
            Ty::Action(params, ret) => {
                f.write_str("Action(")?;
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
            Ty::Dict(k, v) => write!(f, "Dict::({k}, {v})"),
            Ty::Set(t) => write!(f, "Set::{t}"),
            Ty::Sender(inner) => write!(f, "Sender::{inner}"),
            Ty::Receiver(inner) => write!(f, "Receiver::{inner}"),
            Ty::ReceiverFactory(inner) => write!(f, "ReceiverFactory::{inner}"),
            Ty::InferVar(_) => f.write_str("?"),
            Ty::Byte => f.write_str("Byte"),
            Ty::Bytes => f.write_str("Bytes"),
            Ty::File => f.write_str("File"),
            Ty::Socket => f.write_str("Socket"),
            Ty::OverloadSet { name, .. } => write!(f, "OverloadSet({name})"),
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

impl Ty {
    /// Formata o tipo na **sintaxe da linguagem** — o mesmo formato que o
    /// usuário escreve em assinaturas. Usado por `type!()` para produzir
    /// o `Text` que o usuário vê.
    ///
    /// Diferente de `Display` (que é para mensagens de erro e usa vírgulas
    /// em params de função), `display()` usa a sintaxe real da linguagem:
    /// `(Int Int -> Int)` em vez de `(Int, Int) -> Int`.
    pub fn display(&self) -> String {
        match self {
            Ty::Prim(p) => p.to_string(),
            Ty::Unit => "Unit".into(),
            Ty::Struct(name) | Ty::Sum(name) | Ty::Interface(name) => name.clone(),
            Ty::Var(name) => name.clone(),
            Ty::Generic(name, params) => {
                if params.len() == 1 {
                    format!("{name}::{}", params[0].display())
                } else {
                    let params_str = params
                        .iter()
                        .map(|p| p.display())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}::({params_str})")
                }
            }
            Ty::Function(params, ret) => {
                if params.is_empty() {
                    format!("Lambda(-> {})", ret.display())
                } else {
                    let params_str = params
                        .iter()
                        .map(|p| p.display())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("Lambda({params_str} -> {})", ret.display())
                }
            }
            Ty::Action(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Action({params_str}) -> {}", ret.display())
            }
            Ty::Tuple(elements) => {
                let elems_str = elements
                    .iter()
                    .map(|e| e.display())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({elems_str})")
            }
            Ty::List(t) => format!("[{}]", t.display()),
            Ty::Array(t) => format!("{{{}}}", t.display()),
            Ty::Range(_) => "[a..s..b]".into(),
            Ty::Dict(k, v) => format!("Dict::({}, {})", k.display(), v.display()),
            Ty::Set(t) => format!("Set::{}", t.display()),
            Ty::Sender(t) => format!("Sender::{}", t.display()),
            Ty::Receiver(t) => format!("Receiver::{}", t.display()),
            Ty::ReceiverFactory(t) => format!("ReceiverFactory::{}", t.display()),
            Ty::InferVar(_) => panic!("type!() em tipo não-resolvido — bug do typeck"),
            Ty::Byte => "Byte".into(),
            Ty::Bytes => "Bytes".into(),
            Ty::File => "File".into(),
            Ty::Socket => "Socket".into(),
            Ty::OverloadSet { name, .. } => format!("OverloadSet({name})"),
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn display_prim_int() {
        assert_eq!(Ty::int().display(), "Int");
    }

    #[test]
    fn display_prim_float() {
        assert_eq!(Ty::float().display(), "Float");
    }

    #[test]
    fn display_prim_text() {
        assert_eq!(Ty::text().display(), "Text");
    }

    #[test]
    fn display_prim_rational() {
        assert_eq!(Ty::rational().display(), "Rational");
    }

    #[test]
    fn display_unit() {
        assert_eq!(Ty::Unit.display(), "Unit");
    }

    #[test]
    fn display_struct() {
        assert_eq!(Ty::Struct("Pessoa".into()).display(), "Pessoa");
    }

    #[test]
    fn display_sum() {
        assert_eq!(Ty::Sum("Boolean".into()).display(), "Boolean");
    }

    #[test]
    fn display_interface() {
        assert_eq!(Ty::Interface("NUM".into()).display(), "NUM");
    }

    #[test]
    fn display_var() {
        assert_eq!(Ty::Var("T".into()).display(), "T");
    }

    #[test]
    fn display_generic_single_param() {
        assert_eq!(
            Ty::Generic("Optional".into(), vec![Ty::int()]).display(),
            "Optional::Int"
        );
    }

    #[test]
    fn display_generic_multi_param() {
        assert_eq!(
            Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()]).display(),
            "Result::(Int, Text)"
        );
    }

    #[test]
    fn display_function() {
        assert_eq!(
            Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())).display(),
            "Lambda(Int Int -> Int)"
        );
    }

    #[test]
    fn display_function_no_params() {
        assert_eq!(
            Ty::Function(vec![], Box::new(Ty::int())).display(),
            "Lambda(-> Int)"
        );
    }

    #[test]
    fn display_action() {
        assert_eq!(
            Ty::Action(vec![Ty::int()], Box::new(Ty::Unit)).display(),
            "Action(Int) -> Unit"
        );
    }

    #[test]
    fn display_action_multi_param() {
        assert_eq!(
            Ty::Action(vec![Ty::int(), Ty::text()], Box::new(Ty::int())).display(),
            "Action(Int, Text) -> Int"
        );
    }

    #[test]
    fn display_tuple() {
        assert_eq!(
            Ty::Tuple(vec![Ty::int(), Ty::text()]).display(),
            "(Int, Text)"
        );
    }

    #[test]
    fn display_list() {
        assert_eq!(Ty::List(Box::new(Ty::int())).display(), "[Int]");
    }

    #[test]
    fn display_array() {
        assert_eq!(Ty::Array(Box::new(Ty::int())).display(), "{Int}");
    }

    #[test]
    fn display_dict() {
        assert_eq!(
            Ty::Dict(Box::new(Ty::text()), Box::new(Ty::int())).display(),
            "Dict::(Text, Int)"
        );
    }

    #[test]
    fn display_set() {
        assert_eq!(Ty::Set(Box::new(Ty::int())).display(), "Set::Int");
    }

    #[test]
    fn display_sender() {
        assert_eq!(Ty::Sender(Box::new(Ty::int())).display(), "Sender::Int");
    }

    #[test]
    fn display_receiver() {
        assert_eq!(Ty::Receiver(Box::new(Ty::int())).display(), "Receiver::Int");
    }

    #[test]
    fn display_receiver_factory() {
        assert_eq!(
            Ty::ReceiverFactory(Box::new(Ty::int())).display(),
            "ReceiverFactory::Int"
        );
    }

    #[test]
    fn display_byte() {
        assert_eq!(Ty::Byte.display(), "Byte");
    }

    #[test]
    fn display_bytes() {
        assert_eq!(Ty::Bytes.display(), "Bytes");
    }

    #[test]
    fn display_file() {
        assert_eq!(Ty::File.display(), "File");
    }

    #[test]
    fn display_socket() {
        assert_eq!(Ty::Socket.display(), "Socket");
    }

    #[test]
    fn display_nested_generic() {
        assert_eq!(
            Ty::Generic("Optional".into(), vec![Ty::List(Box::new(Ty::int()))]).display(),
            "Optional::[Int]"
        );
    }
}
