//! Action State Layout - Calcula layout de memória para variáveis locais de Actions
//!
//! Cada Action compilada como state machine precisa preservar seu estado entre
//! pontos de suspensão. Este módulo calcula os offsets de cada variável no
//! buffer de estado.

use std::collections::HashMap;

/// Offset de uma variável no buffer de estado
#[derive(Debug, Clone, Copy)]
pub struct VarOffset {
    /// Offset em bytes a partir do início do buffer
    pub offset: usize,
    /// Tamanho em bytes
    pub size: usize,
    /// Alinhamento necessário
    pub align: usize,
}

/// Layout completo do estado de uma Action
#[derive(Debug)]
pub struct ActionStateLayout {
    /// Nome da Action
    pub name: String,
    /// Tamanho total do buffer de estado
    pub total_size: usize,
    /// Alinhamento máximo necessário
    pub max_align: usize,
    /// Offsets das variáveis locais
    pub var_offsets: HashMap<String, VarOffset>,
    /// Número de stages (pontos de suspensão + 1)
    pub num_stages: usize,
}

impl ActionStateLayout {
    /// Cria um novo layout vazio
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            total_size: 8, // Stage inicial (1 byte) + padding (7 bytes)
            max_align: 8,
            var_offsets: HashMap::new(),
            num_stages: 1, // Pelo menos o stage inicial
        }
    }

    /// Adiciona uma variável ao layout e retorna seu offset
    pub fn add_var(&mut self, name: &str, size: usize, align: usize) -> usize {
        // Alinha o offset atual
        let aligned_offset = Self::align_up(self.total_size, align);

        let var_offset = VarOffset {
            offset: aligned_offset,
            size,
            align,
        };

        self.var_offsets.insert(name.to_string(), var_offset);
        self.total_size = aligned_offset + size;

        if align > self.max_align {
            self.max_align = align;
        }

        aligned_offset
    }

    /// Adiciona uma variável do tipo i64
    pub fn add_i64(&mut self, name: &str) -> usize {
        self.add_var(name, 8, 8)
    }

    /// Adiciona uma variável do tipo f64
    pub fn add_f64(&mut self, name: &str) -> usize {
        self.add_var(name, 8, 8)
    }

    /// Adiciona uma variável do tipo i8 (bool)
    pub fn add_i8(&mut self, name: &str) -> usize {
        self.add_var(name, 1, 1)
    }

    /// Adiciona um ponteiro (8 bytes na arquitetura atual)
    pub fn add_ptr(&mut self, name: &str) -> usize {
        self.add_var(name, 8, 8)
    }

    /// Adiciona um novo stage (ponto de suspensão)
    pub fn add_stage(&mut self) -> usize {
        let stage_id = self.num_stages;
        self.num_stages += 1;
        stage_id
    }

    /// Retorna o offset do stage atual (no byte 0)
    pub fn stage_offset(&self) -> usize {
        0
    }

    /// Retorna o offset de uma variável pelo nome
    pub fn get_var_offset(&self, name: &str) -> Option<usize> {
        self.var_offsets.get(name).map(|v| v.offset)
    }

    /// Alinha um valor para cima
    fn align_up(offset: usize, align: usize) -> usize {
        (offset + align - 1) & !(align - 1)
    }

    /// Finaliza o layout, ajustando o tamanho total ao alinhamento máximo
    pub fn finalize(&mut self) {
        self.total_size = Self::align_up(self.total_size, self.max_align);
    }
}

/// Identifica pontos de suspensão no código de uma Action
#[derive(Debug, Clone)]
pub enum SuspensionPoint {
    /// Receção de canal: `<! channel`
    ChannelRecv {
        /// Variável onde o resultado será armazenado
        var_name: String,
        /// Expressão do canal
        channel_expr: String,
        /// Stage antes da suspensão
        stage_before: usize,
        /// Stage após a retomada
        stage_after: usize,
    },
    /// Envio para canal: `!> channel value`
    ChannelSend {
        /// Expressão do canal
        channel_expr: String,
        /// Expressão do valor
        value_expr: String,
        /// Stage antes da suspensão
        stage_before: usize,
        /// Stage após a retomada
        stage_after: usize,
    },
    /// Sleep: `sleep! millis`
    Sleep {
        /// Milissegundos
        millis_expr: String,
        /// Stage antes da suspensão
        stage_before: usize,
        /// Stage após a retomada
        stage_after: usize,
    },
}

/// Analisa uma Action e retorna seus pontos de suspensão
pub struct ActionAnalyzer {
    /// Pontos de suspensão encontrados
    pub suspension_points: Vec<SuspensionPoint>,
    /// Contador de stages
    pub current_stage: usize,
}

impl ActionAnalyzer {
    pub fn new() -> Self {
        Self {
            suspension_points: Vec::new(),
            current_stage: 0,
        }
    }

    /// Registra um novo ponto de suspensão e retorna o novo stage
    pub fn add_suspension(&mut self, point: SuspensionPoint) -> usize {
        let next_stage = self.current_stage + 1;
        self.suspension_points.push(point);
        self.current_stage = next_stage;
        next_stage
    }

    /// Retorna o stage atual
    pub fn stage(&self) -> usize {
        self.current_stage
    }

    /// Avança para o próximo stage e retorna o anterior
    pub fn next_stage(&mut self) -> usize {
        let current = self.current_stage;
        self.current_stage += 1;
        current
    }
}

impl Default for ActionAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_basic() {
        let mut layout = ActionStateLayout::new("test_action");

        // Stage inicial está no offset 0
        assert_eq!(layout.stage_offset(), 0);

        // Adiciona variáveis
        let off1 = layout.add_i64("x");
        let off2 = layout.add_i64("y");
        let off3 = layout.add_i8("flag");

        // Variáveis começam após o stage (offset 8)
        assert_eq!(off1, 8);
        assert_eq!(off2, 16);
        // flag está alinhado, mas pode estar em 24
        assert!(off3 >= 24);

        layout.finalize();
        assert!(layout.total_size >= 25);
    }

    #[test]
    fn test_layout_with_stages() {
        let mut layout = ActionStateLayout::new("async_action");

        layout.add_i64("result");
        layout.add_stage(); // Stage 1
        layout.add_i64("temp");
        layout.add_stage(); // Stage 2

        assert_eq!(layout.num_stages, 3); // stage 0 (init) + 2 stages
    }
}