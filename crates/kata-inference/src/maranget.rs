//! Motor de usefulness (Maranget) para análise de exaustividade e redundância
//! de patterns aninhados.
//!
//! Baseado em: Luc Maranget (INRIA), "Warnings for Pattern Matching", JFP 2007.
//! Implementação de referência: `rustc_pattern_analysis/usefulness.rs`.
//!
//! ## Algoritmo
//!
//! - **Matriz de pattern-tuples**: linhas = braços/cláusulas, colunas =
//!   scrutinees/params. Cada célula é um `TypedPattern`.
//! - **Especialização por construtor**: para cada construtor `c` que aparece
//!   na primeira coluna, cria uma sub-matriz descartando linhas incompatíveis
//!   e abrindo os campos do construtor como novas colunas.
//! - **Constructor splitting para tipos infinitos**: construtores ausentes
//!   agrupados no bucket `Missing` — não enumera Int/Float/Text.
//! - **Usefulness**: pattern `q` é útil w.r.t. `p₁..pₙ` sss existe valor casado
//!   por `q` e por nenhum `pᵢ`.
//!   - Exaustividade: `_` (wildcard row) NÃO é útil → match exaustivo.
//!     Witness do `_` = caso faltante.
//!   - Redundância: braço inútil = nenhum witness (unreachable).
//!
//! ## Composição com Z3
//!
//! O motor conduz estruturalmente; Z3 nunca enxerga estrutura de datatype.
//! Quando só resta decidir por guards, emite query Z3 escopada
//! por célula. `Unknown` → `MissingOtherwise` local à folha.

use kata_core::caps::{ConstVal, Repr};
use kata_core::struct_registry::StructRegistry;
use kata_core::ty::Ty;
use kata_core::{CapsIndex, EnumRegistry};
use kata_resolution::RefinedDeclInfo;

use crate::typed_pattern::TypedLambdaClause;
use crate::typed_pattern::TypedPattern;

// ── Trait de ambiente ─────────────────────────────────────────────

/// Ambiente de tipos — o motor não alcança `TypeEnv` diretamente.
///
/// Fornece construtores de um tipo e tipos de campos de construtores,
/// sem acoplar o motor ao sistema de tipos completo.
pub(crate) trait PatternEnv {
    /// Lista os construtores de um tipo (nomes de variantes para enums,
    /// `["Cons", "Nil"]` para List, `[]` para tipos infinitos/átomos).
    fn constructors_of(&self, ty: &Ty) -> Vec<Constructor>;

    /// Tipo do campo (payload) de um construtor. `None` = unitária.
    /// Para `Tuple`, retorna os tipos dos elementos (multiplexado via
    /// `Constructor::Tuple` com `field_tys`).
    fn field_tys(&self, ctor: &Constructor, ty: &Ty) -> Vec<Ty>;

    /// Se o tipo é infinito (Int, Float, Text, Byte, etc.).
    fn is_infinite(&self, ty: &Ty) -> bool;
}

/// Construtor de pattern — abstração sobre variantes de enum, Cons/Nil,
/// Literal, Tuple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Constructor {
    /// Variante de enum: `Some`, `None`, `Ok`, `Err`, `True`, `False`.
    Variant { enum_name: String, name: String },
    /// Cons (cabeça:cauda) de List.
    Cons,
    /// Nil (lista vazia) de List.
    Nil,
    /// Literal exato (Int, Float, Text, Unit). Para Boolean::True/False,
    /// usa `Variant` (são enums, não literais na TAST).
    Literal(String),
    /// Tupla de N elementos. `field_tys` carrega os tipos dos elementos.
    Tuple { arity: usize },
    /// Bucket "resto" — construtores ausentes de tipo infinito.
    /// Marcanget chama de `Missing`. Não é um construtor real; é o wildcard
    /// para valores não enumerados (ex: Int que não aparece como literal).
    Missing,
}

// ── Substituição de type params ────────────────────────────────────

/// Substitui `Ty::Var(name)` pelo tipo concreto correspondente em `type_args`.
///
/// `type_params` é a lista de nomes de parâmetros na ordem de declaração do
/// enum (ex: `["T", "E"]` para `Result`). `type_args` são os argumentos
/// concretos na mesma ordem (ex: `[Int, Text]` para `Result::(Int, Text)`).
fn substitute_ty(ty: &Ty, type_args: &[Ty], type_params: &[String]) -> Ty {
    match ty {
        Ty::Var(name) => {
            let idx = type_params.iter().position(|p| p == name);
            match idx {
                Some(i) if i < type_args.len() => type_args[i].clone(),
                _ => ty.clone(),
            }
        }
        Ty::Generic(name, args) => Ty::Generic(
            name.clone(),
            args.iter()
                .map(|a| substitute_ty(a, type_args, type_params))
                .collect(),
        ),
        Ty::List(inner) => Ty::List(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| substitute_ty(e, type_args, type_params))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params
                .iter()
                .map(|p| substitute_ty(p, type_args, type_params))
                .collect(),
            Box::new(substitute_ty(ret, type_args, type_params)),
        ),
        Ty::Action(params, ret) => Ty::Action(
            params
                .iter()
                .map(|p| substitute_ty(p, type_args, type_params))
                .collect(),
            Box::new(substitute_ty(ret, type_args, type_params)),
        ),
        Ty::Dict(k, v) => Ty::Dict(
            Box::new(substitute_ty(k, type_args, type_params)),
            Box::new(substitute_ty(v, type_args, type_params)),
        ),
        Ty::Set(inner) => Ty::Set(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::Array(inner) => Ty::Array(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::Range(inner) => Ty::Range(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::Sender(inner) => Ty::Sender(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::Receiver(inner) => Ty::Receiver(Box::new(substitute_ty(inner, type_args, type_params))),
        Ty::ReceiverFactory(inner) => {
            Ty::ReceiverFactory(Box::new(substitute_ty(inner, type_args, type_params)))
        }
        other => other.clone(),
    }
}

// ── Implementação concreta do PatternEnv ───────────────────────────

/// Implementação de `PatternEnv` usando `EnumRegistry`.
pub(crate) struct MarangetEnv<'a> {
    enum_registry: &'a EnumRegistry,
    struct_registry: Option<&'a StructRegistry>,
    refined_decls: Option<&'a [RefinedDeclInfo]>,
    /// Índice de capacidades de tipos (ORD, EQ, NUM, Repr).
    /// Derivado de `TypeGraph` + `StructRegistry` via `CapsIndex`.
    /// `None` em testes unitários do motor puro.
    caps_index: Option<&'a CapsIndex>,
}

impl<'a> MarangetEnv<'a> {
    pub(crate) fn new(enum_registry: &'a EnumRegistry) -> Self {
        Self {
            enum_registry,
            struct_registry: None,
            refined_decls: None,
            caps_index: None,
        }
    }

    pub(crate) fn with_refined(
        enum_registry: &'a EnumRegistry,
        struct_registry: &'a StructRegistry,
        refined_decls: &'a [RefinedDeclInfo],
        caps_index: &'a CapsIndex,
    ) -> Self {
        Self {
            enum_registry,
            struct_registry: Some(struct_registry),
            refined_decls: Some(refined_decls),
            caps_index: Some(caps_index),
        }
    }

    /// Tenta enumerar o domínio finito de um tipo refined.
    ///
    /// Delega a `refined_domain::enum_refined_domain` (fronteira extraída).
    fn enum_refined_domain(&self, name: &str) -> Option<Vec<ConstVal>> {
        crate::refined_domain::enum_refined_domain(
            self.struct_registry,
            self.refined_decls,
            self.caps_index,
            name,
        )
    }
}

impl<'a> PatternEnv for MarangetEnv<'a> {
    fn constructors_of(&self, ty: &Ty) -> Vec<Constructor> {
        match ty {
            Ty::Sum(enum_name) | Ty::Generic(enum_name, _) => {
                let variants = self.enum_registry.variants_of(enum_name);
                if variants.is_empty() {
                    // Enum desconhecido — trata como infinito.
                    return vec![Constructor::Missing];
                }
                variants
                    .iter()
                    .map(|v| Constructor::Variant {
                        enum_name: enum_name.to_string(),
                        name: v.to_string(),
                    })
                    .collect()
            }
            Ty::List(_) => vec![Constructor::Cons, Constructor::Nil],
            Ty::Tuple(elem_tys) => {
                vec![Constructor::Tuple {
                    arity: elem_tys.len(),
                }]
            }
            // Refined com domínio finito enumerável (Int com predicados
            // const-avaliáveis que definem intervalo fechado). Enumera os
            // literais do domínio como construtores.
            Ty::Struct(key) if self.struct_registry.is_some() && self.refined_decls.is_some() => {
                if let Some(domain) = self.enum_refined_domain(key.name()) {
                    // Domínio finito — literais como construtores.
                    // Se o domínio é vazio, retorna Missing (não deveria
                    // acontecer para refined válido).
                    if domain.is_empty() {
                        return vec![Constructor::Missing];
                    }
                    // Serializa cada ConstVal via to_ctor_string(repr).
                    // Repr vem do CapsIndex; sem caps_index, fallback "Int:N".
                    let repr = self
                        .caps_index
                        .map(|ci| ci.get(key.name()).repr.clone())
                        .unwrap_or(Repr::Int);
                    return domain
                        .iter()
                        .map(|v| Constructor::Literal(v.to_ctor_string(&repr)))
                        .collect();
                }
                // Refined sem domínio enumerável — trata como infinito.
                vec![Constructor::Missing]
            }
            // Tipos infinitos, átomos, structs — o construtor
            // é determinado pelos patterns que aparecem, não pelo tipo.
            // `Missing` cobre os ausentes.
            _ => vec![Constructor::Missing],
        }
    }

    fn field_tys(&self, ctor: &Constructor, ty: &Ty) -> Vec<Ty> {
        match (ctor, ty) {
            (Constructor::Variant { enum_name, name }, ty) => {
                // Busca o payload_ty da variante no EnumRegistry.
                if let Some(variants) = self.enum_registry.all_variants(enum_name)
                    && let Some(info) = variants.iter().find(|v| v.name == *name)
                    && let Some(payload) = &info.payload_ty
                {
                    // Substitui type params pelo tipo concreto.
                    // Para Ty::Generic("Optional", [Boolean]),
                    // substitui T -> Boolean.
                    let type_args: Vec<Ty> = match ty {
                        Ty::Generic(_, args) => args.clone(),
                        _ => Vec::new(),
                    };
                    let type_params = self.enum_registry.type_params_of(enum_name).unwrap_or(&[]);
                    return vec![substitute_ty(payload, &type_args, type_params)];
                }
                Vec::new() // unitária ou não encontrada
            }
            (Constructor::Cons, Ty::List(elem_ty)) => {
                vec![elem_ty.as_ref().clone(), elem_ty.as_ref().clone()]
            }
            (Constructor::Nil, Ty::List(_)) => Vec::new(),
            (Constructor::Tuple { arity }, Ty::Tuple(elem_tys)) => {
                elem_tys.iter().take(*arity).cloned().collect()
            }
            _ => Vec::new(),
        }
    }

    fn is_infinite(&self, ty: &Ty) -> bool {
        // Refined com domínio finito enumerável NÃO é infinito.
        if let Ty::Struct(key) = ty
            && self.struct_registry.is_some()
            && self.refined_decls.is_some()
            && self.enum_refined_domain(key.name()).is_some()
        {
            return false;
        }
        matches!(
            ty,
            Ty::Prim(_)
                | Ty::Unit
                | Ty::Byte
                | Ty::Bytes
                | Ty::File
                | Ty::Socket
                | Ty::Dict(_, _)
                | Ty::Set(_)
                | Ty::Sender(_)
                | Ty::Receiver(_)
                | Ty::ReceiverFactory(_)
                | Ty::Range(_)
                | Ty::Array(_)
                | Ty::Function(_, _)
                | Ty::Action(_, _)
                | Ty::Interface(_)
                | Ty::OverloadSet { .. }
        )
    }
}

// ── Extrair construtor de um pattern ───────────────────────────────

/// Retorna o construtor de um pattern, ou `None` se for wildcard/ident
/// (cobre qualquer construtor).
fn pattern_ctor(pattern: &TypedPattern) -> Option<Constructor> {
    match pattern {
        TypedPattern::Variant {
            enum_name, variant, ..
        } => Some(Constructor::Variant {
            enum_name: enum_name.clone(),
            name: variant.clone(),
        }),
        TypedPattern::Cons { .. } => Some(Constructor::Cons),
        TypedPattern::Nil => Some(Constructor::Nil),
        TypedPattern::Literal { value } => {
            // Serializa o literal para string — comparação estrutural.
            Some(Constructor::Literal(literal_to_string(&value.node)))
        }
        TypedPattern::Tuple { elements } => Some(Constructor::Tuple {
            arity: elements.len(),
        }),
        TypedPattern::Ident { .. } | TypedPattern::Wildcard => None,
    }
}

/// Serializa um TypedExpr literal para string comparável.
fn literal_to_string(expr: &crate::typed::TypedExpr) -> String {
    use crate::typed::TypedExprKind;
    match &expr.kind {
        TypedExprKind::IntLit { text } => format!("Int:{}", text),
        TypedExprKind::FloatLit { text } => format!("Float:{}", text),
        TypedExprKind::TextLit { text } => format!("Text:{}", text),
        TypedExprKind::Unit => "Unit".to_string(),
        // `rational N` → Rat:N|1 (round-trip com ConstVal::to_ctor_string).
        TypedExprKind::Closure { callee, args, .. } => {
            if let TypedExprKind::Ident { name } = &callee.node.kind
                && name == "rational"
                && args.len() == 1
                && let TypedExprKind::IntLit { text } = &args[0].node.kind
            {
                return format!("Rat:{}|1", text);
            }
            format!("Other:{:?}", expr.ty)
        }
        _ => format!("Other:{:?}", expr.ty),
    }
}

// ── Extrair sub-patterns de um construtor ──────────────────────────

/// Retorna os sub-patterns de um pattern para um dado construtor.
/// Para `Variant{Some, [x]}` com construtor `Variant{Some}` → `[x]`.
/// Para `Cons{h, t}` com construtor `Cons` → `[h, t]`.
/// Para `Wildcard`/`Ident` → wildcard expandido para o número de campos.
fn expand_pattern(
    pattern: &TypedPattern,
    _ctor: &Constructor,
    n_fields: usize,
) -> Vec<TypedPattern> {
    match pattern {
        TypedPattern::Ident { .. } | TypedPattern::Wildcard => {
            // Curinga: expande para N wildcards.
            (0..n_fields).map(|_| TypedPattern::Wildcard).collect()
        }
        TypedPattern::Variant {
            sub_patterns: Some(subs),
            ..
        } => {
            // Construtor casou — sub-patterns do payload.
            subs.iter().map(|s| s.node.clone()).collect()
        }
        TypedPattern::Variant {
            sub_patterns: None, ..
        } => {
            // Variante unitária — sem sub-patterns.
            Vec::new()
        }
        TypedPattern::Cons { head, tail } => {
            vec![head.node.clone(), tail.node.clone()]
        }
        TypedPattern::Nil => Vec::new(),
        TypedPattern::Tuple { elements } => elements.iter().map(|e| e.node.clone()).collect(),
        TypedPattern::Literal { .. } => Vec::new(),
    }
}

// ── Witness ─────────────────────────────────────────────────────────

/// Witness de não-exaustividade — um pattern-tuple que case um valor
/// não coberto pelos braços.
#[derive(Debug, Clone)]
pub(crate) struct Witness {
    /// Patterns do witness (ex: `["Some (Some False)"]`).
    pub patterns: Vec<String>,
}

// ── Matriz de patterns ──────────────────────────────────────────────

/// Uma linha da matriz de patterns — representa um braço/cláusula.
#[derive(Debug, Clone)]
struct MatrixRow {
    /// Patterns da linha (uma por coluna).
    patterns: Vec<TypedPattern>,
    /// Índice do braço/cláusula original (para redundância).
    /// `None` para a linha wildcard `_` usada no teste de exaustividade.
    arm_index: Option<usize>,
}

/// Matriz de pattern-tuples.
struct PatternMatrix {
    rows: Vec<MatrixRow>,
    /// Tipos das colunas (scrutinee types).
    column_tys: Vec<Ty>,
}

impl PatternMatrix {
    fn new(column_tys: Vec<Ty>) -> Self {
        Self {
            rows: Vec::new(),
            column_tys,
        }
    }

    fn add_row(&mut self, patterns: Vec<TypedPattern>, arm_index: Option<usize>) {
        self.rows.push(MatrixRow {
            patterns,
            arm_index,
        });
    }
}

// ── Algoritmo de usefulness ─────────────────────────────────────────

/// Coleta TODAS as witnesses de não-exaustividade — cada pattern-tuple que
/// casa um valor não coberto pelas linhas da matriz.
///
/// Diferente de `is_useful` (que para na primeira witness), esta função
/// itera sobre TODOS os construtores em `constructors_to_try` e acumula
/// as witnesses de cada um. Usada para exaustividade (reportar todos os
/// casos faltantes); `is_useful` é usada para redundância (basta uma).
fn collect_all_witnesses(
    matrix: &PatternMatrix,
    q: &MatrixRow,
    env: &dyn PatternEnv,
) -> Vec<Witness> {
    if matrix.column_tys.is_empty() {
        if matrix.rows.is_empty() {
            return vec![Witness {
                patterns: Vec::new(),
            }];
        }
        return Vec::new();
    }

    let head_ty = &matrix.column_tys[0].clone();

    let mut ctors_seen: Vec<Constructor> = Vec::new();
    for row in &matrix.rows {
        if let Some(ctor) = pattern_ctor(&row.patterns[0])
            && !ctors_seen.contains(&ctor)
        {
            ctors_seen.push(ctor);
        }
    }
    if let Some(ctor) = pattern_ctor(&q.patterns[0])
        && !ctors_seen.contains(&ctor)
    {
        ctors_seen.push(ctor);
    }

    let type_ctors = env.constructors_of(head_ty);

    let present_ctors: Vec<Constructor> = ctors_seen.clone();
    let missing_ctors: Vec<Constructor> = type_ctors
        .iter()
        .filter(|c| !present_ctors.contains(c))
        .cloned()
        .collect();

    let mut constructors_to_try: Vec<Constructor> = present_ctors.clone();
    if env.is_infinite(head_ty) {
        if !missing_ctors.is_empty() || constructors_to_try.is_empty() {
            constructors_to_try.push(Constructor::Missing);
        }
    } else {
        constructors_to_try.extend(missing_ctors);
    }

    if constructors_to_try.is_empty() {
        if matrix.rows.is_empty() {
            return vec![Witness {
                patterns: vec!["_".to_string()],
            }];
        }
        return Vec::new();
    }

    let mut all_witnesses: Vec<Witness> = Vec::new();

    for ctor in &constructors_to_try {
        let field_tys = env.field_tys(ctor, head_ty);
        let n_fields = field_tys.len();

        let mut sub_tys: Vec<Ty> = field_tys;
        sub_tys.extend(matrix.column_tys[1..].iter().cloned());

        let mut sub_matrix = PatternMatrix::new(sub_tys);

        for row in &matrix.rows {
            let row_ctor = pattern_ctor(&row.patterns[0]);
            match &row_ctor {
                Some(rc) if rc == ctor => {
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                None => {
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                _ => {}
            }
        }

        let q_ctor = pattern_ctor(&q.patterns[0]);
        let sub_q = match &q_ctor {
            Some(qc) if qc == ctor => {
                let expanded = expand_pattern(&q.patterns[0], ctor, n_fields);
                let mut new_patterns = expanded;
                new_patterns.extend(q.patterns[1..].iter().cloned());
                MatrixRow {
                    patterns: new_patterns,
                    arm_index: q.arm_index,
                }
            }
            None => {
                let expanded = expand_pattern(&q.patterns[0], ctor, n_fields);
                let mut new_patterns = expanded;
                new_patterns.extend(q.patterns[1..].iter().cloned());
                MatrixRow {
                    patterns: new_patterns,
                    arm_index: q.arm_index,
                }
            }
            _ => continue,
        };

        let sub_witnesses = collect_all_witnesses(&sub_matrix, &sub_q, env);
        for mut witness in sub_witnesses {
            let prefix = if matches!(ctor, Constructor::Missing) {
                "_".to_string()
            } else {
                witness_prefix_string(ctor, &witness.patterns)
            };
            let n_fields = env.field_tys(ctor, head_ty).len();
            let remaining: Vec<String> = witness.patterns.drain(n_fields..).collect();
            let mut result = vec![prefix];
            result.extend(remaining);
            all_witnesses.push(Witness { patterns: result });
        }
    }

    all_witnesses
}

/// Folha coberta — um valor que pelo menos um braço casa.
/// `arm_indices` são os índices dos braços que casam este valor.
#[derive(Debug, Clone)]
struct GuardLeaf {
    arm_indices: Vec<usize>,
}

/// Percorre a árvore de especialização e coleta folhas cobertas.
///
/// Paralelo a `collect_all_witnesses`, mas em vez de coletar witnesses de
/// valores faltantes, coleta os índices dos braços que cobrem cada folha.
/// Usada na verificação de guards: para cada folha coberta por braços com
/// guards, a camada chamadora emite query Z3 para verificar se a disjunção
/// dos guards é tautologia.
///
/// Diferença de `collect_all_witnesses`: não usa linha wildcard `q` —
/// percorre a árvore apenas com os construtores do tipo e das linhas.
fn collect_guard_leaves(matrix: &PatternMatrix, env: &dyn PatternEnv) -> Vec<GuardLeaf> {
    if matrix.column_tys.is_empty() {
        // Caso base: 0 colunas. Toda linha restante casa o mesmo valor.
        if matrix.rows.is_empty() {
            // Folha não coberta — não é uma GuardLeaf.
            return Vec::new();
        }
        // Folha coberta — coleta arm_indices das linhas.
        let arm_indices: Vec<usize> = matrix.rows.iter().filter_map(|r| r.arm_index).collect();
        return vec![GuardLeaf { arm_indices }];
    }

    let head_ty = &matrix.column_tys[0].clone();

    // Coleta construtores que aparecem na primeira coluna.
    let mut ctors_seen: Vec<Constructor> = Vec::new();
    for row in &matrix.rows {
        if let Some(ctor) = pattern_ctor(&row.patterns[0])
            && !ctors_seen.contains(&ctor)
        {
            ctors_seen.push(ctor);
        }
    }

    let type_ctors = env.constructors_of(head_ty);

    let present_ctors: Vec<Constructor> = ctors_seen.clone();
    let missing_ctors: Vec<Constructor> = type_ctors
        .iter()
        .filter(|c| !present_ctors.contains(c))
        .cloned()
        .collect();

    let mut constructors_to_try: Vec<Constructor> = present_ctors.clone();
    if env.is_infinite(head_ty) {
        if !missing_ctors.is_empty() || constructors_to_try.is_empty() {
            constructors_to_try.push(Constructor::Missing);
        }
    } else {
        constructors_to_try.extend(missing_ctors);
    }

    if constructors_to_try.is_empty() {
        // Sem construtores para tentar — se há linhas, é uma folha coberta.
        if !matrix.rows.is_empty() {
            let arm_indices: Vec<usize> = matrix.rows.iter().filter_map(|r| r.arm_index).collect();
            return vec![GuardLeaf { arm_indices }];
        }
        return Vec::new();
    }

    let mut all_leaves: Vec<GuardLeaf> = Vec::new();

    for ctor in &constructors_to_try {
        let field_tys = env.field_tys(ctor, head_ty);
        let n_fields = field_tys.len();

        let mut sub_tys: Vec<Ty> = field_tys;
        sub_tys.extend(matrix.column_tys[1..].iter().cloned());

        let mut sub_matrix = PatternMatrix::new(sub_tys);

        for row in &matrix.rows {
            let row_ctor = pattern_ctor(&row.patterns[0]);
            match &row_ctor {
                Some(rc) if rc == ctor => {
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                None => {
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                _ => {}
            }
        }

        all_leaves.extend(collect_guard_leaves(&sub_matrix, env));
    }

    all_leaves
}

/// Verifica se `q` é útil w.r.t. as linhas da matriz.
///
/// `q` é uma linha com `arm_index: None` (linha wildcard para exaustividade)
/// ou `Some(i)` (braço i para redundância).
///
/// Retorna `Some(witness)` se útil (witness = pattern-tuple que case um
/// valor não coberto), `None` se não útil.
fn is_useful(matrix: &PatternMatrix, q: &MatrixRow, env: &dyn PatternEnv) -> Option<Witness> {
    if matrix.column_tys.is_empty() {
        // Caso base: 0 colunas. Se há linhas na matriz, nenhuma é casável
        // (0 colunas = tudo casa), então q não é útil.
        // Se a matriz está vazia, q é útil com witness vazio.
        if matrix.rows.is_empty() {
            return Some(Witness {
                patterns: Vec::new(),
            });
        }
        return None;
    }

    // Coluna 0 é a cabeça.
    let head_ty = &matrix.column_tys[0].clone();

    // Coleta construtores que aparecem na primeira coluna.
    let mut ctors_seen: Vec<Constructor> = Vec::new();
    for row in &matrix.rows {
        if let Some(ctor) = pattern_ctor(&row.patterns[0])
            && !ctors_seen.contains(&ctor)
        {
            ctors_seen.push(ctor);
        }
    }
    // Também do pattern q (a linha que estamos testando).
    if let Some(ctor) = pattern_ctor(&q.patterns[0])
        && !ctors_seen.contains(&ctor)
    {
        ctors_seen.push(ctor);
    }

    // Construtores do tipo (universo).
    let type_ctors = env.constructors_of(head_ty);

    // ── Constructor splitting ──
    let present_ctors: Vec<Constructor> = ctors_seen.clone();
    let missing_ctors: Vec<Constructor> = type_ctors
        .iter()
        .filter(|c| !present_ctors.contains(c))
        .cloned()
        .collect();

    let mut constructors_to_try: Vec<Constructor> = present_ctors.clone();
    if env.is_infinite(head_ty) {
        if !missing_ctors.is_empty() || constructors_to_try.is_empty() {
            constructors_to_try.push(Constructor::Missing);
        }
    } else {
        constructors_to_try.extend(missing_ctors);
    }

    // Se não há construtores para tentar (tipo vazio sem construtores),
    // a linha q é útil se a matriz está vazia.
    if constructors_to_try.is_empty() {
        if matrix.rows.is_empty() {
            return Some(Witness {
                patterns: vec!["_".to_string()],
            });
        }
        return None;
    }

    // Para cada construtor, especializa a matriz e recursa.
    for ctor in &constructors_to_try {
        let n_fields = env.field_tys(ctor, head_ty).len();

        // Cria a sub-matriz especializada.
        let mut sub_tys: Vec<Ty> = Vec::new();
        for ft in env.field_tys(ctor, head_ty) {
            sub_tys.push(ft);
        }
        // As colunas restantes (1..N) vêm depois dos campos.
        sub_tys.extend(matrix.column_tys[1..].iter().cloned());

        let mut sub_matrix = PatternMatrix::new(sub_tys);

        for row in &matrix.rows {
            let row_ctor = pattern_ctor(&row.patterns[0]);
            match &row_ctor {
                Some(rc) if rc == ctor => {
                    // Linha com o mesmo construtor — expande.
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                None => {
                    // Wildcard/Ident — expande para campos do construtor.
                    let expanded = expand_pattern(&row.patterns[0], ctor, n_fields);
                    let mut new_patterns = expanded;
                    new_patterns.extend(row.patterns[1..].iter().cloned());
                    sub_matrix.add_row(new_patterns, row.arm_index);
                }
                _ => {
                    // Construtor diferente — descarta a linha.
                }
            }
        }

        // Especializa q.
        let q_ctor = pattern_ctor(&q.patterns[0]);
        let sub_q = match &q_ctor {
            Some(qc) if qc == ctor => {
                let expanded = expand_pattern(&q.patterns[0], ctor, n_fields);
                let mut new_patterns = expanded;
                new_patterns.extend(q.patterns[1..].iter().cloned());
                MatrixRow {
                    patterns: new_patterns,
                    arm_index: q.arm_index,
                }
            }
            None => {
                let expanded = expand_pattern(&q.patterns[0], ctor, n_fields);
                let mut new_patterns = expanded;
                new_patterns.extend(q.patterns[1..].iter().cloned());
                MatrixRow {
                    patterns: new_patterns,
                    arm_index: q.arm_index,
                }
            }
            _ => {
                // q não tem este construtor — não é útil para este ctor.
                // Pula para o próximo construtor.
                continue;
            }
        };

        if let Some(mut witness) = is_useful(&sub_matrix, &sub_q, env) {
            // Reconstrói o witness: prefixa com o construtor.
            let prefix = if matches!(ctor, Constructor::Missing) {
                "_".to_string()
            } else {
                witness_prefix_string(ctor, &witness.patterns)
            };
            // Remove os campos do witness (já no prefix) e adiciona o prefix.
            let n_fields = env.field_tys(ctor, head_ty).len();
            let remaining: Vec<String> = witness.patterns.drain(n_fields..).collect();
            let mut result = vec![prefix];
            result.extend(remaining);
            return Some(Witness { patterns: result });
        }
    }

    None
}

/// Constrói a string do prefix do witness para um construtor.
fn witness_prefix_string(ctor: &Constructor, field_witnesses: &[String]) -> String {
    match ctor {
        Constructor::Variant { name, .. } => {
            if field_witnesses.is_empty() {
                name.clone()
            } else {
                format!("{} ({})", name, field_witnesses.join(" "))
            }
        }
        Constructor::Cons => {
            if field_witnesses.len() >= 2 {
                format!("[{} : {}]", field_witnesses[0], field_witnesses[1])
            } else {
                "[_ : _]".to_string()
            }
        }
        Constructor::Nil => "[]".to_string(),
        Constructor::Literal(s) => s.clone(),
        Constructor::Tuple { .. } => {
            format!("({})", field_witnesses.join(", "))
        }
        Constructor::Missing => "_".to_string(),
    }
}

// ── API pública do motor ─────────────────────────────────────────────

/// Resultado da análise de exaustividade.
#[derive(Debug, Clone)]
pub(crate) struct ExhaustivenessResult {
    /// Se o match é exaustivo.
    pub exhaustive: bool,
    /// Witnesses dos casos faltantes (vazio se exaustivo).
    pub missing: Vec<String>,
}

/// Verifica exaustividade de um conjunto de braços/cláusulas.
///
/// `patterns_per_arm`: patterns de cada braço (já tipados). Para match
/// de 1 scrutinee, cada braço tem 1 pattern. Para lambda de N params,
/// cada cláusula tem N patterns.
///
/// `column_tys`: tipos dos scrutinees/params (1 por coluna).
///
/// `has_otherwise`: se algum braço é `otherwise` (pattern None) ou
/// `Wildcard`. Esses cobrem qualquer valor.
pub(crate) fn check_exhaustiveness_maranget(
    patterns_per_arm: &[Vec<TypedPattern>],
    column_tys: &[Ty],
    has_otherwise: bool,
    enum_registry: &EnumRegistry,
    struct_registry: Option<&StructRegistry>,
    refined_decls: Option<&[RefinedDeclInfo]>,
    caps_index: Option<&CapsIndex>,
) -> ExhaustivenessResult {
    if has_otherwise {
        return ExhaustivenessResult {
            exhaustive: true,
            missing: Vec::new(),
        };
    }

    if column_tys.is_empty() || patterns_per_arm.is_empty() {
        return ExhaustivenessResult {
            exhaustive: true,
            missing: Vec::new(),
        };
    }

    let env = match (struct_registry, refined_decls, caps_index) {
        (Some(sr), Some(rd), Some(ci)) => MarangetEnv::with_refined(enum_registry, sr, rd, ci),
        _ => MarangetEnv::new(enum_registry),
    };

    // Constrói a matriz.
    let mut matrix = PatternMatrix::new(column_tys.to_vec());
    for (i, arm_patterns) in patterns_per_arm.iter().enumerate() {
        matrix.add_row(arm_patterns.clone(), Some(i));
    }

    // Linha wildcard `_` para testar exaustividade.
    let wildcard_row = MatrixRow {
        patterns: (0..column_tys.len())
            .map(|_| TypedPattern::Wildcard)
            .collect(),
        arm_index: None,
    };

    match collect_all_witnesses(&matrix, &wildcard_row, &env) {
        witnesses if witnesses.is_empty() => ExhaustivenessResult {
            exhaustive: true,
            missing: Vec::new(),
        },
        witnesses => {
            let missing_str: Vec<String> = witnesses
                .into_iter()
                .map(|w| {
                    if w.patterns.len() == 1 {
                        w.patterns[0].clone()
                    } else {
                        format!("({})", w.patterns.join(", "))
                    }
                })
                .collect();
            ExhaustivenessResult {
                exhaustive: false,
                missing: missing_str,
            }
        }
    }
}

/// Resultado da análise de exaustividade com guards.
///
/// Combina o motor Maranget (estrutural) com Z3 (guards na folha).
/// O motor conduz até a folha; quando só resta decidir por guards,
/// emite query Z3 escopada por célula com bindings semeados.
///
/// `clauses`: cláusulas lambda tipadas (com patterns, guards, with_bindings).
/// `param_types`: tipos dos parâmetros (1 por coluna).
/// `has_otherwise`: se alguma cláusula tem pattern Ident/Wildcard.
/// `span`: span para mensagens de erro.
///
/// Fluxo:
/// 1. Roda `check_exhaustiveness_maranget` (estrutural, sem guards).
/// 2. Se não-exaustivo estruturalmente → `NonExhaustiveMatch` com witnesses.
/// 3. Se exaustivo estruturalmente e sem guards → Ok (verde estrutural).
/// 4. Se exaustivo estruturalmente com guards → `collect_guard_leaves`
///    identifica quais braços casam cada folha. Para cada folha onde
///    todos os braços têm guards, `check_guard_coverage` prova que a
///    disjunção dos guards é tautologia.
///    - `Proven` → Ok
///    - `Refuted` → `NonExhaustiveMatch` com contra-exemplo
///    - `Unknown` → `MissingOtherwise` local à folha
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_exhaustiveness_with_guards(
    clauses: &[TypedLambdaClause],
    param_types: &[Ty],
    has_otherwise: bool,
    span: &kata_ast::Span,
    enum_registry: &EnumRegistry,
    struct_registry: Option<&StructRegistry>,
    refined_decls: Option<&[RefinedDeclInfo]>,
    caps_index: Option<&CapsIndex>,
    inline_fns: Option<&crate::infer::InlineFnTable>,
) -> Result<(), kata_diagnostics::MiddleError> {
    // Coleta patterns de cada cláusula.
    let arm_patterns: Vec<Vec<TypedPattern>> = clauses
        .iter()
        .map(|c| c.patterns.iter().map(|p| p.node.clone()).collect())
        .collect();

    // 1. Verificação estrutural (sem guards).
    let struct_result = check_exhaustiveness_maranget(
        &arm_patterns,
        param_types,
        has_otherwise,
        enum_registry,
        struct_registry,
        refined_decls,
        caps_index,
    );

    if !struct_result.exhaustive {
        return Err(kata_diagnostics::MiddleError::NonExhaustiveMatch {
            missing: struct_result.missing.clone(),
            span: (*span).into(),
            hint: Some(format!(
                "combinações faltantes: {}. \
                 Adicione cláusulas para cada uma ou use `otherwise:` como fallback",
                struct_result.missing.join(", ")
            )),
        });
    }

    // 2. Exaustivo estruturalmente. Verificar guards.
    // Se nenhuma cláusula tem guards, está verde.
    let any_has_guards = clauses.iter().any(|c| !c.guards.is_empty());
    if !any_has_guards {
        return Ok(());
    }

    // 3. Há cláusulas com guards. Percorrer folhas e verificar Z3.
    // Se has_otherwise é true, o motor Maranget retornou exhaustive: true
    // imediatamente (atalho wildcard). Mas o wildcard pode ter guards!
    // Preciamo percorrer a árvore sem o atalho para identificar as folhas.
    // Se has_otherwise é true e o braço wildcard não tem guards, toda folha
    // é coberta pelo wildcard → Ok. Se o wildcard tem guards, precisamos
    // verificar por folha.
    //
    // Simplificação: se has_otherwise é true e a cláusula wildcard não tem
    // guards (ou tem otherwise), está verde. Senão, percorre folhas.
    if has_otherwise {
        // A cláusula com Ident/Wildcard cobre todas as folhas estruturalmente.
        // Se ela não tem guards, está verde.
        let wildcard_has_guards = clauses.iter().any(|c| {
            !c.guards.is_empty()
                && c.patterns
                    .iter()
                    .any(|p| matches!(p.node, TypedPattern::Ident { .. } | TypedPattern::Wildcard))
        });
        if !wildcard_has_guards {
            return Ok(());
        }
        // Wildcard tem guards — cai para collect_guard_leaves.
        // (Caso raro: `lambda x: > x 0: ...` sem otherwise.)
    }

    // Constrói a matriz para collect_guard_leaves (sem atalho has_otherwise).
    let env = match (struct_registry, refined_decls, caps_index) {
        (Some(sr), Some(rd), Some(ci)) => MarangetEnv::with_refined(enum_registry, sr, rd, ci),
        _ => MarangetEnv::new(enum_registry),
    };
    let mut matrix = PatternMatrix::new(param_types.to_vec());
    for (i, arm_patterns) in arm_patterns.iter().enumerate() {
        matrix.add_row(arm_patterns.clone(), Some(i));
    }

    let leaves = collect_guard_leaves(&matrix, &env);

    for leaf in &leaves {
        // Coleta os braços que casam esta folha e têm guards.
        let guard_arms: Vec<crate::guard_completeness::GuardArm> = leaf
            .arm_indices
            .iter()
            .filter_map(|&idx| {
                let clause = &clauses[idx];
                if clause.guards.is_empty() {
                    // Braço sem guards — cobre a folha trivialmente.
                    // Se há um braço sem guards nesta folha, ela está coberta.
                    return None;
                }
                Some(crate::guard_completeness::GuardArm {
                    guards: &clause.guards,
                    with_bindings: &clause.with_bindings,
                    inline_fns,
                })
            })
            .collect();

        // Se algum braço sem guards está na folha, ela está coberta.
        let has_bare_arm = leaf
            .arm_indices
            .iter()
            .any(|&idx| clauses[idx].guards.is_empty());

        if has_bare_arm {
            continue;
        }

        // Todos os braços na folha têm guards. Verificar Z3.
        if guard_arms.is_empty() {
            // Folha sem braços?? Não deveria acontecer (GuardLeaf só é
            // criada quando há linhas). Defensivo: Ok.
            continue;
        }

        crate::guard_completeness::check_guard_coverage(&guard_arms, span)?;
    }

    Ok(())
}

/// Verifica se um braço é redundante (inútil) w.r.t. os braços anteriores.
///
/// Retorna `true` se o braço `arm_index` é redundante (nenhum valor casa
/// com ele que não case com um braço anterior).
pub(crate) fn is_arm_redundant(
    all_patterns: &[Vec<TypedPattern>],
    column_tys: &[Ty],
    arm_index: usize,
    enum_registry: &EnumRegistry,
) -> bool {
    if arm_index == 0 || column_tys.is_empty() {
        return false;
    }

    let env = MarangetEnv::new(enum_registry);

    // Matriz com apenas os braços ANTERIORES.
    let mut matrix = PatternMatrix::new(column_tys.to_vec());
    for (i, arm_patterns) in all_patterns.iter().enumerate() {
        if i >= arm_index {
            break;
        }
        matrix.add_row(arm_patterns.clone(), Some(i));
    }

    // Linha q = o braço que estamos testando.
    let q = MatrixRow {
        patterns: all_patterns[arm_index].clone(),
        arm_index: Some(arm_index),
    };

    is_useful(&matrix, &q, &env).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kata_core::EnumRegistry;
    use kata_core::ty::PrimTy;

    fn env_bool() -> EnumRegistry {
        let mut reg = EnumRegistry::new();
        reg.register(
            "core",
            "Boolean",
            vec![
                kata_core::VariantInfo {
                    name: "True".to_string(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
                kata_core::VariantInfo {
                    name: "False".to_string(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );
        reg
    }

    fn env_optional() -> EnumRegistry {
        let mut reg = env_bool();
        reg.register(
            "core",
            "Optional",
            vec![
                kata_core::VariantInfo {
                    name: "Some".to_string(),
                    payload_ty: Some(Ty::Prim(PrimTy::Int)),
                    predicate: None,
                    fixed_value: None,
                },
                kata_core::VariantInfo {
                    name: "None".to_string(),
                    payload_ty: None,
                    predicate: None,
                    fixed_value: None,
                },
            ],
        );
        reg
    }

    fn variant(enum_name: &str, name: &str) -> TypedPattern {
        TypedPattern::Variant {
            enum_name: enum_name.to_string(),
            variant: name.to_string(),
            sub_patterns: None,
            tag: 0,
        }
    }

    fn variant_with(enum_name: &str, name: &str, sub: TypedPattern) -> TypedPattern {
        TypedPattern::Variant {
            enum_name: enum_name.to_string(),
            variant: name.to_string(),
            sub_patterns: Some(vec![kata_ast::Spanned::new(sub, kata_ast::Span::zero())]),
            tag: 0,
        }
    }

    fn wildcard() -> TypedPattern {
        TypedPattern::Wildcard
    }

    fn int_literal(text: &str) -> TypedPattern {
        TypedPattern::Literal {
            value: kata_ast::Spanned::new(
                crate::typed::TypedExpr {
                    span: kata_ast::Span::zero(),
                    ty: Ty::Prim(PrimTy::Int),
                    tail_pos: false,
                    escape: kata_core::EscapeTarget::Local,
                    kind: crate::typed::TypedExprKind::IntLit {
                        text: text.to_string(),
                    },
                },
                kata_ast::Span::zero(),
            ),
        }
    }

    #[test]
    fn test_bool_exhaustive() {
        let reg = env_bool();
        let patterns = vec![
            vec![variant("Boolean", "True")],
            vec![variant("Boolean", "False")],
        ];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Sum("Boolean".to_string())],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(result.exhaustive, "True + False should be exhaustive");
    }

    #[test]
    fn test_bool_not_exhaustive() {
        let reg = env_bool();
        let patterns = vec![vec![variant("Boolean", "True")]];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Sum("Boolean".to_string())],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(!result.exhaustive, "Only True should not be exhaustive");
        assert_eq!(result.missing, vec!["False"]);
    }

    #[test]
    fn test_bool_with_wildcard_exhaustive() {
        let reg = env_bool();
        let patterns = vec![vec![variant("Boolean", "True")], vec![wildcard()]];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Sum("Boolean".to_string())],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(result.exhaustive, "True + _ should be exhaustive");
    }

    #[test]
    fn test_optional_exhaustive() {
        let reg = env_optional();
        let patterns = vec![
            vec![variant_with("Optional", "Some", wildcard())],
            vec![variant("Optional", "None")],
        ];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Generic("Optional".to_string(), vec![])],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(
            result.exhaustive,
            "Some _ + None should be exhaustive for Optional"
        );
    }

    #[test]
    fn test_optional_not_exhaustive_missing_some() {
        let reg = env_optional();
        // Only None — missing Some
        let patterns = vec![vec![variant("Optional", "None")]];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Generic("Optional".to_string(), vec![])],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(!result.exhaustive, "Only None should not be exhaustive");
        assert_eq!(result.missing, vec!["Some (_)"]);
    }

    #[test]
    fn test_redundant_arm() {
        let reg = env_bool();
        let patterns = vec![
            vec![variant("Boolean", "True")],
            vec![variant("Boolean", "False")],
            vec![variant("Boolean", "True")], // redundant
        ];
        let col = [Ty::Sum("Boolean".to_string())];
        assert!(!is_arm_redundant(&patterns, &col, 0, &reg));
        assert!(!is_arm_redundant(&patterns, &col, 1, &reg));
        assert!(is_arm_redundant(&patterns, &col, 2, &reg));
    }

    #[test]
    fn test_not_redundant_arm() {
        let reg = env_bool();
        let patterns = vec![
            vec![variant("Boolean", "True")],
            vec![variant("Boolean", "False")],
        ];
        let col = [Ty::Sum("Boolean".to_string())];
        assert!(!is_arm_redundant(&patterns, &col, 1, &reg));
    }

    #[test]
    fn test_int_requires_wildcard() {
        let reg = env_bool();
        let patterns = vec![vec![int_literal("0")]];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Prim(PrimTy::Int)],
            false,
            &reg,
            None,
            None,
            None,
        );
        assert!(
            !result.exhaustive,
            "Single literal on Int should not be exhaustive"
        );
    }

    #[test]
    fn test_int_with_wildcard_exhaustive() {
        let reg = env_bool();
        let patterns = vec![vec![int_literal("0")], vec![wildcard()]];
        let result = check_exhaustiveness_maranget(
            &patterns,
            &[Ty::Prim(PrimTy::Int)],
            true,
            &reg,
            None,
            None,
            None,
        );
        assert!(result.exhaustive, "literal + wildcard should be exhaustive");
    }
}
