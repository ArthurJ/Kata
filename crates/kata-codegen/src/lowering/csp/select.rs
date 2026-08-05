//! Lowering de `TypedExprKind::Select` — multiplexação de recebimento de canais
//! e leitura de I/O, com timeout opcional.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
use cranelift_codegen::ir::{BlockArg, InstBuilder, MemFlagsData, Value};
use kata_core::ty::Ty;
use kata_inference::{TypedExpr, TypedExprKind, TypedReadMode, TypedSelectArm};

use super::super::LowerCtx;
use super::get_ffi;

/// Lowera `TypedExprKind::Select` — multiplexação de recebimento de canais
/// e leitura de I/O, com timeout opcional.
///
/// Estrutura do lowering:
///
/// 1. Separar arms em channel_arms e io_arms (preservando o índice original).
/// 2. Se há channel arms: alocar array de N_c handles na arena, lowerar cada
///    `arm.channel`, store no array, chamar `kata_rt_select(handles_ptr,
///    N_c, timeout_ms)` → `channel_idx` (i64).
/// 3. Se há io arms: alocar array de N_f handles na arena, lowerar cada
///    `arm.handle_expr`, store no array, chamar
///    `kata_rt_select_files(handles_ptr, N_f)` → `file_idx` (i64).
/// 4. Branch chain na ordem original (de cima para baixo):
///    - Para cada braço i:
///      - Se Channel na posição j de channel_arms: `channel_idx == j` →
///        `kata_rt_channel_recv(handle)` → binding → body.
///      - Se IoRead na posição j de io_arms: `file_idx == j` →
///        `kata_rt_file_read_chunk(handle, chunk_size_untagged)` → binding
///        (Result) → body.
///    - Se `channel_idx == -2` (SELECT_TIMEOUT): lowerar `timeout_body`.
/// 5. `cont_block` junta o resultado de todos os braços.
///
/// O tipo de retorno é o tipo unificado de todos os braços e do timeout_body.
#[allow(clippy::type_complexity)]
pub(crate) fn lower_select(
    expr: &TypedExpr,
    arms: &[TypedSelectArm],
    timeout_ms: &Option<Box<kata_ast::Spanned<TypedExpr>>>,
    timeout_body: &Option<Box<kata_ast::Spanned<TypedExpr>>>,
    ctx: &mut LowerCtx,
) -> Result<Value, super::super::CodegenError> {
    use kata_inference::TypedSelectArm;

    let ret_clif = super::super::resolve_clif_ty(&expr.ty, ctx.struct_registry);

    // Arena para alocar os arrays de handles — fiber_arena (consistente com
    // channel_create: os arrays são efêmeros, vivem apenas durante o select).
    let arena = ctx
        .fiber_arena
        .unwrap_or_else(|| ctx.builder.ins().iconst(I64, 0));

    let alloc_fref = get_ffi(ctx, "kata_rt_arena_alloc")?;

    let flags = MemFlagsData::new();

    // ── 1. Separar arms em channel_arms, file_arms e socket_arms. ──
    // channel_arms[j] = (orig_idx, &Channel{...})
    // file_arms[j]    = (orig_idx, &IoRead{...}) onde handle_expr.ty == Ty::File
    // socket_arms[j]  = (orig_idx, &IoRead{...}) onde handle_expr.ty == Ty::Socket
    let mut channel_arms: Vec<(
        usize,
        &kata_ast::Spanned<TypedExpr>,
        &Ty,
        &String,
        &kata_ast::Spanned<TypedExpr>,
    )> = Vec::new();
    let mut file_arms: Vec<(
        usize,
        &kata_ast::Spanned<TypedExpr>,
        &TypedReadMode,
        &Ty,
        &String,
        &kata_ast::Spanned<TypedExpr>,
    )> = Vec::new();
    let mut socket_arms: Vec<(
        usize,
        &kata_ast::Spanned<TypedExpr>,
        &TypedReadMode,
        &Ty,
        &String,
        &kata_ast::Spanned<TypedExpr>,
    )> = Vec::new();
    for (i, arm) in arms.iter().enumerate() {
        match arm {
            TypedSelectArm::Channel {
                channel,
                recv_ty,
                bind_name,
                body,
            } => {
                channel_arms.push((i, channel, recv_ty, bind_name, body));
            }
            TypedSelectArm::IoRead {
                handle_expr,
                read_mode,
                bind_ty,
                bind_name,
                body,
            } => {
                if handle_expr.node.ty == Ty::Socket {
                    socket_arms.push((i, handle_expr, read_mode, bind_ty, bind_name, body));
                } else {
                    file_arms.push((i, handle_expr, read_mode, bind_ty, bind_name, body));
                }
            }
        }
    }

    let n_c = channel_arms.len() as i64;
    let n_f = file_arms.len() as i64;
    let n_s = socket_arms.len() as i64;

    // ── 2. Alocar arrays de handles para channels, files e sockets. ──
    // channel_handle_values[j] = valor CLIF do handle do j-ésimo channel arm.
    let mut channel_handle_values: Vec<Value> = Vec::new();
    // file_handle_values[j]  = valor CLIF do handle do j-ésimo file arm.
    let mut file_handle_values: Vec<Value> = Vec::new();
    // file_chunk_values[j]   = Some(SMI-tagged chunk_size) se Chunk, None se Line.
    let mut file_chunk_values: Vec<Option<Value>> = Vec::new();
    // socket_handle_values[j]  = valor CLIF do handle do j-ésimo socket arm.
    let mut socket_handle_values: Vec<Value> = Vec::new();
    // socket_chunk_values[j]   = Some(SMI-tagged chunk_size) se Chunk, None se Line.
    let mut socket_chunk_values: Vec<Option<Value>> = Vec::new();

    // Alocar array de channel handles (se houver channel arms).
    let chan_ptr = if !channel_arms.is_empty() {
        let array_size = ctx.builder.ins().iconst(I64, n_c * 8);
        let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, array_size]);
        let ptr = ctx.builder.inst_results(alloc_inst)[0];

        for (j, (_orig, channel, _recv_ty, _bind_name, _body)) in channel_arms.iter().enumerate() {
            let handle = super::super::expr::lower_expr(&channel.node, ctx)?;
            channel_handle_values.push(handle);
            let offset = (j as i32) * 8;
            ctx.builder.ins().store(flags, handle, ptr, offset);
        }
        ptr
    } else {
        ctx.builder.ins().iconst(I64, 0) // null ptr
    };

    // Alocar array de file handles (se houver file arms).
    let file_ptr = if !file_arms.is_empty() {
        let array_size = ctx.builder.ins().iconst(I64, n_f * 8);
        let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, array_size]);
        let ptr = ctx.builder.inst_results(alloc_inst)[0];

        for (j, (_orig, handle_expr, read_mode, _bind_ty, _bind_name, _body)) in
            file_arms.iter().enumerate()
        {
            let handle = super::super::expr::lower_expr(&handle_expr.node, ctx)?;
            file_handle_values.push(handle);
            let offset = (j as i32) * 8;
            ctx.builder.ins().store(flags, handle, ptr, offset);

            // chunk_size_expr: lowerar se Chunk, None se Line.
            let chunk_smi = match read_mode {
                TypedReadMode::Chunk(chunk_size_expr) => {
                    Some(super::super::expr::lower_expr(&chunk_size_expr.node, ctx)?)
                }
                TypedReadMode::Line => None,
            };
            file_chunk_values.push(chunk_smi);
        }
        ptr
    } else {
        ctx.builder.ins().iconst(I64, 0) // null ptr
    };

    // Alocar array de socket handles (se houver socket arms).
    let socket_ptr = if !socket_arms.is_empty() {
        let array_size = ctx.builder.ins().iconst(I64, n_s * 8);
        let alloc_inst = ctx.builder.ins().call(alloc_fref, &[arena, array_size]);
        let ptr = ctx.builder.inst_results(alloc_inst)[0];

        for (j, (_orig, handle_expr, read_mode, _bind_ty, _bind_name, _body)) in
            socket_arms.iter().enumerate()
        {
            let handle = super::super::expr::lower_expr(&handle_expr.node, ctx)?;
            socket_handle_values.push(handle);
            let offset = (j as i32) * 8;
            ctx.builder.ins().store(flags, handle, ptr, offset);

            // chunk_size_expr: lowerar se Chunk, None se Line.
            let chunk_smi = match read_mode {
                TypedReadMode::Chunk(chunk_size_expr) => {
                    Some(super::super::expr::lower_expr(&chunk_size_expr.node, ctx)?)
                }
                TypedReadMode::Line => None,
            };
            socket_chunk_values.push(chunk_smi);
        }
        ptr
    } else {
        ctx.builder.ins().iconst(I64, 0) // null ptr
    };

    // ── 3. Lowerar timeout_ms (se presente) ou -1 (sem timeout). ──
    // O timeout_ms é uma TypedExpr Int — lower_expr retorna SMI-tagged.
    // A FFI espera o valor cru em ms, então fazemos untag: val >> 1.
    let timeout_val = if let Some(tm) = timeout_ms {
        let smi_val = super::super::expr::lower_expr(&tm.node, ctx)?;
        // Untag SMI: ushr_imm por 1. Para SMIs positivos (timeout sempre
        // positivo), é seguro.
        ctx.builder.ins().ushr_imm(smi_val, 1)
    } else {
        ctx.builder.ins().iconst(I64, -1)
    };

    // ── 4. Chamar kata_rt_select_combined(chan_ptr, n_c, file_ptr, n_f, socket_ptr, n_s, timeout_ms). ──
    // Retorna índice global: 0..n_c-1 = channel, n_c..n_c+n_f-1 = file,
    // n_c+n_f..n_c+n_f+n_s-1 = socket. -1 = WOULD_BLOCK, -2 = SELECT_TIMEOUT.
    let select_fref = get_ffi(ctx, "kata_rt_select_combined")?;
    let n_c_val = ctx.builder.ins().iconst(I64, n_c);
    let n_f_val = ctx.builder.ins().iconst(I64, n_f);
    let n_s_val = ctx.builder.ins().iconst(I64, n_s);
    let select_inst = ctx.builder.ins().call(
        select_fref,
        &[
            chan_ptr,
            n_c_val,
            file_ptr,
            n_f_val,
            socket_ptr,
            n_s_val,
            timeout_val,
        ],
    );
    let global_idx = ctx.builder.inst_results(select_inst)[0];

    // ── 4. Branch chain na ordem original (de cima para baixo). ──
    // cont_block recebe o resultado de qualquer braço.
    let cont_block = ctx.builder.create_block();
    ctx.builder.append_block_param(cont_block, ret_clif);

    let mut next_test_block = ctx.builder.create_block();
    ctx.builder.ins().jump(next_test_block, &[]);
    ctx.builder.seal_block(next_test_block);

    // Índices relativos dentro de cada sub-lista, rastreados enquanto
    // percorremos os arms na ordem original.
    let mut chan_seen = 0usize;
    let mut file_seen = 0usize;
    let mut socket_seen = 0usize;

    for arm in arms.iter() {
        ctx.builder.switch_to_block(next_test_block);

        let body_block = ctx.builder.create_block();
        next_test_block = ctx.builder.create_block();

        match arm {
            TypedSelectArm::Channel {
                channel: _,
                recv_ty,
                bind_name,
                body,
            } => {
                let j = chan_seen;
                chan_seen += 1;

                // Comparar global_idx == j (channel arms são 0..n_c-1).
                let expected = ctx.builder.ins().iconst(I64, j as i64);
                let is_this =
                    ctx.builder
                        .ins()
                        .icmp(IntCC::Equal, global_idx, expected);
                ctx.builder
                    .ins()
                    .brif(is_this, body_block, &[], next_test_block, &[]);
                ctx.builder.seal_block(next_test_block);
                ctx.builder.seal_block(body_block);

                // body_block: recv do canal j, binding, lowerar body.
                ctx.builder.switch_to_block(body_block);
                ctx.emitted_tail_call = false;

                let recv_fref = get_ffi(ctx, "kata_rt_channel_recv")?;
                let recv_inst = ctx
                    .builder
                    .ins()
                    .call(recv_fref, &[channel_handle_values[j]]);
                let recv_val = ctx.builder.inst_results(recv_inst)[0];

                // Binding do braço (igual lower_channel_recv).
                let clif_ty = super::super::resolve_clif_ty(recv_ty, ctx.struct_registry);
                let var = ctx.new_var(bind_name, clif_ty);
                ctx.builder.def_var(var, recv_val);

                // Lowerar body.
                let body_val = super::super::expr::lower_expr(&body.node, ctx)?;

                let is_terminator = matches!(
                    body.node.kind,
                    TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
                );
                if !is_terminator && !ctx.emitted_tail_call {
                    ctx.builder
                        .ins()
                        .jump(cont_block, &[BlockArg::Value(body_val)]);
                }
            }
            TypedSelectArm::IoRead {
                handle_expr,
                read_mode,
                bind_ty,
                bind_name,
                body,
            } => {
                // Determinar se este braço é File ou Socket pelo tipo do handle.
                let is_socket = handle_expr.node.ty == Ty::Socket;

                let (j, base_offset, handle_values, chunk_values, read_ffi_name) = if is_socket {
                    let j = socket_seen;
                    socket_seen += 1;
                    let ffi_name = match read_mode {
                        TypedReadMode::Chunk(_) => "kata_rt_socket_read_chunk",
                        TypedReadMode::Line => "kata_rt_socket_readline",
                    };
                    (
                        j,
                        n_c + n_f,
                        &socket_handle_values,
                        &socket_chunk_values,
                        ffi_name,
                    )
                } else {
                    let j = file_seen;
                    file_seen += 1;
                    let ffi_name = match read_mode {
                        TypedReadMode::Chunk(_) => "kata_rt_file_read_chunk",
                        TypedReadMode::Line => "kata_rt_file_readline",
                    };
                    (j, n_c, &file_handle_values, &file_chunk_values, ffi_name)
                };

                // Comparar global_idx == base_offset + j.
                let expected = ctx.builder.ins().iconst(I64, base_offset + j as i64);
                let is_this =
                    ctx.builder
                        .ins()
                        .icmp(IntCC::Equal, global_idx, expected);
                ctx.builder
                    .ins()
                    .brif(is_this, body_block, &[], next_test_block, &[]);
                ctx.builder.seal_block(next_test_block);
                ctx.builder.seal_block(body_block);

                // body_block: chamar FFI conforme read_mode.
                // - Chunk: read_chunk(handle, chunk_size_untagged)
                // - Line:  readline(handle)
                ctx.builder.switch_to_block(body_block);
                ctx.emitted_tail_call = false;

                let read_fref = get_ffi(ctx, read_ffi_name)?;

                let result_box_ptr = match read_mode {
                    TypedReadMode::Chunk(_) => {
                        // Untag SMI do chunk_size: val >> 1.
                        let chunk_smi =
                            chunk_values[j].expect("Chunk arm deve ter chunk_size value");
                        let chunk_untagged = ctx.builder.ins().ushr_imm(chunk_smi, 1);
                        let read_inst = ctx
                            .builder
                            .ins()
                            .call(read_fref, &[handle_values[j], chunk_untagged]);
                        ctx.builder.inst_results(read_inst)[0]
                    }
                    TypedReadMode::Line => {
                        // readline(handle) — sem chunk_size.
                        let read_inst = ctx.builder.ins().call(read_fref, &[handle_values[j]]);
                        ctx.builder.inst_results(read_inst)[0]
                    }
                };

                // Binding do braço: bind_ty é o tipo Result.
                let clif_ty = super::super::resolve_clif_ty(bind_ty, ctx.struct_registry);
                let var = ctx.new_var(bind_name, clif_ty);
                ctx.builder.def_var(var, result_box_ptr);

                // Lowerar body.
                let body_val = super::super::expr::lower_expr(&body.node, ctx)?;

                let is_terminator = matches!(
                    body.node.kind,
                    TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
                );
                if !is_terminator && !ctx.emitted_tail_call {
                    ctx.builder
                        .ins()
                        .jump(cont_block, &[BlockArg::Value(body_val)]);
                }
            }
        }
    }

    // ── 5. Timeout / trap no final do branch chain. ──
    // global_idx == -2 (SELECT_TIMEOUT) → timeout_body.
    // Se nenhum braço match e não é timeout → trap (impossível em código válido).
    ctx.builder.switch_to_block(next_test_block);

    if let Some(tb) = timeout_body {
        let timeout_block = ctx.builder.create_block();
        let trap_block = ctx.builder.create_block();

        // global_idx == -2 (SELECT_TIMEOUT).
        let timeout_sentinel = ctx.builder.ins().iconst(I64, -2);
        let is_timeout =
            ctx.builder
                .ins()
                .icmp(IntCC::Equal, global_idx, timeout_sentinel);
        ctx.builder
            .ins()
            .brif(is_timeout, timeout_block, &[], trap_block, &[]);
        ctx.builder.seal_block(timeout_block);
        ctx.builder.seal_block(trap_block);

        // Lowerar timeout_body.
        ctx.builder.switch_to_block(timeout_block);
        ctx.emitted_tail_call = false;
        let tb_val = super::super::expr::lower_expr(&tb.node, ctx)?;

        let is_terminator = matches!(
            tb.node.kind,
            TypedExprKind::Break | TypedExprKind::Continue | TypedExprKind::Return(_)
        );
        if !is_terminator && !ctx.emitted_tail_call {
            ctx.builder
                .ins()
                .jump(cont_block, &[BlockArg::Value(tb_val)]);
        }

        // trap_block: idx não reconhecido (impossível em código válido).
        ctx.builder.switch_to_block(trap_block);
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    } else {
        // Sem timeout_body: idx == -2 não deveria acontecer. Trap por segurança.
        ctx.builder
            .ins()
            .trap(cranelift_codegen::ir::TrapCode::user(1).expect("trap code 1 é sempre válido"));
    }

    // cont_block: resultado do select.
    ctx.builder.seal_block(cont_block);
    ctx.builder.switch_to_block(cont_block);
    let result = ctx.builder.block_params(cont_block)[0];

    ctx.emitted_tail_call = false;
    Ok(result)
}