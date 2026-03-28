pub type Span = std::ops::Range<usize>;
pub type Spanned<T> = (T, Span);

#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
    Simple(String),
    Generic(String, Vec<Spanned<TypeRef>>),
    Function(Vec<Spanned<TypeRef>>, Box<Spanned<TypeRef>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ident(String),
    Int(String),
    Float(String),
    String(String),
    Hole,
    ActionCall(String),
    ChannelSend,
    ChannelRecv,
    ChannelRecvNonBlock,
    Try(Box<Spanned<Expr>>),
    Tuple(Vec<Spanned<Expr>>),
    List(Vec<Spanned<Expr>>),
    #[allow(dead_code)]
    Array(Vec<Spanned<Expr>>),
    Sequence(Vec<Spanned<Expr>>), 
    Lambda(Vec<Spanned<Pattern>>, Box<Spanned<Expr>>, Vec<Spanned<Expr>>), 
    WithDecl(String, Box<Spanned<Expr>>), 
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Expr), // Int, Float, String
    Ident(String),
    Tuple(Vec<Spanned<Pattern>>),
    List(Vec<Spanned<Pattern>>),
    Sequence(Vec<Spanned<Pattern>>), // "Constructor arg1 arg2"
    Hole,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchArm {
    Pattern(Spanned<Pattern>, Vec<Spanned<Stmt>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let(Spanned<Pattern>, Spanned<Expr>), // let (a, b) = ...
    Var(String, Spanned<Expr>),
    Loop(Vec<Spanned<Stmt>>),
    For(String, Spanned<Expr>, Vec<Spanned<Stmt>>),
    Match(Spanned<Expr>, Vec<MatchArm>),
    Expr(Spanned<Expr>),
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariantData {
    Unit,
    Type(Spanned<TypeRef>),
    FixedValue(Expr),
    Predicate(Expr), // ex: < _ 18.5
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub data: VariantData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Data(String, Vec<String>), 
    Enum(String, Vec<Variant>),
    Interface(String, Vec<String>, Vec<Spanned<TopLevel>>), 
    Implements(String, String, Vec<Spanned<TopLevel>>), 
    Export(Vec<String>),
    Import(String, Option<String>),
    Signature(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>), 
    LambdaDef(Vec<Spanned<Pattern>>, Spanned<Expr>, Vec<Spanned<Expr>>), 
    ActionDef(String, Vec<(String, Spanned<TypeRef>)>, Spanned<TypeRef>, Vec<Spanned<Stmt>>),
    Alias(String, String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub declarations: Vec<Spanned<TopLevel>>,
}
