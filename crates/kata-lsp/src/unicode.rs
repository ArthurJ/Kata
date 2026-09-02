//! Conversão entre byte offsets (Span do Kata) e LSP Position (UTF-16 code units).

/// Converte byte offset (do início do arquivo) → LSP Position (0-indexed line, UTF-16 char).
pub(crate) fn byte_offset_to_lsp_position(text: &str, byte_offset: usize) -> Position {
    let offset = byte_offset.min(text.len());
    let prefix = &text[..offset];
    let line = prefix.matches('\n').count() as u32;
    let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_prefix = &text[line_start..offset];
    let character = line_prefix.encode_utf16().count() as u32;
    Position { line, character }
}

/// Converte LSP Position (0-indexed line, UTF-16 char) → byte offset.
pub(crate) fn lsp_position_to_byte_offset(text: &str, pos: Position) -> usize {
    let line = pos.line as usize;
    let target_utf16 = pos.character as usize;
    let mut line_start = 0;
    for _ in 0..line {
        match text[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => return text.len(),
        }
    }
    let line_end = text[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(text.len());
    let mut utf16_acc = 0usize;
    let mut byte_pos = line_start;
    for c in text[line_start..line_end].chars() {
        if utf16_acc >= target_utf16 {
            break;
        }
        utf16_acc += c.len_utf16();
        byte_pos += c.len_utf8();
    }
    byte_pos
}

use tower_lsp::lsp_types::Position;

#[cfg(test)]
#[path = "unicode_tests.rs"]
mod tests;
