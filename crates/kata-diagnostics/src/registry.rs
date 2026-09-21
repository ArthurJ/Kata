//! Registry de códigos de diagnóstico — metadados de severity e ajustabilidade.
//!
//! Cada diagnóstico do compilador tem um código namespaced (ex:
//! `type.mismatch`, `parse.unexpected_token`). O registry mapeia cada
//! código ao seu severity default (`Deny`/`Warn`/`Allow`) e se é
//! ajustável via pragma `#!allow`/`#!warn`/`#!deny`.
//!
//! Os 38 diagnósticos existentes são implicitamente `Deny` + não
//! ajustáveis. Apenas diagnósticos explicitamente marcados no
//! `ADJUSTABLE` set são ajustáveis.

use std::collections::HashMap;

use kata_ast::DiagnosticLevel;

// ── Severity default ───────────────────────────────────────

/// Severity default de um diagnóstico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultSeverity {
    /// Erro — compilação falha.
    Deny,
    /// Warning — compilação continua.
    Warn,
    /// Silenciado — não emite nada.
    Allow,
}

impl DefaultSeverity {
    fn as_level(self) -> DiagnosticLevel {
        match self {
            DefaultSeverity::Deny => DiagnosticLevel::Deny,
            DefaultSeverity::Warn => DiagnosticLevel::Warn,
            DefaultSeverity::Allow => DiagnosticLevel::Allow,
        }
    }
}

// ── Registry ───────────────────────────────────────────────

/// Metadados de um diagnóstico no registry.
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticMeta {
    pub severity: DefaultSeverity,
    pub severity_adjustable: bool,
}

impl DiagnosticMeta {
    /// Default para os 38 diagnósticos existentes: deny + não ajustável.
    const fn default_deny() -> Self {
        DiagnosticMeta {
            severity: DefaultSeverity::Deny,
            severity_adjustable: false,
        }
    }

    /// Diagnóstico ajustável com severity default deny.
    const fn adjustable_deny() -> Self {
        DiagnosticMeta {
            severity: DefaultSeverity::Deny,
            severity_adjustable: true,
        }
    }
}

/// Registry estático de códigos de diagnóstico.
///
/// Códigos não listados explicitamente são tratados como
/// `Deny + não ajustável` (default dos 38 diagnósticos existentes).
/// Apenas códigos em `ADJUSTABLE_CODES` ou com entrada explícita em
/// `EXPLICIT_ENTRIES` divergem do default.
pub struct DiagnosticRegistry {
    /// Códigos com metadados não-default (ajustáveis ou severity ≠ deny).
    entries: HashMap<&'static str, DiagnosticMeta>,
}

impl DiagnosticRegistry {
    /// Constrói o registry com todos os códigos conhecidos.
    ///
    /// Códigos não ajustáveis não precisam estar no mapa — `lookup`
    /// retorna o default para códigos conhecidos mas ausentes do mapa.
    /// O mapa só precisa conter códigos cujos metadados divergem do
    /// default (deny + não ajustável).
    pub fn new() -> Self {
        let mut entries = HashMap::new();

        // ── Diagnósticos ajustáveis ──
        //
        // Estes são os códigos que o usuário pode silenciar ou
        // rebaixar via `#!allow`/`#!warn`. São diagnósticos que
        // representam estilo, convenção, ou situações ambíguas
        // onde o usuário pode querer deliberadamente ignorar.

        // type.redundant_clause — cláusula de match sombreada. O
        // usuário pode ter cláusulas redundantes intencionais (ex:
        // documentação de caso coberto por otherwise).
        entries.insert("type.redundant_clause", DiagnosticMeta::adjustable_deny());

        // type.incomplete_interface — implements não cobre todos os
        // métodos da interface. PRD-family-extension-errors define
        // este código como controlável. O variant do enum será
        // adicionado quando aquele PRD for implementado; o registry
        // já reconhece o código para que `#!allow` funcione.
        entries.insert(
            "type.incomplete_interface",
            DiagnosticMeta::adjustable_deny(),
        );

        // type.missing_overload — uso de sobrecarga não definida.
        // Não ajustável (erro de uso, não de estilo). Código
        // reconhecido no registry para sugestão de typo, mas
        // metadata = default deny + não ajustável.
        entries.insert("type.missing_overload", DiagnosticMeta::default_deny());

        DiagnosticRegistry { entries }
    }

    /// Lookup dos metadados de um código.
    ///
    /// Retorna `Some` para códigos conhecidos (no mapa ou default).
    /// Retorna `None` para códigos desconhecidos (typo).
    ///
    /// Como distingir "conhecido mas não ajustável" de "desconhecido"?
    /// Códigos conhecidos não ajustáveis não estão no mapa, mas são
    /// válidos. Para evitar listar todos os 38 códigos não ajustáveis,
    /// usamos uma lista de `ALL_KNOWN_CODES` — se o código está na
    /// lista, é conhecido (metadata = default). Se não está, é typo.
    pub fn lookup(&self, code: &str) -> Option<DiagnosticMeta> {
        if let Some(meta) = self.entries.get(code) {
            return Some(*meta);
        }
        if ALL_KNOWN_CODES.contains(&code) {
            return Some(DiagnosticMeta::default_deny());
        }
        None
    }

    /// Sugestão de typo — retorna o código conhecido mais próximo
    /// por distância de Levenshtein (threshold ≤ 3).
    pub fn closest_match(&self, code: &str) -> Option<&'static str> {
        let mut best: Option<(&'static str, usize)> = None;
        for &known in ALL_KNOWN_CODES.iter() {
            let dist = levenshtein(code, known);
            if dist <= 3 {
                match best {
                    None => best = Some((known, dist)),
                    Some((_, bd)) if dist < bd => best = Some((known, dist)),
                    _ => {}
                }
            }
        }
        best.map(|(c, _)| c)
    }

    /// Lista códigos ajustáveis (para sugestão em erro de
    /// não-ajustável).
    pub fn adjustable_codes(&self) -> Vec<&'static str> {
        self.entries
            .iter()
            .filter(|(_, m)| m.severity_adjustable)
            .map(|(c, _)| *c)
            .collect()
    }

    /// Verifica se um pragma é redundante (mesmo nível do default).
    pub fn is_redundant(&self, code: &str, level: &DiagnosticLevel) -> bool {
        match self.lookup(code) {
            None => false,
            Some(meta) => &meta.severity.as_level() == level,
        }
    }
}

impl Default for DiagnosticRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Lista de todos os códigos conhecidos ────────────────────

/// Todos os códigos de diagnóstico conhecidos pelo compilador.
///
/// Inclui os 38 códigos dos enums `FrontendError` e `MiddleError`,
/// mais os códigos de `ResolveError`, mais códigos futuros já
/// registrados no `DiagnosticRegistry` (incomplete_interface,
/// missing_overload).
static ALL_KNOWN_CODES: &[&str] = &[
    // FrontendError (12)
    "parse.unexpected_token",
    "lex.invalid_char",
    "lex.unterminated_string",
    "lex.invalid_number",
    "parse.trailing_tokens",
    "parse.expected_expr",
    "lex.inconsistent_indent",
    "parse.invalid_casing",
    "lex.unterminated_comment",
    "lex.invalid_pipe_limit",
    "parse.reserved_name",
    "parse.nesting_too_deep",
    // MiddleError (26)
    "type.unbound_name",
    "type.mismatch",
    "type.ambiguous_dispatch",
    "type.no_overload",
    "type.no_cross_type_overload",
    "type.duplicate_decl",
    "type.duplicate_constant",
    "type.constant_name_collision",
    "type.arity_mismatch",
    "type.unknown_ffi",
    "type.non_exhaustive_match",
    "type.missing_otherwise",
    "type.redundant_clause",
    "type.lambda_inference_fail",
    "action.recursive",
    "type.unknown_field",
    "type.index_out_of_bounds",
    "type.not_indexable",
    "type.field_access_on_tuple",
    "type.index_access_on_struct",
    "type.channel_in_return",
    "type.cache_type_constraint",
    "constant.lambda_not_serializable",
    "constant.not_comptime",
    "constant.impure",
    "type.family_extension_invalid",
    // ResolveError (12)
    "resolve.unknown_type",
    "resolve.unknown_ffi",
    "resolve.duplicate_signature",
    "resolve.unknown_directive",
    "resolve.invalid_refines",
    "resolve.duplicate_directive",
    "resolve.directive_target_mismatch",
    "resolve.directive_any_conflict",
    "type.unknown_cache_strategy",
    "type.cache_capacity_invalid",
    "resolve.empty_data_no_ffi",
    "type.orphan_impl",
    // Códigos futuros (PRD-family-extension-errors)
    "type.incomplete_interface",
    "type.missing_overload",
];

// ── Levenshtein ────────────────────────────────────────────

/// Distância de Levenshtein entre duas strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

// ── Erros de pragma ─────────────────────────────────────────

use kata_ast::Span;
use thiserror::Error;

/// Wrapper de `Span` para miette (reutiliza o do frontend).
pub(crate) use crate::frontend::MietteSpan;

/// Erro de pragma — diagnostic code desconhecido (typo).
#[derive(Debug, Clone, Error, miette::Diagnostic)]
pub enum PragmaError {
    #[error("código de diagnóstico desconhecido: `{code}`")]
    #[diagnostic(code = "pragma.unknown_code")]
    UnknownCode {
        code: String,
        #[help]
        suggestion: Option<String>,
        #[label("pragma com código desconhecido")]
        span: MietteSpan,
    },

    #[error("`{code}` não é severity-adjustable")]
    #[diagnostic(code = "pragma.not_adjustable")]
    NotAdjustable {
        code: String,
        #[help]
        adjustable: Option<String>,
        #[label("pragma em diagnóstico não-ajustável")]
        span: MietteSpan,
    },

    #[error("pragma redundante: `#!{level}` é o severity default de `{code}`")]
    #[diagnostic(code = "pragma.redundant", severity = "warning")]
    Redundant {
        level: String,
        code: String,
        #[label("pragma redundante")]
        span: MietteSpan,
    },
}

impl PragmaError {
    pub fn span(&self) -> Span {
        match self {
            PragmaError::UnknownCode { span, .. }
            | PragmaError::NotAdjustable { span, .. }
            | PragmaError::Redundant { span, .. } => span.0,
        }
    }
}

// ── PragmaOverrides ─────────────────────────────────────────

/// Mapa de overrides de severity vindos de pragmas `#!`.
///
/// Chave: código de diagnóstico (ex: `type.redundant_clause`).
/// Valor: nível ajustado (`Allow`/`Warn`/`Deny`).
///
/// Construído a partir de `module.pragmas` filtrando
/// `DiagnosticControl`. Usado pelo pipeline para filtrar erros
/// após cada fase.
#[derive(Debug, Clone, Default)]
pub struct PragmaOverrides {
    overrides: HashMap<String, DiagnosticLevel>,
}

impl PragmaOverrides {
    pub fn new() -> Self {
        PragmaOverrides {
            overrides: HashMap::new(),
        }
    }

    pub fn insert(&mut self, code: &str, level: DiagnosticLevel) {
        self.overrides.insert(code.to_string(), level);
    }

    /// Consulta o override para um código.
    ///
    /// Retorna `Some(level)` se há override, `None` se não há.
    pub fn get(&self, code: &str) -> Option<&DiagnosticLevel> {
        self.overrides.get(code)
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}
