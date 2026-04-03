pub type Span = std::ops::Range<usize>;
pub type Spanned<T> = (T, Span);

#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
    Simple(String),
    TypeVar(String),
    Generic(String, Vec<Spanned<TypeRef>>),
    Function(Vec<Spanned<TypeRef>>, Box<Spanned<TypeRef>>),
    Refined(Box<Spanned<TypeRef>>, Vec<Spanned<Expr>>),
    Variadic(Box<Spanned<TypeRef>>),
    Const(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ident(String),
    Int(String),
    Float(String),
    String(String),
    Hole,
    ActionCall(String, Vec<Spanned<Expr>>),
    ChannelSend,
    ChannelRecv,
    ChannelRecvNonBlock,
    Try(Box<Spanned<Expr>>),
    ExplicitApp(Box<Spanned<Expr>>), // Operador $
    Pipe(Box<Spanned<Expr>>, Box<Spanned<Expr>>), // Operador |>
    Tuple(Vec<Spanned<Expr>>),
    List(Vec<Spanned<Expr>>),
    #[allow(dead_code)]
    Array(Vec<Vec<Spanned<Expr>>>),
    Sequence(Vec<Spanned<Expr>>), 
    Guard(Vec<(String, Spanned<Expr>)>, Box<Spanned<Expr>>),
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
pub struct SelectArm {
    pub operation: Spanned<Expr>,
    pub binding: Option<Spanned<Pattern>>,
    pub body: Vec<Spanned<Stmt>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let(Spanned<Pattern>, Spanned<Expr>), // let (a, b) = ...
    Var(String, Spanned<Expr>),
    Loop(Vec<Spanned<Stmt>>),
    For(String, Spanned<Expr>, Vec<Spanned<Stmt>>),
    Match(Spanned<Expr>, Vec<MatchArm>),
    Select(Vec<SelectArm>, Option<(Spanned<Expr>, Vec<Spanned<Stmt>>)>),
    ActionAssign(Spanned<Expr>, Spanned<Expr>), // x! = expr
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
pub enum DirectiveArgs {
    Positional(Vec<Spanned<Expr>>),
    Named(Vec<(String, Spanned<Expr>)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub name: String,
    pub args: DirectiveArgs,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DataDef {
    Struct(Vec<String>),
    Refined(Spanned<TypeRef>, Vec<Spanned<Expr>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Data(String, DataDef, Vec<Spanned<Directive>>), 
    Enum(String, Vec<Variant>, Vec<Spanned<Directive>>),
    Interface(String, Vec<String>, Vec<Spanned<TopLevel>>, Vec<Spanned<Directive>>), 
    Implements(String, String, Vec<Spanned<TopLevel>>), 
    Export(Vec<String>),
    Import(String, Vec<(String, Option<String>)>),
    Signature(String, Vec<Spanned<TypeRef>>, Spanned<TypeRef>, Vec<Spanned<Directive>>), 
    LambdaDef(Vec<Spanned<Pattern>>, Spanned<Expr>, Vec<Spanned<Expr>>, Vec<Spanned<Directive>>), 
    ActionDef(String, Vec<(String, Spanned<TypeRef>)>, Spanned<TypeRef>, Vec<Spanned<Stmt>>, Vec<Spanned<Directive>>),
    Alias(String, String, Vec<Spanned<Directive>>),
    Execution(Spanned<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub declarations: Vec<Spanned<TopLevel>>,
}
