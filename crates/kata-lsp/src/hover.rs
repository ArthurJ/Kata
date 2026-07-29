//! Busca de tipo no TAST por posição (hover).
//!
//! Dada uma posição LSP, converte para byte offset, busca o `TypedExpr` cujo
//! span contém o offset (mais específico = mais profundo), e retorna o tipo.

use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use crate::unicode::{byte_offset_to_lsp_position, lsp_position_to_byte_offset};

/// Busca o `TypedExpr` na posição dada e retorna um `Hover` com o tipo.
pub(crate) fn hover_at(
    typed: &kata_inference::TypedModule,
    text: &str,
    pos: Position,
) -> Option<Hover> {
    let offset = lsp_position_to_byte_offset(text, pos);

    // Busca no entry point e pre_entry (expressões top-level)
    let best = typed
        .pre_entry
        .iter()
        .chain(std::iter::once(&typed.entry))
        .filter_map(|expr| find_typed_expr_at(expr, offset))
        .min_by_key(|(_, depth)| *depth);

    // Se não encontrou no entry, busca nos bodies das actions
    let best = best.or_else(|| {
        typed
            .actions
            .iter()
            .flat_map(|a| &a.body)
            .filter_map(|expr| find_typed_expr_at(expr, offset))
            .min_by_key(|(_, depth)| *depth)
    });

    // Se não encontrou nas actions, busca nas cláusulas das funções
    let best = best.or_else(|| {
        typed
            .functions
            .iter()
            .flat_map(|f| &f.clauses)
            .filter_map(|clause| find_typed_expr_at(&clause.body, offset))
            .min_by_key(|(_, depth)| *depth)
    });

    let (expr, _) = best?;

    let ty_str = format!("{}", expr.ty);
    let range = span_to_range(text, expr.span);

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```kata\n{ty_str}\n```"),
        }),
        range: Some(range),
    })
}

/// Encontra o `TypedExpr` mais específico cujo span contém o offset.
/// Retorna `(expr, depth)` onde depth = profundidade na árvore (menor = mais específico).
fn find_typed_expr_at(
    expr: &kata_ast::Spanned<kata_inference::TypedExpr>,
    offset: usize,
) -> Option<(&kata_inference::TypedExpr, usize)> {
    let span = expr.node.span;
    if !span_contains(span, offset) {
        return None;
    }

    // Tenta descer para os filhos — o filho mais profundo que contém
    // o offset é mais específico.
    for child in children(&expr.node.kind) {
        if let Some((found, depth)) = find_typed_expr_at(child, offset) {
            return Some((found, depth + 1));
        }
    }

    // Nenhum filho contém o offset — este é o nó mais específico
    Some((&expr.node, 0))
}

/// Verifica se o span contém o offset (inclusivo no início, exclusivo no fim).
fn span_contains(span: kata_ast::Span, offset: usize) -> bool {
    offset >= span.offset && offset < span.offset + span.len
}

/// Itera sobre os filhos `Spanned<TypedExpr>` de um `TypedExprKind`.
fn children<'a>(
    kind: &'a kata_inference::TypedExprKind,
) -> Box<dyn Iterator<Item = &'a kata_ast::Spanned<kata_inference::TypedExpr>> + 'a> {
    use kata_inference::TypedExprKind::*;

    match kind {
        // Sem filhos
        IntLit { .. }
        | FloatLit { .. }
        | TextLit { .. }
        | Unit
        | Ident { .. }
        | Break
        | Continue
        | VariantQual { .. }
        | ChannelCreate { .. }
        | HeapSnapshot { .. } => Box::new(std::iter::empty()),
        Comptime { expr } => Box::new(std::iter::once(expr.as_ref())),

        // Um filho Box<Spanned<TypedExpr>>
        Grouping { inner } => Box::new(std::iter::once(inner.as_ref())),
        TypeAscription { expr, .. } => Box::new(std::iter::once(expr.as_ref())),
        TypeOf { expr } => Box::new(std::iter::once(expr.as_ref())),
        Return(expr) => Box::new(std::iter::once(expr.as_ref())),
        Reassign { value, .. } => Box::new(std::iter::once(value.as_ref())),
        ChannelRecv { channel, .. } => Box::new(std::iter::once(channel.as_ref())),
        ReceiverFactoryCall { factory, .. } => Box::new(std::iter::once(factory.as_ref())),

        // Closure: callee + args
        Closure { callee, args, .. } => {
            Box::new(std::iter::once(callee.as_ref()).chain(args.iter()))
        }

        // Let / Var
        Let { value, .. } | Var { value, .. } => Box::new(std::iter::once(value.as_ref())),

        // LetDestruct: value + bindings
        LetDestruct {
            value, bindings, ..
        } => Box::new(std::iter::once(value.as_ref()).chain(bindings.iter().map(|(_, e)| e))),

        // Tuple / ListLit / ArrayLit / StructConstruct
        Tuple { elements }
        | ListLit { elements }
        | ArrayLit { elements }
        | StructConstruct {
            values: elements, ..
        }
        | SetLit { elements, .. } => Box::new(elements.iter()),

        // FieldAccess / IndexAccess
        FieldAccess { expr, .. } | IndexAccess { expr, .. } => {
            Box::new(std::iter::once(expr.as_ref()))
        }

        // VariantConstruct
        VariantConstruct { payload, .. } => Box::new(std::iter::once(payload.as_ref())),

        // Match: scrutinee + arm bodies
        Match { scrutinee, arms } => {
            Box::new(std::iter::once(scrutinee.as_ref()).chain(arms.iter().map(|a| &a.body)))
        }

        // ActionCall: args + indirect_callee
        ActionCall {
            args,
            indirect_callee,
            ..
        } => Box::new(
            std::iter::once(args.as_ref()).chain(indirect_callee.iter().map(|e| e.as_ref())),
        ),

        // Loop body
        Loop { body } => Box::new(body.iter()),

        // ForIn: iterable + body
        ForIn { iterable, body, .. } => {
            Box::new(std::iter::once(iterable.as_ref()).chain(body.iter()))
        }

        // In: item + collection
        In { item, collection } => {
            Box::new(std::iter::once(item.as_ref()).chain(std::iter::once(collection.as_ref())))
        }

        // DictLit: entries (key + value)
        DictLit { entries, .. } => Box::new(
            entries
                .iter()
                .flat_map(|(k, v)| [k, v])
                .collect::<Vec<_>>()
                .into_iter(),
        ),

        // RangeLit: start + step + end
        RangeLit {
            start, step, end, ..
        } => Box::new([start.as_ref(), step.as_ref(), end.as_ref()].into_iter()),

        // Map / Filter: callback + collection
        Map {
            callback,
            collection,
            ..
        }
        | Filter {
            callback,
            collection,
            ..
        } => Box::new([callback.as_ref(), collection.as_ref()].into_iter()),

        // Fold: callback + initial + collection
        Fold {
            callback,
            initial,
            collection,
            ..
        } => Box::new([callback.as_ref(), initial.as_ref(), collection.as_ref()].into_iter()),

        // FusedStream: source + stage callbacks
        FusedStream { source, stages, .. } => Box::new(std::iter::once(source.as_ref()).chain(
            stages.iter().map(|s| match s {
                kata_inference::FusedStage::Filter { callback, .. } => callback.as_ref(),
                kata_inference::FusedStage::Map { callback, .. } => callback.as_ref(),
            }),
        )),

        // ChannelSend: channel + value
        ChannelSend { channel, value } => Box::new([channel.as_ref(), value.as_ref()].into_iter()),

        // Select: arm bodies + timeout
        Select {
            arms,
            timeout_ms,
            timeout_body,
            ..
        } => Box::new(
            arms.iter()
                .map(|a| &a.body)
                .chain(timeout_ms.iter().map(|e| e.as_ref()))
                .chain(timeout_body.iter().map(|e| e.as_ref())),
        ),

        // Fork: action_expr + args
        Fork {
            action_expr, args, ..
        } => Box::new([action_expr.as_ref(), args.as_ref()].into_iter()),

        // Lambda: cláusulas — body de cada cláusula
        Lambda { clauses, .. } => Box::new(clauses.iter().map(|c| &c.body)),
    }
}

fn span_to_range(text: &str, span: kata_ast::Span) -> Range {
    let start = byte_offset_to_lsp_position(text, span.offset);
    let end = byte_offset_to_lsp_position(text, span.offset + span.len);
    Range { start, end }
}
