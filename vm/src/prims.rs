//! VM primitives, called from Smalltalk with `{{receiver primitiveName: arg ifFail: [:err | ...]}}`.
//! Names follow the original Strongtalk VM where it had an equivalent.

use crate::interp::{Event, Interp, Unwind, R, CanvasBuf};
use crate::object::*;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero, One, Num};
use std::collections::HashMap;
use std::rc::Rc;

pub enum PErr {
    Fail(&'static str),
    Unwind(Unwind),
}
impl From<Unwind> for PErr {
    fn from(u: Unwind) -> Self {
        PErr::Unwind(u)
    }
}
pub type PR = Result<Value, PErr>;
pub type PrimFn = fn(&mut Interp, &Value, &[Value]) -> PR;

// Error symbols as the original VM registers them (sub_40f100, slots 0x576490...).
const WRONG_TYPE: &str = "FirstArgumentHasWrongType";
const SECOND_WRONG: &str = "SecondArgumentHasWrongType";
const THIRD_WRONG: &str = "ThirdArgumentHasWrongType";
const RECV_TYPE: &str = "ReceiverHasWrongType";
const BOUNDS: &str = "OutOfBounds";
const DIV0: &str = "DivisionByZero";

pub fn call(it: &mut Interp, name: &Sym, rcv: Value, mut args: Vec<Value>) -> R {
    let has_fail = name.ends_with("ifFail:") || name.ends_with("IfFail:");
    let fail = if has_fail { args.pop() } else { None };
    let f = it.prims.get(&**name).copied();
    match f {
        None => it.signal_error("Error", &format!("primitive {} is not defined", name)),
        Some(f) => match f(it, &rcv, &args) {
            Ok(v) => Ok(v),
            Err(PErr::Unwind(u)) => Err(u),
            Err(PErr::Fail(code)) => match fail {
                Some(fb) => {
                    let s = Value::Sym(it.intern(code));
                    it.call_flexible(&fb, vec![s])
                }
                None => it.signal_error("Error", &format!("primitive {} failed: {}", name, code)),
            },
        },
    }
}

// ---------- numbers ----------

#[derive(Clone)]
enum N {
    I(i64),
    B(BigInt),
    F(f64),
}

fn num(v: &Value) -> Option<N> {
    match v {
        Value::Int(i) => Some(N::I(*i)),
        Value::Big(b) => Some(N::B((**b).clone())),
        Value::Float(f) => Some(N::F(*f)),
        _ => None,
    }
}

fn to_big(n: &N) -> BigInt {
    match n {
        N::I(i) => BigInt::from(*i),
        N::B(b) => b.clone(),
        N::F(f) => BigInt::from(*f as i64),
    }
}

fn to_f(n: &N) -> f64 {
    match n {
        N::I(i) => *i as f64,
        N::B(b) => b.to_f64().unwrap_or(f64::INFINITY),
        N::F(f) => *f,
    }
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Quo,
    Rem,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    Divide,
}

fn arith(op: Op, a: &Value, b: &Value) -> PR {
    let x = num(a).ok_or(PErr::Fail(RECV_TYPE))?;
    let y = num(b).ok_or(PErr::Fail(WRONG_TYPE))?;
    let float = matches!(x, N::F(_)) || matches!(y, N::F(_));
    if float {
        let (p, q) = (to_f(&x), to_f(&y));
        return Ok(match op {
            Op::Add => Value::Float(p + q),
            Op::Sub => Value::Float(p - q),
            Op::Mul => Value::Float(p * q),
            Op::Divide => {
                if q == 0.0 {
                    return Err(PErr::Fail(DIV0));
                }
                Value::Float(p / q)
            }
            Op::Div => {
                if q == 0.0 {
                    return Err(PErr::Fail(DIV0));
                }
                float_to_int((p / q).floor())
            }
            Op::Mod => {
                if q == 0.0 {
                    return Err(PErr::Fail(DIV0));
                }
                Value::Float(p - (p / q).floor() * q)
            }
            Op::Quo => {
                if q == 0.0 {
                    return Err(PErr::Fail(DIV0));
                }
                float_to_int((p / q).trunc())
            }
            Op::Rem => {
                if q == 0.0 {
                    return Err(PErr::Fail(DIV0));
                }
                Value::Float(p % q)
            }
            Op::Lt => Value::Bool(p < q),
            Op::Gt => Value::Bool(p > q),
            Op::Le => Value::Bool(p <= q),
            Op::Ge => Value::Bool(p >= q),
            Op::Eq => Value::Bool(p == q),
            Op::Ne => Value::Bool(p != q),
        });
    }
    if let (N::I(p), N::I(q)) = (&x, &y) {
        let (p, q) = (*p, *q);
        let r = match op {
            Op::Add => p.checked_add(q).map(Value::int),
            Op::Sub => p.checked_sub(q).map(Value::int),
            Op::Mul => p.checked_mul(q).map(Value::int),
            Op::Div => {
                if q == 0 {
                    return Err(PErr::Fail(DIV0));
                }
                Some(Value::int(Integer::div_floor(&p, &q)))
            }
            Op::Mod => {
                if q == 0 {
                    return Err(PErr::Fail(DIV0));
                }
                Some(Value::int(Integer::mod_floor(&p, &q)))
            }
            Op::Quo => {
                if q == 0 {
                    return Err(PErr::Fail(DIV0));
                }
                Some(Value::int(p / q))
            }
            Op::Rem => {
                if q == 0 {
                    return Err(PErr::Fail(DIV0));
                }
                Some(Value::int(p % q))
            }
            Op::Lt => Some(Value::Bool(p < q)),
            Op::Gt => Some(Value::Bool(p > q)),
            Op::Le => Some(Value::Bool(p <= q)),
            Op::Ge => Some(Value::Bool(p >= q)),
            Op::Eq => Some(Value::Bool(p == q)),
            Op::Ne => Some(Value::Bool(p != q)),
            Op::Divide => return Err(PErr::Fail(WRONG_TYPE)),
        };
        if let Some(v) = r {
            return Ok(v);
        }
    }
    let (p, q) = (to_big(&x), to_big(&y));
    let zero = q.is_zero();
    Ok(match op {
        Op::Add => Value::big(p + q),
        Op::Sub => Value::big(p - q),
        Op::Mul => Value::big(p * q),
        Op::Div => {
            if zero {
                return Err(PErr::Fail(DIV0));
            }
            Value::big(Integer::div_floor(&p, &q))
        }
        Op::Mod => {
            if zero {
                return Err(PErr::Fail(DIV0));
            }
            Value::big(Integer::mod_floor(&p, &q))
        }
        Op::Quo => {
            if zero {
                return Err(PErr::Fail(DIV0));
            }
            Value::big(&p / &q)
        }
        Op::Rem => {
            if zero {
                return Err(PErr::Fail(DIV0));
            }
            Value::big(&p % &q)
        }
        Op::Lt => Value::Bool(p < q),
        Op::Gt => Value::Bool(p > q),
        Op::Le => Value::Bool(p <= q),
        Op::Ge => Value::Bool(p >= q),
        Op::Eq => Value::Bool(p == q),
        Op::Ne => Value::Bool(p != q),
        Op::Divide => return Err(PErr::Fail(WRONG_TYPE)),
    })
}

fn float_to_int(f: f64) -> Value {
    if f.is_finite() && f.abs() < 9.0e15 {
        Value::int(f as i64)
    } else if f.is_finite() {
        Value::big(BigInt::from_str_radix(&format!("{:.0}", f), 10).unwrap_or_default())
    } else {
        Value::Float(f)
    }
}

fn int_arg(v: &Value) -> Result<i64, PErr> {
    match v {
        Value::Int(i) => Ok(*i),
        _ => Err(PErr::Fail(WRONG_TYPE)),
    }
}

fn bits(op: char, a: &Value, b: &Value) -> PR {
    if op == '<' {
        // primitiveBitShift:ifFail: (0x4c3170) on a SmallInteger
        if let Value::Int(x) = a {
            let n = match b {
                Value::Int(n) => *n,
                _ => return Err(PErr::Fail(WRONG_TYPE)),
            };
            if n > 0 {
                // shifting left by 29 or more, or out of the 30-bit range, overflows
                if n >= 29 {
                    return Err(PErr::Fail("SmallIntegerOverflow"));
                }
                let r = x << n;
                if !(SMI_MIN..=SMI_MAX).contains(&r) {
                    return Err(PErr::Fail("SmallIntegerOverflow"));
                }
                return Ok(Value::Int(r));
            }
            // right shifts are clamped to 29 places (cmp -0x1d) and round down (sar)
            let s = (-n).min(29);
            return Ok(Value::Int(x >> s));
        }
    }
    let x = match num(a) {
        Some(N::F(_)) | None => return Err(PErr::Fail(RECV_TYPE)),
        Some(n) => to_big(&n),
    };
    let y = match num(b) {
        Some(N::F(_)) | None => return Err(PErr::Fail(WRONG_TYPE)),
        Some(n) => to_big(&n),
    };
    Ok(Value::big(match op {
        '&' => x & y,
        '|' => x | y,
        '^' => x ^ y,
        _ => {
            let s = y.to_i64().ok_or(PErr::Fail(WRONG_TYPE))?;
            if s >= 0 {
                if s > 100_000 {
                    return Err(PErr::Fail("ValueOutOfRange"));
                }
                x << (s as usize)
            } else {
                x >> ((-s) as usize)
            }
        }
    }))
}

/// primitiveFloatPrintString (0x4cc840): `sprintf(buf, "%1.6f", value)`, then trailing '0's
/// are dropped while the character before is not '.', so one digit always follows the point.
pub fn format_float(f: f64) -> String {
    // The original links Microsoft's C library, whose printf writes NaN as "-1.#IND00" (the x87
    // "indefinite" NaN that 0/0 and sqrt(-1) give) and infinity as "1.#INF00"; trimming the
    // zeros as below leaves "-1.#IND" and "1.#INF".
    if f.is_nan() {
        return "-1.#IND".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "1.#INF".into() } else { "-1.#INF".into() };
    }
    let mut s: Vec<u8> = format!("{:.6}", f).into_bytes();
    while s.len() > 1 && s[s.len() - 1] == b'0' && s[s.len() - 2] != b'.' {
        s.pop();
    }
    String::from_utf8(s).unwrap_or_default()
}

fn float_fn(rcv: &Value, f: fn(f64) -> f64) -> PR {
    match num(rcv) {
        Some(n) => Ok(Value::Float(f(to_f(&n)))),
        None => Err(PErr::Fail(RECV_TYPE)),
    }
}

// ---------- strings and indexing ----------

pub fn chars_of(v: &Value) -> Option<Vec<char>> {
    match v {
        Value::Str(s) => Some(s.borrow().clone()),
        Value::Sym(s) => Some(s.chars().collect()),
        _ => None,
    }
}

pub fn string_of(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.borrow().iter().collect()),
        Value::Sym(s) => Some(s.to_string()),
        _ => None,
    }
}

fn str_arg(v: &Value) -> Result<String, PErr> {
    string_of(v).ok_or(PErr::Fail(WRONG_TYPE))
}

fn index_arg(v: &Value, size: usize) -> Result<usize, PErr> {
    match v {
        Value::Int(i) if *i >= 1 && (*i as usize) <= size => Ok(*i as usize - 1),
        Value::Int(_) => Err(PErr::Fail(BOUNDS)),
        _ => Err(PErr::Fail(WRONG_TYPE)),
    }
}

fn size_of(v: &Value) -> Option<usize> {
    match v {
        Value::Str(s) => Some(s.borrow().len()),
        Value::Sym(s) => Some(s.chars().count()),
        Value::Obj(o) => o.indexed.as_ref().map(|x| x.borrow().len()),
        _ => None,
    }
}

fn at(it: &mut Interp, rcv: &Value, args: &[Value]) -> PR {
    let _ = it;
    match rcv {
        Value::Str(s) => {
            let s = s.borrow();
            let i = index_arg(&args[0], s.len())?;
            Ok(Value::Char(s[i]))
        }
        Value::Sym(s) => {
            let c: Vec<char> = s.chars().collect();
            let i = index_arg(&args[0], c.len())?;
            Ok(Value::Char(c[i]))
        }
        Value::Obj(o) => match &o.indexed {
            Some(v) => {
                let v = v.borrow();
                let i = index_arg(&args[0], v.len())?;
                Ok(v[i].clone())
            }
            None => Err(PErr::Fail(RECV_TYPE)),
        },
        _ => Err(PErr::Fail(RECV_TYPE)),
    }
}

fn at_put(it: &mut Interp, rcv: &Value, args: &[Value]) -> PR {
    let _ = it;
    match rcv {
        Value::Str(s) => {
            let c = match &args[1] {
                Value::Char(c) => *c,
                _ => return Err(PErr::Fail(WRONG_TYPE)),
            };
            let mut s = s.borrow_mut();
            let i = index_arg(&args[0], s.len())?;
            s[i] = c;
            Ok(args[1].clone())
        }
        Value::Obj(o) => match &o.indexed {
            Some(v) => {
                let byte = Rc::ptr_eq(&o.class.borrow(), &it.core.byte_array);
                if byte && !matches!(args[1], Value::Int(0..=255)) {
                    return Err(PErr::Fail(WRONG_TYPE));
                }
                let mut v = v.borrow_mut();
                let i = index_arg(&args[0], v.len())?;
                v[i] = args[1].clone();
                Ok(args[1].clone())
            }
            None => Err(PErr::Fail(RECV_TYPE)),
        },
        Value::Sym(_) => Err(PErr::Fail("ReceiverHasWrongType")),
        _ => Err(PErr::Fail(RECV_TYPE)),
    }
}

/// primitiveIndexedByteAt:put:ifFail: (0x4d3210): stores a byte (a SmallInteger 0..255), not a
/// Character, and answers the receiver. Checked in this order: index is a SmallInteger
/// (FirstArgumentHasWrongType), value is a SmallInteger (SecondArgumentHasWrongType), index in
/// range (OutOfBounds), value below 256 as an unsigned compare (ValueOutOfRange).
fn byte_at_put(it: &mut Interp, rcv: &Value, args: &[Value]) -> PR {
    let idx = match args[0] {
        Value::Int(i) => i,
        _ => return Err(PErr::Fail(WRONG_TYPE)),
    };
    let byte = match args[1] {
        Value::Int(b) => b,
        _ => return Err(PErr::Fail(SECOND_WRONG)),
    };
    let size = size_of(rcv).ok_or(PErr::Fail(RECV_TYPE))?;
    if idx < 1 || idx as usize > size {
        return Err(PErr::Fail(BOUNDS));
    }
    if !(0..256).contains(&byte) {
        return Err(PErr::Fail("ValueOutOfRange"));
    }
    let i = idx as usize - 1;
    match rcv {
        Value::Str(s) => s.borrow_mut()[i] = char::from_u32(byte as u32).unwrap_or('?'),
        Value::Obj(o) if o.indexed.is_some() => o.indexed.as_ref().unwrap().borrow_mut()[i] = Value::Int(byte),
        Value::Sym(_) => return Err(PErr::Fail(RECV_TYPE)),
        _ => return Err(PErr::Fail(RECV_TYPE)),
    }
    let _ = it;
    Ok(rcv.clone())
}

fn items_of(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Obj(o) => o.indexed.as_ref().map(|x| x.borrow().clone()),
        Value::Str(s) => Some(s.borrow().iter().map(|c| Value::Char(*c)).collect()),
        Value::Sym(s) => Some(s.chars().map(Value::Char).collect()),
        _ => None,
    }
}

fn same_kind(it: &Interp, like: &Value, items: Vec<Value>) -> Value {
    match like {
        Value::Str(_) | Value::Sym(_) => {
            let cs: Vec<char> = items.iter().map(|v| if let Value::Char(c) = v { *c } else { '?' }).collect();
            Value::from_chars(cs)
        }
        Value::Obj(o) => it.new_indexed(&o.class.borrow().clone(), items),
        _ => it.array(items),
    }
}

fn parse_number(s: &str) -> Option<Value> {
    let t = s.trim();
    let toks = crate::lexer::tokenize(t).ok()?;
    if toks.len() != 2 {
        return None;
    }
    match &toks[0].tok {
        crate::lexer::Tok::Int(i) => Some(Value::int(*i)),
        crate::lexer::Tok::Big(b) => Some(Value::big(b.clone())),
        crate::lexer::Tok::Float(f) => Some(Value::Float(*f)),
        _ => None,
    }
}

/// primitiveMandelbrotAtRe:im:iterate:ifFail: (0x4ccbd0): z starts at c, at most n-1 steps,
/// stopping as soon as x*x + y*y > 4.0 (the constant at 0x54abf8); answers the step count.
fn mandel(re: f64, im: f64, n: i64) -> i64 {
    let (mut x, mut y) = (re, im);
    let (mut x2, mut y2) = (re * re, im * im);
    let limit = n - 1;
    let mut i = 0;
    while i < limit {
        if x2 + y2 > 4.0 {
            break;
        }
        i += 1;
        y = 2.0 * x * y + im;
        x = x2 - y2 + re;
        x2 = x * x;
        y2 = y * y;
    }
    i
}

fn canvas_id(it: &Interp, v: &Value) -> Result<usize, PErr> {
    match v {
        Value::Int(i) if *i >= 0 && (*i as usize) < it.canvases.len() => Ok(*i as usize),
        _ => Err(PErr::Fail(WRONG_TYPE)),
    }
}

pub fn table() -> HashMap<&'static str, PrimFn> {
    let mut t: HashMap<&'static str, PrimFn> = HashMap::new();
    macro_rules! p {
        ($name:expr, $f:expr) => {
            t.insert($name, $f as PrimFn);
        };
    }
    // --- integers and floats
    for (name, op) in [
        ("primitiveAdd:ifFail:", Op::Add),
        ("primitiveSubtract:ifFail:", Op::Sub),
        ("primitiveMultiply:ifFail:", Op::Mul),
        ("primitiveDiv:ifFail:", Op::Div),
        ("primitiveMod:ifFail:", Op::Mod),
        ("primitiveQuo:ifFail:", Op::Quo),
        ("primitiveRemainder:ifFail:", Op::Rem),
        ("primitiveLessThan:ifFail:", Op::Lt),
        ("primitiveGreaterThan:ifFail:", Op::Gt),
        ("primitiveLessThanOrEqual:ifFail:", Op::Le),
        ("primitiveGreaterThanOrEqual:ifFail:", Op::Ge),
        ("primitiveSmallIntegerEqual:ifFail:", Op::Eq),
        ("primitiveSmallIntegerNotEqual:ifFail:", Op::Ne),
        ("primitiveFloatAdd:ifFail:", Op::Add),
        ("primitiveFloatSubtract:ifFail:", Op::Sub),
        ("primitiveFloatMultiply:ifFail:", Op::Mul),
        ("primitiveFloatDivide:ifFail:", Op::Divide),
        ("primitiveFloatMod:ifFail:", Op::Mod),
        ("primitiveFloatLessThan:ifFail:", Op::Lt),
        ("primitiveFloatGreaterThan:ifFail:", Op::Gt),
        ("primitiveFloatLessThanOrEqual:ifFail:", Op::Le),
        ("primitiveFloatGreaterThanOrEqual:ifFail:", Op::Ge),
        ("primitiveFloatEqual:ifFail:", Op::Eq),
        ("primitiveFloatNotEqual:ifFail:", Op::Ne),
    ] {
        let f: PrimFn = match op {
            Op::Add => |_, r, a| arith(Op::Add, r, &a[0]),
            Op::Sub => |_, r, a| arith(Op::Sub, r, &a[0]),
            Op::Mul => |_, r, a| arith(Op::Mul, r, &a[0]),
            Op::Div => |_, r, a| arith(Op::Div, r, &a[0]),
            Op::Mod => |_, r, a| arith(Op::Mod, r, &a[0]),
            Op::Quo => |_, r, a| arith(Op::Quo, r, &a[0]),
            Op::Rem => |_, r, a| arith(Op::Rem, r, &a[0]),
            Op::Lt => |_, r, a| arith(Op::Lt, r, &a[0]),
            Op::Gt => |_, r, a| arith(Op::Gt, r, &a[0]),
            Op::Le => |_, r, a| arith(Op::Le, r, &a[0]),
            Op::Ge => |_, r, a| arith(Op::Ge, r, &a[0]),
            Op::Eq => |_, r, a| arith(Op::Eq, r, &a[0]),
            Op::Ne => |_, r, a| arith(Op::Ne, r, &a[0]),
            Op::Divide => |_, r, a| arith(Op::Divide, r, &a[0]),
        };
        t.insert(name, f);
    }
    p!("primitiveBitAnd:ifFail:", |_, r, a| bits('&', r, &a[0]));
    p!("primitiveBitOr:ifFail:", |_, r, a| bits('|', r, &a[0]));
    p!("primitiveBitXor:ifFail:", |_, r, a| bits('^', r, &a[0]));
    p!("primitiveBitShift:ifFail:", |_, r, a| bits('<', r, &a[0]));
    p!("primitiveAsFloatIfFail:", |_, r, _| match num(r) {
        Some(n) => Ok(Value::Float(to_f(&n))),
        None => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveGcd:ifFail:", |_, r, a| {
        let (x, y) = (num(r).ok_or(PErr::Fail(RECV_TYPE))?, num(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?);
        if matches!(x, N::F(_)) || matches!(y, N::F(_)) {
            return Err(PErr::Fail(WRONG_TYPE));
        }
        Ok(Value::big(to_big(&x).gcd(&to_big(&y))))
    });
    p!("primitiveIntegerPrintBase:ifFail:", |_, r, a| {
        let base = int_arg(&a[0])?;
        if !(2..=36).contains(&base) {
            return Err(PErr::Fail(WRONG_TYPE));
        }
        let n = num(r).ok_or(PErr::Fail(RECV_TYPE))?;
        if let N::F(_) = n {
            return Err(PErr::Fail(RECV_TYPE));
        }
        Ok(Value::str(&to_big(&n).to_str_radix(base as u32).to_uppercase()))
    });
    p!("primitiveIntegerSqrtFloorIfFail:", |_, r, _| match num(r) {
        Some(N::F(_)) | None => Err(PErr::Fail(RECV_TYPE)),
        Some(n) => {
            let b = to_big(&n);
            if b.is_negative() {
                return Err(PErr::Fail("ArgumentIsInvalid"));
            }
            Ok(Value::big(b.sqrt()))
        }
    });
    p!("primitiveIntegerRaisedTo:ifFail:", |_, r, a| {
        let e = int_arg(&a[0])?;
        if !(0..=100_000).contains(&e) {
            return Err(PErr::Fail(WRONG_TYPE));
        }
        let n = num(r).ok_or(PErr::Fail(RECV_TYPE))?;
        if let N::F(f) = n {
            return Ok(Value::Float(f.powi(e as i32)));
        }
        Ok(Value::big(num_traits::pow(to_big(&n), e as usize)))
    });
    p!("primitiveIntegerIsPrimeIfFail:", |_, r, _| {
        let n = int_arg(r).map_err(|_| PErr::Fail(RECV_TYPE))?;
        if n < 2 {
            return Ok(Value::Bool(false));
        }
        let mut i = 2i64;
        while i * i <= n {
            if n % i == 0 {
                return Ok(Value::Bool(false));
            }
            i += 1;
        }
        Ok(Value::Bool(true))
    });
    p!("primitiveFloatSqrtIfFail:", |_, r, _| float_fn(r, f64::sqrt));
    p!("primitiveFloatLnIfFail:", |_, r, _| float_fn(r, f64::ln));
    p!("primitiveFloatLog10IfFail:", |_, r, _| float_fn(r, f64::log10));
    p!("primitiveFloatExpIfFail:", |_, r, _| float_fn(r, f64::exp));
    p!("primitiveFloatSineIfFail:", |_, r, _| float_fn(r, f64::sin));
    p!("primitiveFloatCosineIfFail:", |_, r, _| float_fn(r, f64::cos));
    p!("primitiveFloatTangentIfFail:", |_, r, _| float_fn(r, f64::tan));
    p!("primitiveFloatArcTangentIfFail:", |_, r, _| float_fn(r, f64::atan));
    p!("primitiveFloatArcSineIfFail:", |_, r, _| float_fn(r, f64::asin));
    p!("primitiveFloatArcCosineIfFail:", |_, r, _| float_fn(r, f64::acos));
    p!("primitiveFloatHyperbolicSineIfFail:", |_, r, _| float_fn(r, f64::sinh));
    p!("primitiveFloatHyperbolicCosineIfFail:", |_, r, _| float_fn(r, f64::cosh));
    p!("primitiveFloatHyperbolicTangentIfFail:", |_, r, _| float_fn(r, f64::tanh));
    p!("primitiveFloatFractionPartIfFail:", |_, r, _| float_fn(r, f64::fract));
    p!("primitiveFloatRaisedTo:ifFail:", |_, r, a| {
        let x = num(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let y = num(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?;
        Ok(Value::Float(to_f(&x).powf(to_f(&y))))
    });
    p!("primitiveFloatTimesTwoPower:ifFail:", |_, r, a| {
        let x = num(r).ok_or(PErr::Fail(RECV_TYPE))?;
        Ok(Value::Float(to_f(&x) * 2f64.powi(int_arg(&a[0])? as i32)))
    });
    // primitiveFloatTruncated (0x4cc2b0): never fails, answers a Float cut toward zero
    // (floor above zero, ceiling otherwise).
    p!("primitiveFloatTruncated", |_, r, _| match r {
        Value::Float(f) => Ok(Value::Float(if *f > 0.0 { f.floor() } else { f.ceil() })),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    // primitiveFloatSmallIntegerFloorIfFail: (0x4cbfb0): floor, then the same strict window as
    // rounding (-536870912.0 < r < 536870911.0); outside it ConversionFailed.
    p!("primitiveFloatSmallIntegerFloorIfFail:", |_, r, _| match r {
        Value::Float(f) => {
            let x = f.floor();
            if x > -536870912.0 && x < 536870911.0 {
                Ok(Value::Int(x as i64))
            } else {
                Err(PErr::Fail("ConversionFailed"))
            }
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    // primitiveFloatAsSmallIntegerIfFail: (0x4cc5a0): only a Float that already is a whole
    // number (floor(x) == x) inside the window converts; else SmallIntegerConversionFailed.
    p!("primitiveFloatAsSmallIntegerIfFail:", |_, r, _| match r {
        Value::Float(f) => {
            if f.floor() == *f && *f > -536870912.0 && *f < 536870911.0 {
                Ok(Value::Int(*f as i64))
            } else {
                Err(PErr::Fail("SmallIntegerConversionFailed"))
            }
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatTruncatedIfFail:", |_, r, _| match r {
        Value::Float(f) if f.is_finite() => Ok(float_to_int(f.trunc())),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatFloorIfFail:", |_, r, _| match r {
        Value::Float(f) if f.is_finite() => Ok(float_to_int(f.floor())),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatCeilingIfFail:", |_, r, _| match r {
        Value::Float(f) if f.is_finite() => Ok(float_to_int(f.ceil())),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    // primitiveFloatRoundedAsSmallIntegerIfFail: (0x4cc480): floor(x + 0.5) for x >= 0,
    // ceil(x - 0.5) below zero; the result must lie strictly between the Float constants
    // -536870912.0 (0x54ab28) and 536870911.0 (0x54ab20), else SmallIntegerConversionFailed.
    p!("primitiveFloatRoundedAsSmallIntegerIfFail:", |_, r, _| match r {
        Value::Float(f) => {
            let x = if *f >= 0.0 { (f + 0.5).floor() } else { (f - 0.5).ceil() };
            if x > -536870912.0 && x < 536870911.0 {
                Ok(Value::Int(x as i64))
            } else {
                Err(PErr::Fail("SmallIntegerConversionFailed"))
            }
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatRoundedIfFail:", |_, r, _| match r {
        Value::Float(f) if f.is_finite() => Ok(float_to_int(if *f >= 0.0 { (f + 0.5).floor() } else { (f - 0.5).ceil() })),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatPrintStringIfFail:", |_, r, _| match r {
        Value::Float(f) => Ok(Value::str(&format_float(*f))),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatPrintString:ifFail:", |_, r, a| match r {
        Value::Float(f) => {
            let d = int_arg(&a[0])?.clamp(0, 20) as usize;
            Ok(Value::str(&format!("{:.*}", d, f)))
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatIsNaNIfFail:", |_, r, _| match r {
        Value::Float(f) => Ok(Value::Bool(f.is_nan())),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveFloatPiIfFail:", |_, _, _| Ok(Value::Float(std::f64::consts::PI)));
    p!("primitiveFloatEIfFail:", |_, _, _| Ok(Value::Float(std::f64::consts::E)));
    p!("primitiveFloatInfinityIfFail:", |_, _, _| Ok(Value::Float(f64::INFINITY)));
    p!("primitiveFloatNaNIfFail:", |_, _, _| Ok(Value::Float(f64::NAN)));
    p!("primitiveMandelbrotAtRe:im:iterate:ifFail:", |_, _, a| {
        // the original accepts only Floats for re/im and a SmallInteger count
        let re = match a[0] {
            Value::Float(f) => f,
            _ => return Err(PErr::Fail(WRONG_TYPE)),
        };
        let im = match a[1] {
            Value::Float(f) => f,
            _ => return Err(PErr::Fail(SECOND_WRONG)),
        };
        let n = match a[2] {
            Value::Int(n) => n,
            _ => return Err(PErr::Fail(THIRD_WRONG)),
        };
        Ok(Value::Int(mandel(re, im, n)))
    });
    p!("primitiveSmallIntegerMaxValue", |_, _, _| Ok(Value::Int(SMI_MAX)));
    p!("primitiveSmallIntegerMinValue", |_, _, _| Ok(Value::Int(SMI_MIN)));

    // --- objects
    p!("primitiveEquivalent:", |_, r, a| Ok(Value::Bool(r.identical(&a[0]))));
    p!("primitiveClass", |it, r, _| Ok(it.class_value_of(r)));
    p!("primitiveHash", |_, r, _| Ok(Value::Int(r.identity_hash())));
    p!("primitiveShallowCopyIfFail:", |it, r, _| Ok(match r {
        Value::Str(s) => Value::from_chars(s.borrow().clone()),
        Value::Obj(o) => Value::Obj(Rc::new(Object {
            class: std::cell::RefCell::new(o.class.borrow().clone()),
            slots: std::cell::RefCell::new(o.slots.borrow().clone()),
            indexed: o.indexed.as_ref().map(|x| std::cell::RefCell::new(x.borrow().clone())),
        })),
        other => {
            let _ = it;
            other.clone()
        }
    }));
    p!("primitiveInstVarAt:ifFail:", |_, r, a| match r {
        Value::Obj(o) => {
            let n = o.slots.borrow().len();
            let i = index_arg(&a[0], n)?;
            Ok(o.get(i))
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveInstVarAt:put:ifFail:", |_, r, a| match r {
        Value::Obj(o) => {
            let n = o.slots.borrow().len();
            let i = index_arg(&a[0], n)?;
            o.set(i, a[1].clone());
            Ok(a[1].clone())
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveInstVarCount", |_, r, _| Ok(Value::Int(match r {
        Value::Obj(o) => o.slots.borrow().len() as i64,
        _ => 0,
    })));
    p!("primitivePerform:ifFail:", |it, r, a| perform(it, r, &a[0], vec![]));
    p!("primitivePerform:with:ifFail:", |it, r, a| perform(it, r, &a[0], vec![a[1].clone()]));
    p!("primitivePerform:with:with:ifFail:", |it, r, a| perform(it, r, &a[0], vec![a[1].clone(), a[2].clone()]));
    p!("primitivePerform:with:with:with:ifFail:", |it, r, a| perform(
        it,
        r,
        &a[0],
        vec![a[1].clone(), a[2].clone(), a[3].clone()]
    ));
    p!("primitivePerform:arguments:ifFail:", |it, r, a| {
        let args = items_of(&a[1]).ok_or(PErr::Fail(WRONG_TYPE))?;
        perform(it, r, &a[0], args)
    });
    p!("primitiveRespondsTo:", |it, r, a| {
        let s = match &a[0] {
            Value::Sym(s) => s.clone(),
            _ => return Ok(Value::Bool(false)),
        };
        Ok(Value::Bool(it.responds_to(r, &s)))
    });
    p!("primitiveNewIfFail:", |it, r, _| match r {
        Value::Class(c) => match c.format.get() {
            Format::Special => {
                if Rc::ptr_eq(c, &it.core.string) {
                    Ok(Value::str(""))
                } else {
                    Err(PErr::Fail("ReceiverHasWrongType"))
                }
            }
            _ => Ok(it.new_object(c)),
        },
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveNew:ifFail:", |it, r, a| {
        let n = match a[0] {
            Value::Int(n) if n < 0 => return Err(PErr::Fail("NegativeSize")),
            Value::Int(n) if n <= 50_000_000 => n as usize,
            Value::Int(_) => return Err(PErr::Fail("FailedAllocation")),
            _ => return Err(PErr::Fail(WRONG_TYPE)),
        };
        match r {
            Value::Class(c) => {
                if Rc::ptr_eq(c, &it.core.string) {
                    return Ok(Value::from_chars(vec![' '; n]));
                }
                match c.format.get() {
                    Format::Indexed => {
                        let fill = if Rc::ptr_eq(c, &it.core.byte_array) { Value::Int(0) } else { Value::Nil };
                        Ok(it.new_indexed(c, vec![fill; n]))
                    }
                    _ => Err(PErr::Fail("NotIndexable")),
                }
            }
            _ => Err(PErr::Fail(RECV_TYPE)),
        }
    });
    p!("primitiveIndexedObjectAt:ifFail:", at);
    p!("primitiveIndexedObjectAt:put:ifFail:", at_put);
    p!("primitiveIndexedByteCharacterAt:ifFail:", at);
    p!("primitiveIndexedByteAt:put:ifFail:", byte_at_put);
    p!("primitiveSizeIfFail:", |_, r, _| match size_of(r) {
        Some(n) => Ok(Value::Int(n as i64)),
        None => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveIndexedObjectReplaceFrom:to:with:startingAt:ifFail:", |_, r, a| {
        let (from, to, start) = (int_arg(&a[0])?, int_arg(&a[1])?, int_arg(&a[3])?);
        let src = items_of(&a[2]).ok_or(PErr::Fail(WRONG_TYPE))?;
        if to < from {
            return Ok(r.clone());
        }
        let count = (to - from + 1) as usize;
        if from < 1 || start < 1 || (start as usize - 1 + count) > src.len() {
            return Err(PErr::Fail(BOUNDS));
        }
        match r {
            Value::Obj(o) => {
                let mut v = o.indexed.as_ref().ok_or(PErr::Fail(RECV_TYPE))?.borrow_mut();
                if to as usize > v.len() {
                    return Err(PErr::Fail(BOUNDS));
                }
                for k in 0..count {
                    v[from as usize - 1 + k] = src[start as usize - 1 + k].clone();
                }
            }
            Value::Str(s) => {
                let mut v = s.borrow_mut();
                if to as usize > v.len() {
                    return Err(PErr::Fail(BOUNDS));
                }
                for k in 0..count {
                    match &src[start as usize - 1 + k] {
                        Value::Char(c) => v[from as usize - 1 + k] = *c,
                        _ => return Err(PErr::Fail(WRONG_TYPE)),
                    }
                }
            }
            _ => return Err(PErr::Fail(RECV_TYPE)),
        }
        Ok(r.clone())
    });
    p!("primitiveCopyFrom:to:ifFail:", |it, r, a| {
        let items = items_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let (from, to) = (int_arg(&a[0])?, int_arg(&a[1])?);
        if to < from {
            return Ok(same_kind(it, r, vec![]));
        }
        if from < 1 || to as usize > items.len() {
            return Err(PErr::Fail(BOUNDS));
        }
        Ok(same_kind(it, r, items[from as usize - 1..to as usize].to_vec()))
    });
    p!("primitiveCopyWithSize:ifFail:", |it, r, a| {
        let mut items = items_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let n = int_arg(&a[0])?.max(0) as usize;
        let fill = if matches!(r, Value::Str(_) | Value::Sym(_)) { Value::Char(' ') } else { Value::Nil };
        items.resize(n, fill);
        Ok(same_kind(it, r, items))
    });
    p!("primitiveArrayGrowTo:ifFail:", |_, r, a| match r {
        Value::Obj(o) => {
            let n = int_arg(&a[0])?.max(0) as usize;
            o.indexed.as_ref().ok_or(PErr::Fail(RECV_TYPE))?.borrow_mut().resize(n, Value::Nil);
            Ok(r.clone())
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });

    // --- strings, symbols, characters
    p!("primitiveStringConcatenate:ifFail:", |it, r, a| {
        if let (Some(mut x), Some(y)) = (chars_of(r), chars_of(&a[0])) {
            x.extend(y);
            return Ok(Value::from_chars(x));
        }
        let mut x = items_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        x.extend(items_of(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?);
        Ok(same_kind(it, r, x))
    });
    p!("primitiveStringAppend:ifFail:", |_, r, a| match r {
        Value::Str(s) => {
            match &a[0] {
                Value::Char(c) => s.borrow_mut().push(*c),
                other => {
                    let y = chars_of(other).ok_or(PErr::Fail(WRONG_TYPE))?;
                    s.borrow_mut().extend(y);
                }
            }
            Ok(r.clone())
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveIndexedByteCompare:ifFail:", |_, r, a| {
        let x = chars_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let y = chars_of(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?;
        Ok(Value::Int(match x.cmp(&y) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }))
    });
    p!("primitiveStringEqual:", |_, r, a| Ok(Value::Bool(match (chars_of(r), chars_of(&a[0])) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    })));
    p!("primitiveStringHash", |_, r, _| {
        let s = chars_of(r).unwrap_or_default();
        let mut h: u32 = 2166136261;
        for c in s {
            h = (h ^ c as u32).wrapping_mul(16777619);
        }
        Ok(Value::Int((h & 0x0fff_ffff) as i64))
    });
    p!("primitiveIndexedByteInternIfFail:", |it, r, _| {
        let s = string_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        Ok(Value::Sym(it.intern(&s)))
    });
    p!("primitiveSymbolAsStringIfFail:", |_, r, _| {
        let s = string_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        Ok(Value::str(&s))
    });
    p!("primitiveStringAsUppercaseIfFail:", |_, r, _| Ok(Value::str(&str_arg(r)?.to_uppercase())));
    p!("primitiveStringAsLowercaseIfFail:", |_, r, _| Ok(Value::str(&str_arg(r)?.to_lowercase())));
    p!("primitiveStringTrimIfFail:", |_, r, _| Ok(Value::str(str_arg(r)?.trim())));
    p!("primitiveStringReversedIfFail:", |_, r, _| Ok(Value::str(&str_arg(r)?.chars().rev().collect::<String>())));
    p!("primitiveStringAsNumberIfFail:", |_, r, _| Ok(parse_number(&str_arg(r)?).unwrap_or(Value::Nil)));
    p!("primitiveStringIndexOf:startingAt:ifFail:", |_, r, a| {
        let hay = chars_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let start = (int_arg(&a[1])?.max(1) - 1) as usize;
        let needle: Vec<char> = match &a[0] {
            Value::Char(c) => vec![*c],
            other => chars_of(other).ok_or(PErr::Fail(WRONG_TYPE))?,
        };
        if needle.is_empty() || start > hay.len() {
            return Ok(Value::Int(0));
        }
        for i in start..=hay.len().saturating_sub(needle.len()) {
            if i + needle.len() <= hay.len() && hay[i..i + needle.len()] == needle[..] {
                return Ok(Value::Int(i as i64 + 1));
            }
        }
        Ok(Value::Int(0))
    });
    p!("primitiveStringReplaceAll:with:ifFail:", |_, r, a| {
        let s = str_arg(r)?;
        let (x, y) = (str_arg(&a[0])?, str_arg(&a[1])?);
        if x.is_empty() {
            return Ok(Value::str(&s));
        }
        Ok(Value::str(&s.replace(&x, &y)))
    });
    p!("primitiveStringSubstrings:ifFail:", |it, r, a| {
        let s = str_arg(r)?;
        let seps: Vec<char> = match &a[0] {
            Value::Nil => vec![' ', '\t', '\n', '\r'],
            Value::Char(c) => vec![*c],
            other => chars_of(other).ok_or(PErr::Fail(WRONG_TYPE))?,
        };
        let parts: Vec<Value> = s.split(|c| seps.contains(&c)).filter(|p| !p.is_empty()).map(Value::str).collect();
        Ok(it.array(parts))
    });
    p!("primitiveStringLinesIfFail:", |it, r, _| {
        let s = str_arg(r)?;
        Ok(it.array(s.lines().map(Value::str).collect()))
    });
    p!("primitiveSymbolNumArgsIfFail:", |_, r, _| {
        let s = string_of(r).ok_or(PErr::Fail(RECV_TYPE))?;
        let n = if s.chars().next().map(|c| crate::lexer::is_binary_char(c) || c == '|').unwrap_or(false) {
            1
        } else {
            s.matches(':').count()
        };
        Ok(Value::Int(n as i64))
    });
    // primitiveCharacterFor:ifFail: (0x4bd810): only the 256 characters of the VM's character
    // table exist; other codes (negative ones too, an unsigned compare) fail with OutOfBounds.
    p!("primitiveCharacterFor:ifFail:", |_, _, a| match a[0] {
        Value::Int(i) if (0..256).contains(&i) => Ok(Value::Char(char::from_u32(i as u32).unwrap_or('?'))),
        Value::Int(_) => Err(PErr::Fail(BOUNDS)),
        _ => Err(PErr::Fail(WRONG_TYPE)),
    });
    p!("primitiveCharacterValueIfFail:", |_, r, _| match r {
        Value::Char(c) => Ok(Value::Int(*c as i64)),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveCharacterTest:ifFail:", |_, r, a| {
        let c = match r {
            Value::Char(c) => *c,
            _ => return Err(PErr::Fail(RECV_TYPE)),
        };
        let t = string_of(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?;
        Ok(match t.as_str() {
            "letter" => Value::Bool(c.is_alphabetic()),
            "digit" => Value::Bool(c.is_ascii_digit()),
            "upper" => Value::Bool(c.is_uppercase()),
            "lower" => Value::Bool(c.is_lowercase()),
            "separator" => Value::Bool(c.is_whitespace()),
            "vowel" => Value::Bool("aeiouAEIOU".contains(c)),
            "alphanumeric" => Value::Bool(c.is_alphanumeric()),
            "asUppercase" => Value::Char(c.to_uppercase().next().unwrap_or(c)),
            "asLowercase" => Value::Char(c.to_lowercase().next().unwrap_or(c)),
            "asString" => Value::str(&c.to_string()),
            _ => return Err(PErr::Fail(WRONG_TYPE)),
        })
    });

    // --- blocks and exceptions
    p!("primitiveValueWithArguments:ifFail:", |it, r, a| match r {
        Value::Block(b) => {
            let args = items_of(&a[0]).ok_or(PErr::Fail(WRONG_TYPE))?;
            if args.len() != b.code.nparams {
                return Err(PErr::Fail("ArgumentIsInvalid"));
            }
            let b = b.clone();
            Ok(it.call_block(&b, args)?)
        }
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveBlockNumArgs", |_, r, _| match r {
        Value::Block(b) => Ok(Value::Int(b.code.nparams as i64)),
        _ => Err(PErr::Fail(RECV_TYPE)),
    });
    p!("primitiveOnException:do:", |it, r, a| Ok(it.on_do(r.clone(), a[0].clone(), a[1].clone())?));
    p!("primitiveEnsure:", |it, r, a| {
        let b = match r {
            Value::Block(b) => b.clone(),
            _ => return Err(PErr::Fail(RECV_TYPE)),
        };
        let res = it.call_block(&b, vec![]);
        it.call_flexible(&a[0], vec![])?;
        Ok(res?)
    });
    p!("primitiveIfCurtailed:", |it, r, a| {
        let b = match r {
            Value::Block(b) => b.clone(),
            _ => return Err(PErr::Fail(RECV_TYPE)),
        };
        let res = it.call_block(&b, vec![]);
        if res.is_err() {
            it.call_flexible(&a[0], vec![])?;
        }
        Ok(res?)
    });
    p!("primitiveSignal", |it, r, _| Ok(it.signal(r.clone())?));
    p!("primitiveExceptionReturn:", |it, r, a| match it.get_ivar(r, "handlerId") {
        Value::Int(h) => Err(PErr::Unwind(Unwind::ExcReturn(h as u64, a[0].clone()))),
        _ => Err(PErr::Fail("NotSignaled")),
    });
    p!("primitiveExceptionRetry", |it, r, _| match it.get_ivar(r, "handlerId") {
        Value::Int(h) => Err(PErr::Unwind(Unwind::Retry(h as u64))),
        _ => Err(PErr::Fail("NotSignaled")),
    });
    p!("primitiveExceptionResume:", |it, r, a| match it.get_ivar(r, "signalId") {
        Value::Int(s) => Err(PErr::Unwind(Unwind::Resume(s as u64, a[0].clone()))),
        _ => Err(PErr::Fail("NotSignaled")),
    });
    p!("primitiveUnhandledError", |it, r, _| {
        let u = it.unhandled(r.clone());
        Err(PErr::Unwind(u))
    });

    // --- system
    p!("primitiveTranscriptShow:ifFail:", |it, _, a| {
        let s = match &a[0] {
            Value::Char(c) => c.to_string(),
            other => str_arg(other)?,
        };
        it.transcript_write(&s);
        Ok(Value::Nil)
    });
    p!("primitiveTranscriptClear", |it, _, _| {
        it.events.push(Event::ClearTranscript);
        Ok(Value::Nil)
    });
    p!("primitiveMillisecondClock", |_, _, _| Ok(Value::int((Interp::now_ms() as i64) % (1 << 29))));
    p!("primitiveMillisecondClockFloat", |_, _, _| Ok(Value::Float(Interp::now_ms())));
    p!("primitiveSendCount", |it, _, _| Ok(Value::int(it.sends as i64)));
    p!("primitiveRandomNext", |it, _, _| {
        let mut x = it.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        it.rng = x;
        Ok(Value::Float((x >> 11) as f64 / (1u64 << 53) as f64))
    });
    p!("primitiveRandomSeed:", |it, _, a| {
        it.rng = (int_arg(&a[0]).unwrap_or(1) as u64).wrapping_mul(0x9E3779B97F4A7C15) | 1;
        Ok(Value::Nil)
    });
    p!("primitiveInspect:", |it, _, a| {
        it.inspected.push(a[0].clone());
        let id = it.inspected.len() - 1;
        it.events.push(Event::Inspect(id));
        Ok(a[0].clone())
    });
    p!("primitiveBrowse:", |it, _, a| {
        let n = match &a[0] {
            Value::Class(c) | Value::Meta(c) => c.name(),
            other => string_of(other).unwrap_or_default(),
        };
        it.events.push(Event::Browse(n));
        Ok(a[0].clone())
    });
    p!("primitiveSmalltalkAt:ifFail:", |it, _, a| {
        let k = str_arg(&a[0])?;
        it.globals.get(&k).cloned().ok_or(PErr::Fail("NotFound"))
    });
    p!("primitiveSmalltalkAt:Put:ifFail:", |it, _, a| {
        let k = str_arg(&a[0])?;
        it.globals.insert(k, a[1].clone());
        Ok(a[1].clone())
    });
    p!("primitiveSmalltalkRemoveAt:ifFail:", |it, _, a| {
        let k = str_arg(&a[0])?;
        it.globals.remove(&k).ok_or(PErr::Fail("NotFound"))
    });
    p!("primitiveSmalltalkKeys", |it, _, _| {
        let mut ks: Vec<String> = it.globals.keys().cloned().collect();
        ks.sort();
        let v = ks.iter().map(|k| Value::Sym(it.intern(k))).collect();
        Ok(it.array(v))
    });
    p!("primitiveEvaluate:ifFail:", |it, _, a| {
        let src = str_arg(&a[0])?;
        match it.evaluate(&src, None, Value::Nil) {
            Ok(v) => Ok(v),
            Err(e) => Err(PErr::Unwind(Unwind::Error(Box::new(e)))),
        }
    });
    p!("primitiveFileIn:ifFail:", |it, _, a| {
        let src = str_arg(&a[0])?;
        let rep = it.file_in(&src);
        for e in &rep.errors {
            it.transcript_write(&format!("{}\n", e));
        }
        Ok(Value::Int(rep.methods as i64))
    });

    // --- canvas windows (the web page draws them)
    p!("primitiveCanvasOpenWidth:height:title:ifFail:", |it, _, a| {
        let w = int_arg(&a[0])?.clamp(1, 2000) as usize;
        let h = int_arg(&a[1])?.clamp(1, 2000) as usize;
        let title = string_of(&a[2]).unwrap_or_else(|| "Canvas".into());
        it.canvases.push(CanvasBuf { width: w, height: h, title, pixels: vec![0xFFFFFF; w * h] });
        Ok(Value::Int(it.canvases.len() as i64 - 1))
    });
    p!("primitiveCanvas:setX:y:rgb:ifFail:", |it, _, a| {
        let id = canvas_id(it, &a[0])?;
        let (x, y, c) = (int_arg(&a[1])?, int_arg(&a[2])?, int_arg(&a[3])?);
        let cv = &mut it.canvases[id];
        if x >= 0 && y >= 0 && (x as usize) < cv.width && (y as usize) < cv.height {
            cv.pixels[y as usize * cv.width + x as usize] = c as u32 & 0xFFFFFF;
        }
        Ok(Value::Nil)
    });
    p!("primitiveCanvas:fillX:y:width:height:rgb:ifFail:", |it, _, a| {
        let id = canvas_id(it, &a[0])?;
        let (x0, y0, w, h, c) = (int_arg(&a[1])?, int_arg(&a[2])?, int_arg(&a[3])?, int_arg(&a[4])?, int_arg(&a[5])?);
        let cv = &mut it.canvases[id];
        for y in y0.max(0)..(y0 + h).min(cv.height as i64) {
            for x in x0.max(0)..(x0 + w).min(cv.width as i64) {
                cv.pixels[y as usize * cv.width + x as usize] = c as u32 & 0xFFFFFF;
            }
        }
        Ok(Value::Nil)
    });
    p!("primitiveCanvas:lineX1:y1:x2:y2:rgb:ifFail:", |it, _, a| {
        let id = canvas_id(it, &a[0])?;
        let (mut x0, mut y0, x1, y1, c) = (int_arg(&a[1])?, int_arg(&a[2])?, int_arg(&a[3])?, int_arg(&a[4])?, int_arg(&a[5])?);
        let cv = &mut it.canvases[id];
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut guard = 0;
        loop {
            if x0 >= 0 && y0 >= 0 && (x0 as usize) < cv.width && (y0 as usize) < cv.height {
                cv.pixels[y0 as usize * cv.width + x0 as usize] = c as u32 & 0xFFFFFF;
            }
            if (x0 == x1 && y0 == y1) || guard > 10_000 {
                break;
            }
            guard += 1;
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
        Ok(Value::Nil)
    });
    p!("primitiveCanvasShow:ifFail:", |it, _, a| {
        let id = canvas_id(it, &a[0])?;
        if !it.events.iter().any(|e| matches!(e, Event::Canvas(i) if *i == id)) {
            it.events.push(Event::Canvas(id));
        }
        Ok(Value::Nil)
    });
    let _ = (BigInt::one(),);
    t
}

fn perform(it: &mut Interp, r: &Value, sel: &Value, args: Vec<Value>) -> PR {
    let s = match sel {
        Value::Sym(s) => s.clone(),
        Value::Str(_) => {
            let st = string_of(sel).unwrap();
            it.intern(&st)
        }
        _ => return Err(PErr::Fail(WRONG_TYPE)),
    };
    Ok(it.send(r.clone(), &s, args)?)
}
