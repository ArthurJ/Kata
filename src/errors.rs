use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum KataError {
    TypeError(String),
    PurityError(String),
    OrphanRuleError(String),
    TcoError(String),
    NameError(String),
    AmbiguityError(String),
    ComptimeError(String),
    ExhaustivenessError(String),
    SyntaxError(String),
    GenericError(String),
}

impl fmt::Display for KataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeError(m) => write!(f, "TypeError: {}", m),
            Self::PurityError(m) => write!(f, "PurityError: {}", m),
            Self::OrphanRuleError(m) => write!(f, "OrphanRuleError: {}", m),
            Self::TcoError(m) => write!(f, "TcoError: {}", m),
            Self::NameError(m) => write!(f, "NameError: {}", m),
            Self::AmbiguityError(m) => write!(f, "AmbiguityError: {}", m),
            Self::ComptimeError(m) => write!(f, "ComptimeError: {}", m),
            Self::ExhaustivenessError(m) => write!(f, "ExhaustivenessError: {}", m),
            Self::SyntaxError(m) => write!(f, "SyntaxError: {}", m),
            Self::GenericError(m) => write!(f, "GenericError: {}", m),
        }
    }
}

impl KataError {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::TypeError(_) => "TypeError",
            Self::PurityError(_) => "PurityError",
            Self::OrphanRuleError(_) => "OrphanRuleError",
            Self::TcoError(_) => "TcoError",
            Self::NameError(_) => "NameError",
            Self::AmbiguityError(_) => "AmbiguityError",
            Self::ComptimeError(_) => "ComptimeError",
            Self::ExhaustivenessError(_) => "ExhaustivenessError",
            Self::SyntaxError(_) => "SyntaxError",
            Self::GenericError(_) => "GenericError",
        }
    }
}