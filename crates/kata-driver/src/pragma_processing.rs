//! Processamento de pragmas `#!` — validação, overrides, e filtro de erros.
//!
//! Este módulo orquestra a Fase 2 do PRD-pragma-mechanism:
//!
//! 1. **Validação:** percorre `module.pragmas` e valida cada
//!    `DiagnosticControl` contra o `DiagnosticRegistry`. Produz
//!    erros para códigos inexistentes (typo), diagnósticos não
//!    ajustáveis, e warnings para pragmas redundantes.
//! 2. **Overrides:** constrói `PragmaOverrides` a partir dos
//!    `DiagnosticControl` válidos.
//! 3. **Filtro:** aplica overrides sobre `Vec<Report>` após cada
//!    fase do pipeline — `Allow` silencia, `Warn` rebaixa,
//!    `Deny` mantém.

use kata_ast::{DiagnosticControl, DiagnosticLevel, Pragma};
use kata_diagnostics::{DiagnosticRegistry, MietteSpan, PragmaError, PragmaOverrides};
use miette::{Report, Severity};

use crate::IntoReport;

/// Processa pragmas do módulo: valida e constrói overrides.
///
/// Retorna `(pragma_errors, overrides)`. `pragma_errors` contém
/// erros de validação (código inexistente, não-ajustável) e
/// warnings (pragma redundante). `overrides` contém o mapa
/// de código → nível para filtrar erros do pipeline.
pub fn process_pragmas(
    pragmas: &[Pragma],
    registry: &DiagnosticRegistry,
) -> (Vec<Report>, PragmaOverrides) {
    let mut errors: Vec<Report> = Vec::new();
    let mut overrides = PragmaOverrides::new();

    for pragma in pragmas {
        if let Pragma::DiagnosticControl(dc) = pragma {
            match validate_diagnostic_control(dc, registry) {
                Ok(level) => {
                    overrides.insert(&dc.code, level);
                }
                Err(pragma_err) => {
                    errors.push(pragma_err.into_report_with_source("", None));
                }
            }
        }
    }

    (errors, overrides)
}

/// Valida um `DiagnosticControl` contra o registry.
///
/// Retorna `Ok(level)` se o pragma é válido (e o nível para
/// aplicar). Retorna `Err(PragmaError)` se o código é
/// inexistente, não-ajustável, ou redundante.
fn validate_diagnostic_control(
    dc: &DiagnosticControl,
    registry: &DiagnosticRegistry,
) -> Result<DiagnosticLevel, PragmaError> {
    let meta = match registry.lookup(&dc.code) {
        Some(meta) => meta,
        None => {
            let suggestion = registry.closest_match(&dc.code);
            return Err(PragmaError::UnknownCode {
                code: dc.code.clone(),
                suggestion: suggestion.map(|s| format!("did you mean `{s}`?")),
                span: MietteSpan(dc.span),
            });
        }
    };

    if !meta.severity_adjustable {
        let adjustable: Vec<&str> = registry.adjustable_codes();
        let suggestion = if adjustable.is_empty() {
            None
        } else {
            Some(format!("adjustable diagnostics: {}", adjustable.join(", ")))
        };
        return Err(PragmaError::NotAdjustable {
            code: dc.code.clone(),
            adjustable: suggestion,
            span: MietteSpan(dc.span),
        });
    }

    // Verificar redundância (mesmo nível do default).
    if registry.is_redundant(&dc.code, &dc.level) {
        let level_str = match dc.level {
            DiagnosticLevel::Allow => "allow",
            DiagnosticLevel::Warn => "warn",
            DiagnosticLevel::Deny => "deny",
        };
        return Err(PragmaError::Redundant {
            level: level_str.to_string(),
            code: dc.code.clone(),
            span: MietteSpan(dc.span),
        });
    }

    Ok(dc.level.clone())
}

/// Filtra `Vec<Report>` aplicando `PragmaOverrides`.
///
/// Retorna `(errors, warnings)`:
/// - `Allow` em ajustável → remove (silencia).
/// - `Warn` em ajustável → rebaixa para warning (separado de errors).
/// - `Deny` em ajustável → mantém como erro.
/// - Sem override → mantém como erro (default).
///
/// Warnings são separados de errors para que o pipeline possa
/// imprimi-los sem abortar a compilação.
pub fn filter_errors(
    errors: Vec<Report>,
    overrides: &PragmaOverrides,
) -> (Vec<Report>, Vec<Report>) {
    if overrides.is_empty() {
        return (errors, Vec::new());
    }

    let mut errs = Vec::with_capacity(errors.len());
    let mut warns = Vec::new();
    for report in errors {
        let code = extract_code(&report);
        match code.as_deref() {
            Some(code_str) => {
                if let Some(level) = overrides.get(code_str) {
                    match level {
                        DiagnosticLevel::Allow => {
                            // Silencia — não adiciona.
                            continue;
                        }
                        DiagnosticLevel::Warn => {
                            // Rebaixa para warning.
                            warns.push(rewrap_as_warning(report));
                        }
                        DiagnosticLevel::Deny => {
                            // Mantém como erro.
                            errs.push(report);
                        }
                    }
                } else {
                    errs.push(report);
                }
            }
            None => {
                // Erro sem código — não afetado por pragmas.
                errs.push(report);
            }
        }
    }
    (errs, warns)
}

/// Extrai o código de diagnóstico de um `miette::Report`.
///
/// O `Diagnostic::code()` retorna `Option<Box<dyn Display>>`.
/// Convertendo para string obtemos o código (ex: "type.mismatch").
fn extract_code(report: &Report) -> Option<String> {
    report
        .code()
        .map(|c| c.to_string())
        .filter(|s| !s.is_empty())
}

/// Re-envelopa um `Report` como warning, preservando source code.
///
/// Como `miette::Report` não implementa `Diagnostic` diretamente,
/// usamos `ReportAsWarning` — um wrapper que delega para o
/// `dyn Diagnostic` interno (acessível via `AsRef`) mas
/// sobrescreve `severity()` para `Warning`.
fn rewrap_as_warning(report: Report) -> Report {
    let wrapped = ReportAsWarning::new(report);
    Report::new(wrapped)
}

/// Wrapper que delega todos os métodos de `Diagnostic` para o
/// `Report` interno, exceto `severity()` que retorna `Warning`.
struct ReportAsWarning {
    inner: Report,
}

impl ReportAsWarning {
    fn new(inner: Report) -> Self {
        ReportAsWarning { inner }
    }

    fn as_diag(&self) -> &dyn miette::Diagnostic {
        self.inner.as_ref()
    }
}

impl std::fmt::Debug for ReportAsWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportAsWarning")
            .field("inner", &self.inner)
            .finish()
    }
}

impl std::fmt::Display for ReportAsWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl std::error::Error for ReportAsWarning {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.as_diag().source()
    }
}

impl miette::Diagnostic for ReportAsWarning {
    fn severity(&self) -> Option<Severity> {
        Some(Severity::Warning)
    }

    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.as_diag().code()
    }

    fn url<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.as_diag().url()
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.as_diag().source_code()
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        self.as_diag().labels()
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn miette::Diagnostic> + 'a>> {
        // Não podemos delegar related() seguramente porque o lifetime
        // do borrow de self.inner.as_ref() não estende 'a.
        // Warnings rebaixados não precisam de related().
        None
    }

    fn diagnostic_source(&self) -> Option<&dyn miette::Diagnostic> {
        self.as_diag().diagnostic_source()
    }

    fn help<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.as_diag().help()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kata_ast::Span;

    fn make_dc(level: DiagnosticLevel, code: &str) -> DiagnosticControl {
        DiagnosticControl {
            level,
            code: code.to_string(),
            span: Span::zero(),
        }
    }

    #[test]
    fn t7_adjustable_responds_to_allow() {
        let registry = DiagnosticRegistry::new();
        let dc = make_dc(DiagnosticLevel::Allow, "type.redundant_clause");
        let (errors, overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        assert!(errors.is_empty(), "no pragma errors expected");
        assert_eq!(
            overrides.get("type.redundant_clause"),
            Some(&DiagnosticLevel::Allow)
        );
    }

    #[test]
    fn t7_adjustable_responds_to_warn() {
        let registry = DiagnosticRegistry::new();
        let dc = make_dc(DiagnosticLevel::Warn, "type.redundant_clause");
        let (errors, overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        assert!(errors.is_empty());
        assert_eq!(
            overrides.get("type.redundant_clause"),
            Some(&DiagnosticLevel::Warn)
        );
    }

    #[test]
    fn t7_adjustable_responds_to_deny() {
        let registry = DiagnosticRegistry::new();
        let dc = make_dc(DiagnosticLevel::Deny, "type.redundant_clause");
        // deny is the default for type.redundant_clause → redundant warning
        let (errors, _overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        // Should produce a redundant warning
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn t8_not_adjustable_rejects_allow() {
        let registry = DiagnosticRegistry::new();
        let dc = make_dc(DiagnosticLevel::Allow, "type.mismatch");
        let (errors, _overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        assert_eq!(errors.len(), 1);
        // Should be NotAdjustable error
        let msg = errors[0].to_string();
        assert!(msg.contains("não é severity-adjustable"));
    }

    #[test]
    fn t9_unknown_code_with_suggestion() {
        let registry = DiagnosticRegistry::new();
        let dc = make_dc(DiagnosticLevel::Allow, "type.incomplte_interface");
        let (errors, _overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        assert_eq!(errors.len(), 1);
        let msg = errors[0].to_string();
        assert!(msg.contains("desconhecido"));
        // Should suggest the correct code
        let help = errors[0].help().map(|h| h.to_string()).unwrap_or_default();
        assert!(help.contains("type.incomplete_interface"));
    }

    #[test]
    fn t10_redundant_deny_on_deny_default() {
        let registry = DiagnosticRegistry::new();
        // type.redundant_clause is deny by default → #!deny is redundant
        let dc = make_dc(DiagnosticLevel::Deny, "type.redundant_clause");
        let (errors, _overrides) = process_pragmas(&[Pragma::DiagnosticControl(dc)], &registry);
        assert_eq!(errors.len(), 1);
        let msg = errors[0].to_string();
        assert!(msg.contains("redundante"));
    }

    #[test]
    fn filter_allow_silences_error() {
        let mut overrides = PragmaOverrides::new();
        overrides.insert("type.redundant_clause", DiagnosticLevel::Allow);

        let report = miette::Report::new(crate::test_utils::TestDiagnostic::new(
            "type.redundant_clause",
            "cláusula redundante",
        ));
        let (errs, warns) = filter_errors(vec![report], &overrides);
        assert!(errs.is_empty(), "Allow should silence the error");
        assert!(warns.is_empty(), "Allow should not produce warnings");
    }

    #[test]
    fn filter_warn_downgrades_to_warning() {
        let mut overrides = PragmaOverrides::new();
        overrides.insert("type.redundant_clause", DiagnosticLevel::Warn);

        let report = miette::Report::new(crate::test_utils::TestDiagnostic::new(
            "type.redundant_clause",
            "cláusula redundante",
        ));
        let (errs, warns) = filter_errors(vec![report], &overrides);
        assert!(errs.is_empty(), "Warn should not keep errors");
        assert_eq!(warns.len(), 1);
        assert_eq!(warns[0].severity(), Some(Severity::Warning));
    }

    #[test]
    fn filter_no_override_preserves() {
        let overrides = PragmaOverrides::new();
        let report = miette::Report::new(crate::test_utils::TestDiagnostic::new(
            "type.mismatch",
            "tipos incompatíveis",
        ));
        let (errs, warns) = filter_errors(vec![report], &overrides);
        assert_eq!(errs.len(), 1);
        assert!(warns.is_empty());
    }
}
