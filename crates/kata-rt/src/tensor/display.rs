//! Display — formatação tabular de tensores N-D.
//!
//! Itera coordenadas N-D via strides (respeita transposições zero-copy e
//! sub-tensores não-contíguos) e monta a representação de display alinhada
//! por colunas.

use crate::bytes::untag_smi;

use super::{ELEM_INT, read_elem_float, read_elem_int, shape_nelems};

/// Constrói a representação de display de um elemento do tensor.
///
/// # Safety
/// `ptr` deve ser um header de tensor válido e `flat` um índice in-bounds.
pub(super) unsafe fn format_elem(ptr: i64, flat: i64, elem_type: i64) -> String {
    unsafe {
        if elem_type == ELEM_INT {
            let v = read_elem_int(ptr, flat);
            let untagged = untag_smi(v);
            untagged.to_string()
        } else {
            let v = read_elem_float(ptr, flat);
            format!("{}", v)
        }
    }
}

/// Calcula o flat index a partir de coords N-D e strides.
pub(super) fn coords_to_flat(coords: &[i64], strides: &[i64]) -> i64 {
    coords
        .iter()
        .zip(strides.iter())
        .map(|(&c, &s)| c * s)
        .sum()
}

/// Formata o tensor como string tabular.
///
/// Itera coordenadas N-D (usando shape) e mapeia cada uma para o flat index
/// real via strides. Isto respeita transposições (zero-copy stride swap) e
/// sub-tensores não-contíguos.
pub(super) fn format_tensor(ptr: i64, shape: &[i64], strides: &[i64], elem_type: i64) -> String {
    let rank = shape.len();
    let nelems = shape_nelems(shape);
    if nelems == 0 {
        return String::new();
    }

    if rank == 1 {
        // 1-D: uma linha, espaços entre colunas
        let mut cells: Vec<String> = Vec::with_capacity(nelems as usize);
        for c in 0..shape[0] {
            let flat = c * strides[0];
            cells.push(unsafe { format_elem(ptr, flat, elem_type) });
        }
        let max_width = cells.iter().map(|s| s.len()).max().unwrap_or(0);
        let padded: Vec<String> = cells
            .iter()
            .map(|s| format!("{:>width$}", s, width = max_width))
            .collect();
        return padded.join("  ");
    }

    if rank == 2 {
        let rows = shape[0] as usize;
        let cols = shape[1] as usize;
        // Formata células usando strides para mapear coords → flat
        let mut cells: Vec<String> = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let flat = (r as i64) * strides[0] + (c as i64) * strides[1];
                cells.push(unsafe { format_elem(ptr, flat, elem_type) });
            }
        }
        // Largura de cada coluna
        let mut col_widths = vec![0usize; cols];
        for r in 0..rows {
            for c in 0..cols {
                let cell = &cells[r * cols + c];
                col_widths[c] = col_widths[c].max(cell.len());
            }
        }
        let mut lines = Vec::new();
        for r in 0..rows {
            let row_cells: Vec<String> = (0..cols)
                .map(|c| format!("{:>width$}", cells[r * cols + c], width = col_widths[c]))
                .collect();
            lines.push(row_cells.join("  "));
        }
        return lines.join("\n");
    }

    // Rank > 2: fatias 2-D ao longo do eixo 0
    let n_slices = shape[0] as usize;
    let slice_shape = &shape[1..];
    let slice_strides = &strides[1..];
    let slice_nelems = shape_nelems(slice_shape);
    let mut parts = Vec::new();
    for s in 0..n_slices {
        let base_offset = (s as i64) * strides[0];
        // Coleta células da fatia usando slice_strides
        let mut slice_cells: Vec<String> = Vec::with_capacity(slice_nelems as usize);
        // Itera coords N-D da fatia (rank-1 dimensões)
        let slice_rank = slice_shape.len();
        let mut coords = vec![0i64; slice_rank];
        for _ in 0..slice_nelems {
            let flat = base_offset + coords_to_flat(&coords, slice_strides);
            slice_cells.push(unsafe { format_elem(ptr, flat, elem_type) });
            // Incrementa coords (row-major order para display)
            for i in (0..slice_rank).rev() {
                coords[i] += 1;
                if coords[i] < slice_shape[i] {
                    break;
                }
                coords[i] = 0;
            }
        }
        let slice_str = format_2d_or_deeper(&slice_cells, slice_shape);
        if n_slices > 1 {
            parts.push(format!("[{}]:\n{}", s, slice_str));
        } else {
            parts.push(slice_str);
        }
    }
    parts.join("\n\n")
}

/// Formata uma fatia (rank-1 ou rank-2) a partir de células já convertidas em string.
/// As células já estão em ordem row-major (o chamador mapeia via strides).
fn format_2d_or_deeper(cells: &[String], shape: &[i64]) -> String {
    let rank = shape.len();
    if rank == 1 {
        let max_width = cells.iter().map(|s| s.len()).max().unwrap_or(0);
        let padded: Vec<String> = cells
            .iter()
            .map(|s| format!("{:>width$}", s, width = max_width))
            .collect();
        return padded.join("  ");
    }
    // rank == 2
    let rows = shape[0] as usize;
    let cols = shape[1] as usize;
    let mut col_widths = vec![0usize; cols];
    for r in 0..rows {
        for c in 0..cols {
            let cell = &cells[r * cols + c];
            col_widths[c] = col_widths[c].max(cell.len());
        }
    }
    let mut lines = Vec::new();
    for r in 0..rows {
        let row_cells: Vec<String> = (0..cols)
            .map(|c| format!("{:>width$}", cells[r * cols + c], width = col_widths[c]))
            .collect();
        lines.push(row_cells.join("  "));
    }
    lines.join("\n")
}
