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
//! Na Fase 3, quando só resta decidir por guards, emite query Z3 escopada
//! por célula. `Unknown` → `MissingOtherwise` local à folha.

use kata_core::EnumRegistry;
use kata_core::ty::Ty;

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

// ── Implementação concreta do PatternEnv ───────────────────────────

/// Implementação de `PatternEnv` usando `EnumRegistry`.
pub(crate) struct MarangetEnv<'a> {
    enum_registry: &'a EnumRegistry,
}

impl<'a> MarangetEnv<'a> {
    pub(crate) fn new(enum_registry: &'a EnumRegistry) -> Self {
        Self { enum_registry }
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
            // Tipos infinitos, átomos, tuplas, structs — o construtor
            // é determinado pelos patterns que aparecem, não pelo tipo.
            // `Missing` cobre os ausentes.
            _ => vec![Constructor::Missing],
        }
    }

    fn field_tys(&self, ctor: &Constructor, ty: &Ty) -> Vec<Ty> {
        match (ctor, ty) {
            (Constructor::Variant { enum_name, name }, _) => {
                // Busca o payload_ty da variante no EnumRegistry.
                if let Some(variants) = self.enum_registry.all_variants(enum_name) {
                    if let Some(info) = variants.iter().find(|v| v.name == *name) {
                        if let Some(payload) = &info.payload_ty {
                            return vec![payload.clone()];
                        }
                    }
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
    ctor: &Constructor,
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

/// Formata um pattern do witness como string legível.
fn pattern_witness_string(pattern: &TypedPattern) -> String {
    match pattern {
        TypedPattern::Wildcard => "_".to_string(),
        TypedPattern::Ident { .. } => "_".to_string(),
        TypedPattern::Variant {
            variant,
            sub_patterns,
            ..
        } => {
            if let Some(subs) = sub_patterns {
                if subs.is_empty() {
                    variant.clone()
                } else {
                    let inner: Vec<String> = subs
                        .iter()
                        .map(|s| pattern_witness_string(&s.node))
                        .collect();
                    format!("{} ({})", variant, inner.join(" "))
                }
            } else {
                variant.clone()
            }
        }
        TypedPattern::Cons { head, tail } => {
            format!(
                "[{} : {}]",
                pattern_witness_string(&head.node),
                pattern_witness_string(&tail.node)
            )
        }
        TypedPattern::Nil => "[]".to_string(),
        TypedPattern::Literal { value } => {
            use crate::typed::TypedExprKind;
            match &value.node.kind {
                TypedExprKind::IntLit { text } => text.clone(),
                TypedExprKind::FloatLit { text } => text.clone(),
                TypedExprKind::TextLit { text } => format!("\"{}\"", text),
                TypedExprKind::Unit => "()".to_string(),
                _ => "_".to_string(),
            }
        }
        TypedPattern::Tuple { elements } => {
            let inner: Vec<String> = elements
                .iter()
                .map(|e| pattern_witness_string(&e.node))
                .collect();
            format!("({})", inner.join(", "))
        }
    }
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

    /// Número de colunas (scrutinees).
    fn width(&self) -> usize {
        self.column_tys.len()
    }
}

// ── Algoritmo de usefulness ─────────────────────────────────────────

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
        if let Some(ctor) = pattern_ctor(&row.patterns[0]) {
            if !ctors_seen.contains(&ctor) {
                ctors_seen.push(ctor);
            }
        }
    }
    // Também do pattern q (a linha que estamos testando).
    if let Some(ctor) = pattern_ctor(&q.patterns[0]) {
        if !ctors_seen.contains(&ctor) {
            ctors_seen.push(ctor);
        }
    }

    // Construtores do tipo (universo).
    let type_ctors = env.constructors_of(head_ty);

    // Determina se há wildcard/ident na primeira coluna de alguma linha.
    let has_wildcard_head = matrix
        .rows
        .iter()
        .any(|r| pattern_ctor(&r.patterns[0]).is_none())
        || pattern_ctor(&q.patterns[0]).is_none();

    // ── Constructor splitting ──
    //
    // Se o tipo é finito (enum/list) e não há wildcard na primeira coluna,
    // e nem todos os construtores do tipo aparecem, precisamos especializar
    // para os presentes E para os ausentes (bucket Missing).
    //
    // Se há wildcard na primeira coluna, ou o tipo é infinito, ou todos
    // os construtores aparecem, especializamos apenas para os presentes
    // (mais o bucket Missing se infinito e não há wildcard cobrindo).

    let mut constructors_to_try: Vec<Constructor> = Vec::new();

    // Construtores que aparecem na matriz (e em q).
    let present_ctors: Vec<Constructor> = ctors_seen.clone();

    // Construtores do tipo que não aparecem na matriz.
    let missing_ctors: Vec<Constructor> = type_ctors
        .iter()
        .filter(|c| !present_ctors.contains(c))
        .cloned()
        .collect();

    if has_wildcard_head {
        // Há wildcard na cabeça — cobre todos os construtores.
        // Para tipos finitos, tentamos cada construtor ausente
        // individualmente (para nomeá-lo no witness).
        // Para tipos infinitos, usamos o bucket Missing.
        constructors_to_try = present_ctors.clone();
        if env.is_infinite(head_ty) {
            if !missing_ctors.is_empty() || constructors_to_try.is_empty() {
                constructors_to_try.push(Constructor::Missing);
            }
        } else {
            // Tipo finito — cada construtor ausente é tentado individualmente.
            constructors_to_try.extend(missing_ctors);
        }
    } else {
        // Sem wildcard na cabeça — construtores que aparecem mais os
        // ausentes. Para tipos finitos (enum/list), tentamos cada
        // construtor ausente individualmente (para nomeá-lo no witness).
        // Para tipos infinitos, agrupamos no bucket Missing.
        constructors_to_try = present_ctors.clone();
        if env.is_infinite(head_ty) {
            if !missing_ctors.is_empty() || constructors_to_try.is_empty() {
                constructors_to_try.push(Constructor::Missing);
            }
        } else {
            // Tipo finito — cada construtor ausente é tentado individualmente.
            constructors_to_try.extend(missing_ctors);
        }
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

    let env = MarangetEnv::new(enum_registry);

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

    match is_useful(&matrix, &wildcard_row, &env) {
        None => ExhaustivenessResult {
            exhaustive: true,
            missing: Vec::new(),
        },
        Some(witness) => {
            let missing_str = if witness.patterns.len() == 1 {
                witness.patterns.clone()
            } else {
                vec![format!("({})", witness.patterns.join(", "))]
            };
            ExhaustivenessResult {
                exhaustive: false,
                missing: missing_str,
            }
        }
    }
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

/// Verifica redundância de todos os braços. Retorna índices dos braços
/// redundantes.
pub(crate) fn find_redundant_arms(
    all_patterns: &[Vec<TypedPattern>],
    column_tys: &[Ty],
    enum_registry: &EnumRegistry,
) -> Vec<usize> {
    let mut redundant = Vec::new();
    for i in 0..all_patterns.len() {
        if is_arm_redundant(all_patterns, column_tys, i, enum_registry) {
            redundant.push(i);
        }
    }
    redundant
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
        let redundant = find_redundant_arms(&patterns, &[Ty::Sum("Boolean".to_string())], &reg);
        assert_eq!(redundant, vec![2]);
    }

    #[test]
    fn test_not_redundant_arm() {
        let reg = env_bool();
        let patterns = vec![
            vec![variant("Boolean", "True")],
            vec![variant("Boolean", "False")],
        ];
        let redundant = find_redundant_arms(&patterns, &[Ty::Sum("Boolean".to_string())], &reg);
        assert!(redundant.is_empty());
    }

    #[test]
    fn test_int_requires_wildcard() {
        let reg = env_bool();
        let patterns = vec![vec![int_literal("0")]];
        let result =
            check_exhaustiveness_maranget(&patterns, &[Ty::Prim(PrimTy::Int)], false, &reg);
        assert!(
            !result.exhaustive,
            "Single literal on Int should not be exhaustive"
        );
    }

    #[test]
    fn test_int_with_wildcard_exhaustive() {
        let reg = env_bool();
        let patterns = vec![vec![int_literal("0")], vec![wildcard()]];
        let result = check_exhaustiveness_maranget(&patterns, &[Ty::Prim(PrimTy::Int)], true, &reg);
        assert!(result.exhaustive, "literal + wildcard should be exhaustive");
    }
}
