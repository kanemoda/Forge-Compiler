//! Operator enums for AST expression nodes.
//!
//! Separated from `ast.rs` so the file stays focused on tree structure
//! while this module holds the flat operator tables.  Every operator in
//! C17 that has its own precedence level or associativity gets a variant.

/// Binary infix operators (arithmetic, bitwise, logical, relational).
///
/// Assignment operators live in [`AssignOp`] because they have different
/// associativity and semantic rules.  The comma operator is modelled as
/// [`Expr::Comma`](super::ast::Expr::Comma) rather than a binary op.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
    /// `&&`
    LogAnd,
    /// `||`
    LogOr,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

/// Unary prefix operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `++expr`
    PreIncrement,
    /// `--expr`
    PreDecrement,
    /// `&expr` (address-of)
    AddrOf,
    /// `*expr` (dereference)
    Deref,
    /// `+expr`
    Plus,
    /// `-expr`
    Minus,
    /// `~expr`
    BitNot,
    /// `!expr`
    LogNot,
}

/// Unary postfix operators.
///
/// Postfix `++` and `--` are separated from prefix because they have
/// different precedence and (in C) different semantics (the value
/// before vs after the side-effect).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostfixOp {
    /// `expr++`
    PostIncrement,
    /// `expr--`
    PostDecrement,
}

/// Assignment operators.
///
/// Right-associative and lower precedence than all binary ops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=`
    AddAssign,
    /// `-=`
    SubAssign,
    /// `*=`
    MulAssign,
    /// `/=`
    DivAssign,
    /// `%=`
    ModAssign,
    /// `&=`
    BitAndAssign,
    /// `|=`
    BitOrAssign,
    /// `^=`
    BitXorAssign,
    /// `<<=`
    ShlAssign,
    /// `>>=`
    ShrAssign,
}
