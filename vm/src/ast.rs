//! Syntax trees for Strongtalk methods, blocks, expressions and type annotations.

use num_bigint::BigInt;
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// A class, protocol, alias or generic parameter name (`Int`, `Self`, `E`).
    Name(String),
    /// `Array[Int]`
    Generic(String, Vec<TypeExpr>),
    /// `Int | Str`
    Union(Vec<TypeExpr>),
    /// `[Int, Str, ^Bool]`
    Block(Vec<TypeExpr>, Option<Box<TypeExpr>>),
    /// `Foo class`
    Meta(Box<TypeExpr>),
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeExpr::Name(n) => write!(f, "{}", n),
            TypeExpr::Generic(n, args) => {
                write!(f, "{}[", n)?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, "]")
            }
            TypeExpr::Union(ts) => {
                for (i, a) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", a)?;
                }
                Ok(())
            }
            TypeExpr::Block(args, ret) => {
                write!(f, "[")?;
                let mut first = true;
                for a in args {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{}", a)?;
                }
                if let Some(r) = ret {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "^{}", r)?;
                }
                write!(f, "]")
            }
            TypeExpr::Meta(t) => write!(f, "{} class", t),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Lit {
    Nil,
    True,
    False,
    Int(i64),
    Big(BigInt),
    Float(f64),
    Char(char),
    Str(String),
    Sym(String),
    Array(Vec<Lit>),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub pos: usize,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Lit(Lit, usize),
    Var(String, usize),
    Assign(String, Box<Expr>, usize),
    Send { recv: Box<Expr>, sel: String, args: Vec<Expr>, pos: usize },
    Cascade { recv: Box<Expr>, msgs: Vec<(String, Vec<Expr>, usize)> },
    Block(Rc<BlockAst>),
    Brace(Vec<Expr>, usize),
    /// `{{receiver primitiveFoo: a ifFail: [...]}}` (receiver optional)
    Prim { recv: Option<Box<Expr>>, sel: String, args: Vec<Expr>, pos: usize },
    /// `guaranteed <T> expr`: the checker takes the value to be a T (no run-time effect).
    Guaranteed(TypeExpr, Box<Expr>, usize),
}

impl Expr {
    pub fn pos(&self) -> usize {
        match self {
            Expr::Lit(_, p) | Expr::Var(_, p) | Expr::Assign(_, _, p) | Expr::Brace(_, p) => *p,
            Expr::Send { pos, .. } | Expr::Prim { pos, .. } => *pos,
            Expr::Guaranteed(_, _, p) => *p,
            Expr::Cascade { recv, .. } => recv.pos(),
            Expr::Block(b) => b.pos,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Return(Expr, usize),
}

#[derive(Debug, Clone)]
pub struct BlockAst {
    pub params: Vec<Var>,
    pub temps: Vec<Var>,
    pub ret: Option<TypeExpr>,
    pub body: Vec<Stmt>,
    pub pos: usize,
}

#[derive(Debug, Clone)]
pub struct MethodAst {
    pub selector: String,
    pub params: Vec<Var>,
    pub ret: Option<TypeExpr>,
    pub temps: Vec<Var>,
    pub body: Vec<Stmt>,
    /// Byte offset where the header ends (for showing signatures).
    pub header_end: usize,
}

impl MethodAst {
    /// The method header as Strongtalk prints it: `at: i <Int> put: v <E> ^<E>`.
    pub fn signature(&self) -> String {
        let mut s = String::new();
        let parts: Vec<&str> = if self.params.is_empty() {
            vec![self.selector.as_str()]
        } else if !self.selector.ends_with(':') {
            vec![self.selector.as_str()]
        } else {
            self.selector.split_inclusive(':').collect()
        };
        if self.params.is_empty() {
            s.push_str(&self.selector);
        } else {
            for (i, p) in self.params.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                s.push_str(parts.get(i).copied().unwrap_or(""));
                s.push(' ');
                s.push_str(&p.name);
                if let Some(t) = &p.ty {
                    s.push_str(&format!(" <{}>", t));
                }
            }
        }
        if let Some(r) = &self.ret {
            s.push_str(&format!(" ^<{}>", r));
        }
        s
    }
}
