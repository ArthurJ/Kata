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
mod tests {
    use super::*;

    #[test]
    fn ascii() {
        let text = "hello world";
        let pos = byte_offset_to_lsp_position(text, 0);
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 0
            }
        );

        let pos = byte_offset_to_lsp_position(text, 6);
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 6
            }
        );
    }

    #[test]
    fn bmp_accented() {
        let text = "Olá mundo";
        // O=0, l=1, á=2bytes, offset 4 = byte after á
        let pos = byte_offset_to_lsp_position(text, 4);
        // O(1) + l(1) + á(1 UTF-16 unit) = 3
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 3
            }
        );
    }

    #[test]
    fn supplementary_emoji() {
        let text = "x 🚀 y";
        // x=0, space=1, 🚀=4 bytes at offset 2-5, space at offset 6
        let pos = byte_offset_to_lsp_position(text, 6);
        // x(1) + space(1) + 🚀(2 UTF-16 units) = 4
        assert_eq!(
            pos,
            Position {
                line: 0,
                character: 4
            }
        );
    }

    #[test]
    fn multiline() {
        let text = "line1\nline2\nline3";
        let pos = byte_offset_to_lsp_position(text, 7);
        assert_eq!(
            pos,
            Position {
                line: 1,
                character: 1
            }
        );
    }

    #[test]
    fn roundtrip_ascii() {
        let text = "hello world";
        for i in 0..=text.len() {
            let pos = byte_offset_to_lsp_position(text, i);
            let back = lsp_position_to_byte_offset(text, pos);
            assert_eq!(back, i, "roundtrip failed at byte {}", i);
        }
    }

    #[test]
    fn roundtrip_bmp() {
        let text = "Olá café";
        // Iterar só em char boundaries ( offsets válidos para &text[..i] )
        let mut offsets = vec![0];
        let mut acc = 0;
        for c in text.chars() {
            acc += c.len_utf8();
            offsets.push(acc);
        }
        for &i in &offsets {
            let pos = byte_offset_to_lsp_position(text, i);
            let back = lsp_position_to_byte_offset(text, pos);
            assert_eq!(back, i, "roundtrip failed at byte {}", i);
        }
    }

    #[test]
    fn roundtrip_supplementary() {
        let text = "a 🚀 b";
        let mut offsets = vec![0];
        let mut acc = 0;
        for c in text.chars() {
            acc += c.len_utf8();
            offsets.push(acc);
        }
        for &i in &offsets {
            let pos = byte_offset_to_lsp_position(text, i);
            let back = lsp_position_to_byte_offset(text, pos);
            assert_eq!(back, i, "roundtrip failed at byte {}", i);
        }
    }

    #[test]
    fn lsp_to_byte_multiline() {
        let text = "line1\nline2\nline3";
        let offset = lsp_position_to_byte_offset(
            text,
            Position {
                line: 2,
                character: 2,
            },
        );
        assert_eq!(offset, 14); // "line1\nline2\nli" = 14 bytes
    }
}
