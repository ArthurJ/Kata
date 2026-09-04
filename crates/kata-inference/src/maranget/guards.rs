//! Análise de guards — coleta de folhas cobertas pela árvore de especialização.
//!
//! Camada separada do motor Maranget: percorre a árvore de especialização
//! (paralelo a `collect_all_witnesses`) coletando os índices dos braços
//! que cobrem cada folha, em vez de witnesses de valores faltantes.

use kata_core::ty::Ty;

use super::{Constructor, PatternEnv, PatternMatrix, expand_pattern, pattern_ctor};

/// Folha coberta — um valor que pelo menos um braço casa.
/// `arm_indices` são os índices dos braços que casam este valor.
#[derive(Debug, Clone)]
pub(crate) struct GuardLeaf {
    pub(crate) arm_indices: Vec<usize>,
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
pub(crate) fn collect_guard_leaves(matrix: &PatternMatrix, env: &dyn PatternEnv) -> Vec<GuardLeaf> {
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
