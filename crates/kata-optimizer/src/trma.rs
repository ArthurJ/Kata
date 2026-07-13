//! TRMA — Tail Recursion Modulo Associativity.
//!
//! Detecta auto-recursão direta com operador associativo e reescreve
//! em recursão de cauda com acumulador.
//!
//! Exemplo:
//! ```text
//! soma n:
//!     match n
//!         0: 0
//!         otherwise: + n (soma (- n 1))
//! ```
//! Transforma em:
//! ```text
//! soma n:
//!     soma_acc n 0
//!
//! soma_acc n acc:
//!     match n
//!         0: acc
//!         otherwise: soma_acc (- n 1) (+ acc n)
//! ```
//!
//! O `+` (associative_neutral = 0) e `*` (associative_neutral = 1) são
//! os operadores associativos suportados. `-` e `/` não são associativos.

use kata_ast::{Span, Spanned};
use kata_core::dispatch::DispatchTable;
use kata_core::ty::Ty;
use kata_inference::{
    Effect, TypedExpr, TypedExprKind, TypedFunction, TypedLambdaClause, TypedMatchArm,
    TypedModule, TypedPattern,
};

/// Verifica se uma expressão é uma chamada recursiva para `func_name`.
///
/// Aceita `Grouping` wrapper (parênteses) transparente.
fn is_recursive_call(expr: &Spanned<TypedExpr>, func_name: &str) -> bool {
    match &expr.node.kind {
        TypedExprKind::Closure { callee, ffi_symbol, .. } => {
            // ffi_symbol = None significa função Kata pura (não FFI)
            ffi_symbol.is_none()
                && matches!(&callee.node.kind, TypedExprKind::Ident { name } if name == func_name)
        }
        TypedExprKind::Grouping { inner } => is_recursive_call(inner, func_name),
        _ => false,
    }
}

/// Extrai o nome de um Ident de uma expressão (ignorando Grouping).
fn extract_ident_name(expr: &Spanned<TypedExpr>) -> Option<String> {
    match &expr.node.kind {
        TypedExprKind::Ident { name } => Some(name.clone()),
        TypedExprKind::Grouping { inner } => extract_ident_name(inner),
        _ => None,
    }
}

/// Tira o wrapper Grouping de uma expressão, se houver.
fn unwrap_grouping(expr: &Spanned<TypedExpr>) -> &Spanned<TypedExpr> {
    match &expr.node.kind {
        TypedExprKind::Grouping { inner } => unwrap_grouping(inner),
        _ => expr,
    }
}

/// Informação extraída de uma chamada recursiva dentro de um operador associativo.
struct RecCallInfo {
    /// O argumento passado para a chamada recursiva (ex: `(- n 1)`).
    /// Já com Grouping removido se houver.
    rec_arg: Spanned<TypedExpr>,
}

/// Informação do padrão TRMA detectado.
struct TrmaPattern {
    /// Nome da função recursiva (ex: "soma").
    func_name: String,
    /// Operador associativo (ex: "+").
    op: String,
    /// ffi_symbol do operador (ex: "kata_rt_bi_add").
    op_ffi: String,
    /// Elemento neutro (ex: 0).
    neutral: i64,
    /// Argumento não-recursivo do operador (ex: `n`).
    non_rec_arg: Spanned<TypedExpr>,
    /// Argumento da chamada recursiva (ex: `(- n 1)`).
    rec_arg: Spanned<TypedExpr>,
    /// Pattern do caso base (ex: `Literal(0)`).
    base_pattern: Spanned<TypedPattern>,
    /// Valor retornado no caso base (ex: `0`).
    base_value: Spanned<TypedExpr>,
}

/// Verifica se uma função é candidata a TRMA.
fn is_trma_candidate(
    func: &TypedFunction,
    table: &DispatchTable,
) -> Option<TrmaPattern> {
    // 1. Tem exatamente 1 parâmetro.
    if func.param_types.len() != 1 {
        return None;
    }

    // 2. Tem exatamente 1 cláusula.
    if func.clauses.len() != 1 {
        return None;
    }

    let clause = &func.clauses[0];

    // 3. O body é Match.
    let arms = match &clause.body.node.kind {
        TypedExprKind::Match { scrutinee, arms } => arms,
        _ => return None,
    };

    // 4. Procurar: um arm é caso base (não-recursivo), outro é recursivo com op associativo.
    let mut base_arm = None;
    let mut rec_arm = None;

    for arm in arms {
        if is_recursive_call(&arm.body, &func.name) {
            // Body inteiro é chamada recursiva — não é TRMA (precisa de op associativo)
            return None;
        }
        // Verifica se o body é `op(arg, self_call(arg))` onde op é associativo
        if let Some(_) = detect_assoc_recursion(&arm.body, &func.name, table) {
            rec_arm = Some(arm);
        } else {
            base_arm = Some(arm);
        }
    }

    let base_arm = base_arm?;
    let rec_arm = rec_arm?;

    // 5. Extrair o padrão do arm recursivo.
    let rec_info = detect_assoc_recursion(&rec_arm.body, &func.name, table)?;

    Some(TrmaPattern {
        func_name: func.name.clone(),
        op: rec_info.op_name,
        op_ffi: rec_info.op_ffi,
        neutral: rec_info.neutral,
        non_rec_arg: rec_info.non_rec_arg,
        rec_arg: rec_info.rec_arg,
        base_pattern: base_arm.pattern.clone()?,
        base_value: base_arm.body.clone(),
    })
}

struct AssocRecursion {
    op_name: String,
    op_ffi: String,
    neutral: i64,
    non_rec_arg: Spanned<TypedExpr>,
    rec_arg: Spanned<TypedExpr>,
}

/// Verifica se `expr` é `op(arg1, self_call(arg2))` onde `op` é associativo.
fn detect_assoc_recursion(
    expr: &Spanned<TypedExpr>,
    func_name: &str,
    table: &DispatchTable,
) -> Option<AssocRecursion> {
    let inner = unwrap_grouping(expr);

    match &inner.node.kind {
        TypedExprKind::Closure {
            callee,
            args,
            ffi_symbol,
        } => {
            // Verifica que op é associativo no DispatchTable.
            let op_name = extract_ident_name(callee)?;
            let overloads = table.get_overloads(&op_name)?;
            let overload = overloads.first()?;
            let neutral = overload.associative_neutral?;

            // Precisa de ffi_symbol (para repassar no codegen).
            let op_ffi = ffi_symbol.clone()?;

            // Um dos args é a chamada recursiva, o outro não é.
            if args.len() != 2 {
                return None;
            }

            let arg0 = &args[0];
            let arg1 = &args[1];

            let (non_rec_arg, rec_arg_expr) = if is_recursive_call(arg1, func_name) {
                (arg0.clone(), arg1.clone())
            } else if is_recursive_call(arg0, func_name) {
                (arg1.clone(), arg0.clone())
            } else {
                return None;
            };

            // Extrair o argumento da chamada recursiva (tirar Grouping).
            let rec_arg = extract_rec_arg(&rec_arg_expr)?;

            Some(AssocRecursion {
                op_name,
                op_ffi,
                neutral,
                non_rec_arg,
                rec_arg,
            })
        }
        _ => None,
    }
}

/// Extrai o argumento de uma chamada recursiva, removendo Grouping.
fn extract_rec_arg(rec_call: &Spanned<TypedExpr>) -> Option<Spanned<TypedExpr>> {
    let inner = unwrap_grouping(rec_call);
    match &inner.node.kind {
        TypedExprKind::Closure { args, .. } => {
            // A chamada recursiva tem 1 argumento: `soma (- n 1)`.
            // Pode estar envolvido em Grouping.
            Some(args.first()?.clone())
        }
        _ => None,
    }
}

/// Cria um `Spanned<TypedExpr>` sintético com tipo apropriado.
fn syn_expr(kind: TypedExprKind, ty: Ty) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty,
            tail_pos: false,
            effect: Effect::Puro,
            kind,
        },
        Span::synthetic(),
    )
}

/// Cria um `Spanned<TypedExpr>` sintético em tail_pos.
fn syn_tail_expr(kind: TypedExprKind, ty: Ty) -> Spanned<TypedExpr> {
    Spanned::new(
        TypedExpr {
            span: Span::synthetic(),
            ty,
            tail_pos: true,
            effect: Effect::Puro,
            kind,
        },
        Span::synthetic(),
    )
}

/// Reescreve a função com acumulador.
///
/// Retorna (soma_rewritten, soma_acc) onde:
/// - `soma_rewritten` tem 1 param e chama `soma_acc n 0`
/// - `soma_acc` tem 2 params (n, acc) e faz a recursão de cauda
fn rewrite_with_accumulator(
    func: &TypedFunction,
    pattern: &TrmaPattern,
) -> (TypedFunction, TypedFunction) {
    let acc_name = format!("{}_acc", func.name);
    let param_ty = func.param_types[0].clone();
    let ret_ty = func.ret_ty.clone();

    // Extrai o nome do parâmetro original (ex: "n")
    let orig_param_name = func.clauses[0]
        .patterns
        .first()
        .and_then(|p| match &p.node {
            TypedPattern::Ident { name, .. } => Some(name.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "n".to_string());

    // ── Função original reescrita: chama _acc com (n, neutral) ──
    let original = TypedFunction {
        name: func.name.clone(),
        param_types: func.param_types.clone(),
        ret_ty: ret_ty.clone(),
        clauses: vec![TypedLambdaClause {
            patterns: func.clauses[0].patterns.clone(),
            body: syn_tail_expr(
                TypedExprKind::Closure {
                    callee: Box::new(syn_expr(
                        TypedExprKind::Ident {
                            name: acc_name.clone(),
                        },
                        Ty::Function(
                            vec![param_ty.clone(), param_ty.clone()],
                            Box::new(ret_ty.clone()),
                        ),
                    )),
                    args: vec![
                        syn_expr(
                            TypedExprKind::Ident {
                                name: orig_param_name.clone(),
                            },
                            param_ty.clone(),
                        ),
                        syn_expr(
                            TypedExprKind::IntLit {
                                text: pattern.neutral.to_string(),
                            },
                            param_ty.clone(),
                        ),
                    ],
                    ffi_symbol: None,
                },
                ret_ty.clone(),
            ),
            guards: vec![],
            with_bindings: vec![],
        }],
    };

    // ── Função _acc: match n { base: acc, otherwise: _acc(rec_arg, op(acc, non_rec_arg)) } ──
    let acc_func = TypedFunction {
        name: acc_name.clone(),
        param_types: vec![param_ty.clone(), param_ty.clone()],
        ret_ty: ret_ty.clone(),
        clauses: vec![TypedLambdaClause {
            patterns: vec![
                Spanned::new(
                    TypedPattern::Ident {
                        name: "n".into(),
                        ty: param_ty.clone(),
                    },
                    Span::synthetic(),
                ),
                Spanned::new(
                    TypedPattern::Ident {
                        name: "acc".into(),
                        ty: param_ty.clone(),
                    },
                    Span::synthetic(),
                ),
            ],
            body: syn_tail_expr(
                TypedExprKind::Match {
                    scrutinee: Box::new(syn_expr(
                        TypedExprKind::Ident { name: "n".into() },
                        param_ty.clone(),
                    )),
                    arms: vec![
                        // caso base: retorna acc
                        TypedMatchArm {
                            pattern: Some(Spanned::new(
                                pattern.base_pattern.node.clone(),
                                Span::synthetic(),
                            )),
                            guard: None,
                            body: syn_tail_expr(
                                TypedExprKind::Ident { name: "acc".into() },
                                ret_ty.clone(),
                            ),
                        },
                        // caso recursivo: _acc(rec_arg, op(acc, non_rec_arg))
                        TypedMatchArm {
                            pattern: None, // otherwise
                            guard: None,
                            body: syn_tail_expr(
                                TypedExprKind::Closure {
                                    callee: Box::new(syn_expr(
                                        TypedExprKind::Ident {
                                            name: acc_name.clone(),
                                        },
                                        Ty::Function(
                                            vec![param_ty.clone(), param_ty.clone()],
                                            Box::new(ret_ty.clone()),
                                        ),
                                    )),
                                    args: vec![
                                        // rec_arg (ex: (- n 1))
                                        pattern.rec_arg.clone(),
                                        // op(acc, non_rec_arg) (ex: + acc n)
                                        syn_expr(
                                            TypedExprKind::Closure {
                                                callee: Box::new(syn_expr(
                                                    TypedExprKind::Ident {
                                                        name: pattern.op.clone(),
                                                    },
                                                    Ty::Function(
                                                        vec![param_ty.clone(), param_ty.clone()],
                                                        Box::new(ret_ty.clone()),
                                                    ),
                                                )),
                                                args: vec![
                                                    syn_expr(
                                                        TypedExprKind::Ident {
                                                            name: "acc".into(),
                                                        },
                                                        param_ty.clone(),
                                                    ),
                                                    pattern.non_rec_arg.clone(),
                                                ],
                                                ffi_symbol: Some(pattern.op_ffi.clone()),
                                            },
                                            ret_ty.clone(),
                                        ),
                                    ],
                                    ffi_symbol: None,
                                },
                                ret_ty.clone(),
                            ),
                        },
                    ],
                },
                ret_ty.clone(),
            ),
            guards: vec![],
            with_bindings: vec![],
        }],
    };

    (original, acc_func)
}

/// Pass TRMA — percorre funções e reescreve candidatas.
pub fn trma_pass(typed: &mut TypedModule) {
    let mut new_functions = Vec::new();
    for func in &typed.functions {
        if let Some(pattern) = is_trma_candidate(func, &typed.dispatch_table) {
            let (rewritten, acc) = rewrite_with_accumulator(func, &pattern);
            new_functions.push(rewritten);
            new_functions.push(acc);
        } else {
            new_functions.push(func.clone());
        }
    }
    typed.functions = new_functions;
}