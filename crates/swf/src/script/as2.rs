//! Compiles the script subset to `ActionScript` 1/2 actions, the body of a `DoAction` tag.
//!
//! Timeline control with a literal argument compiles to the dedicated actions the Flash IDE emits (`GotoFrame`,
//! `GotoLabel`, `Play`, `Stop`, `NextFrame`, `PreviousFrame`); everything else is a call of a global function or a
//! method. Every constant is pushed with its own `Push`, and no constant pool is used.

use anim_script::{BinOp, Expr, Func, Stmt, UnOp};

use crate::bits::PutBytes;
use crate::error::SwfError;

mod op {
    pub const NEXT_FRAME: u8 = 0x04;
    pub const PREV_FRAME: u8 = 0x05;
    pub const PLAY: u8 = 0x06;
    pub const STOP: u8 = 0x07;
    pub const SUBTRACT: u8 = 0x0B;
    pub const MULTIPLY: u8 = 0x0C;
    pub const DIVIDE: u8 = 0x0D;
    pub const NOT: u8 = 0x12;
    pub const POP: u8 = 0x17;
    pub const GET_VARIABLE: u8 = 0x1C;
    pub const SET_VARIABLE: u8 = 0x1D;
    pub const TRACE: u8 = 0x26;
    pub const DEFINE_LOCAL: u8 = 0x3C;
    pub const CALL_FUNCTION: u8 = 0x3D;
    pub const RETURN: u8 = 0x3E;
    pub const MODULO: u8 = 0x3F;
    pub const DEFINE_LOCAL2: u8 = 0x41;
    pub const ADD2: u8 = 0x47;
    pub const LESS2: u8 = 0x48;
    pub const EQUALS2: u8 = 0x49;
    pub const PUSH_DUPLICATE: u8 = 0x4C;
    pub const GET_MEMBER: u8 = 0x4E;
    pub const SET_MEMBER: u8 = 0x4F;
    pub const CALL_METHOD: u8 = 0x52;
    pub const STRICT_EQUALS: u8 = 0x66;
    pub const GREATER: u8 = 0x67;
    pub const GOTO_FRAME: u8 = 0x81;
    pub const STORE_REGISTER: u8 = 0x87;
    pub const GOTO_LABEL: u8 = 0x8C;
    pub const PUSH: u8 = 0x96;
    pub const JUMP: u8 = 0x99;
    pub const DEFINE_FUNCTION: u8 = 0x9B;
    pub const IF: u8 = 0x9D;
}

/// `Push` value types.
mod ty {
    pub const STRING: u8 = 0;
    pub const NULL: u8 = 2;
    pub const UNDEFINED: u8 = 3;
    pub const REGISTER: u8 = 4;
    pub const BOOLEAN: u8 = 5;
    pub const DOUBLE: u8 = 6;
}

/// Compiles statements to the body of a `DoAction` tag (terminated by a zero byte). Constructs the compiler cannot
/// express are skipped and described in the returned notes.
///
/// # Errors
/// [`SwfError::ScriptTooLarge`] when a jump or a function body exceeds the 16-bit offsets of the action format.
pub fn compile(stmts: &[Stmt]) -> Result<(Vec<u8>, Vec<String>), SwfError> {
    let mut c = Compiler { out: Vec::new(), notes: Vec::new(), depth: 0 };
    c.block(stmts)?;
    c.out.push(0);
    Ok((c.out, c.notes))
}

struct Compiler {
    out: Vec<u8>,
    notes: Vec<String>,
    /// How many function bodies deep the code being written is.
    depth: u32,
}

impl Compiler {
    fn block(&mut self, stmts: &[Stmt]) -> Result<(), SwfError> {
        for s in stmts {
            self.stmt(s)?;
        }
        Ok(())
    }

    fn simple(&mut self, code: u8) {
        self.out.push(code);
    }

    fn action(&mut self, code: u8, body: &[u8]) -> Result<(), SwfError> {
        self.out.push(code);
        let len = u16::try_from(body.len()).map_err(|_| SwfError::ScriptTooLarge("an action body"))?;
        self.out.put_u16(len);
        self.out.extend_from_slice(body);
        Ok(())
    }

    fn push(&mut self, ty: u8, payload: &[u8]) -> Result<(), SwfError> {
        let mut body = vec![ty];
        body.extend_from_slice(payload);
        self.action(op::PUSH, &body)
    }

    fn push_str(&mut self, s: &str) -> Result<(), SwfError> {
        let mut bytes = s.replace('\0', "").into_bytes();
        bytes.push(0);
        self.push(ty::STRING, &bytes)
    }

    fn push_num(&mut self, v: f64) -> Result<(), SwfError> {
        // A double is stored with its two 32-bit halves swapped.
        let le = v.to_le_bytes();
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(le.get(4..8).unwrap_or(&[0; 4]));
        bytes.extend_from_slice(le.get(0..4).unwrap_or(&[0; 4]));
        self.push(ty::DOUBLE, &bytes)
    }

    fn push_register(&mut self, n: u8) -> Result<(), SwfError> {
        self.push(ty::REGISTER, &[n])
    }

    /// Writes a forward `Jump` or `If` with an unknown offset and returns where the offset sits.
    fn branch(&mut self, code: u8) -> usize {
        self.out.push(code);
        self.out.put_u16(2);
        let at = self.out.len();
        self.out.put_u16(0);
        at
    }

    /// Points the branch written at `at` to the current end of the code.
    fn patch(&mut self, at: usize) -> Result<(), SwfError> {
        let offset = i16::try_from(self.out.len().saturating_sub(at.saturating_add(2))).map_err(|_| SwfError::ScriptTooLarge("a jump"))?;
        if let Some(slot) = self.out.get_mut(at..at.saturating_add(2)) {
            slot.copy_from_slice(&offset.to_le_bytes());
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), SwfError> {
        match s {
            Stmt::Expr(e) => self.expr(e, false),
            Stmt::Var { name, init } => {
                if self.depth > 0 {
                    self.push_str(name)?;
                    if let Some(init) = init {
                        self.expr(init, true)?;
                        self.simple(op::DEFINE_LOCAL);
                    } else {
                        self.simple(op::DEFINE_LOCAL2);
                    }
                } else if let Some(init) = init {
                    self.push_str(name)?;
                    self.expr(init, true)?;
                    self.simple(op::SET_VARIABLE);
                }
                Ok(())
            }
            Stmt::If { cond, then, otherwise } => {
                self.expr(cond, true)?;
                self.simple(op::NOT);
                let to_else = self.branch(op::IF);
                self.block(then)?;
                if otherwise.is_empty() {
                    self.patch(to_else)
                } else {
                    let to_end = self.branch(op::JUMP);
                    self.patch(to_else)?;
                    self.block(otherwise)?;
                    self.patch(to_end)
                }
            }
            Stmt::Block(stmts) => self.block(stmts),
            Stmt::Function(f) => self.function(f, false),
            Stmt::Return(value) => {
                match value {
                    Some(e) => self.expr(e, true)?,
                    None => self.push(ty::UNDEFINED, &[])?,
                }
                self.simple(op::RETURN);
                Ok(())
            }
            Stmt::Unsupported { line, text } => {
                self.notes.push(format!("line {line}: `{text}` is outside the supported subset and was not exported"));
                Ok(())
            }
        }
    }

    /// A function declaration, or (with `value`) a literal that leaves the function on the stack.
    fn function(&mut self, f: &Func, value: bool) -> Result<(), SwfError> {
        let mut body = Self { out: Vec::new(), notes: Vec::new(), depth: self.depth.saturating_add(1) };
        body.block(&f.body)?;
        self.notes.append(&mut body.notes);
        let mut head = Vec::new();
        head.put_cstr(if value { "" } else { f.name.as_deref().unwrap_or("") })?;
        head.put_u16(u16::try_from(f.params.len()).map_err(|_| SwfError::ScriptTooLarge("a parameter list"))?);
        for p in &f.params {
            head.put_cstr(p)?;
        }
        head.put_u16(u16::try_from(body.out.len()).map_err(|_| SwfError::ScriptTooLarge("a function body"))?);
        self.action(op::DEFINE_FUNCTION, &head)?;
        self.out.extend_from_slice(&body.out);
        Ok(())
    }

    /// Writes code that leaves the value of `e` on the stack when `want`, and nothing otherwise.
    fn expr(&mut self, e: &Expr, want: bool) -> Result<(), SwfError> {
        match e {
            Expr::Call(callee, args) => return self.call(callee, args, want),
            Expr::Assign(target, op, value) => return self.assign(target, *op, value, want),
            Expr::Update { target, delta, prefix } => {
                if want && !*prefix {
                    self.expr(target, true)?;
                    self.assign(target, Some(BinOp::Add), &Expr::Num(*delta), false)?;
                    return Ok(());
                }
                return self.assign(target, Some(BinOp::Add), &Expr::Num(*delta), want);
            }
            _ => {}
        }
        if !want {
            // Only effects matter, and none of the remaining forms have any beyond their operands.
            return match e {
                Expr::Unary(_, a) | Expr::Member(a, _) => self.expr(a, false),
                Expr::Binary(_, a, b) => {
                    self.expr(a, false)?;
                    self.expr(b, false)
                }
                _ => Ok(()),
            };
        }
        match e {
            Expr::Num(v) => self.push_num(*v),
            Expr::Str(s) => self.push_str(s),
            Expr::Bool(b) => self.push(ty::BOOLEAN, &[u8::from(*b)]),
            Expr::Null => self.push(ty::NULL, &[]),
            Expr::This => {
                self.push_str("this")?;
                self.simple(op::GET_VARIABLE);
                Ok(())
            }
            Expr::Ident(name) if name == "undefined" => self.push(ty::UNDEFINED, &[]),
            Expr::Ident(name) => {
                self.push_str(name)?;
                self.simple(op::GET_VARIABLE);
                Ok(())
            }
            Expr::Member(base, name) => {
                self.expr(base, true)?;
                self.push_str(name)?;
                self.simple(op::GET_MEMBER);
                Ok(())
            }
            Expr::Unary(UnOp::Not, a) => {
                self.expr(a, true)?;
                self.simple(op::NOT);
                Ok(())
            }
            Expr::Unary(UnOp::Neg, a) => {
                self.push_num(0.0)?;
                self.expr(a, true)?;
                self.simple(op::SUBTRACT);
                Ok(())
            }
            Expr::Binary(BinOp::And, a, b) => {
                self.expr(a, true)?;
                self.simple(op::PUSH_DUPLICATE);
                self.simple(op::NOT);
                let end = self.branch(op::IF);
                self.simple(op::POP);
                self.expr(b, true)?;
                self.patch(end)
            }
            Expr::Binary(BinOp::Or, a, b) => {
                self.expr(a, true)?;
                self.simple(op::PUSH_DUPLICATE);
                let end = self.branch(op::IF);
                self.simple(op::POP);
                self.expr(b, true)?;
                self.patch(end)
            }
            Expr::Binary(o, a, b) => {
                self.expr(a, true)?;
                self.expr(b, true)?;
                self.binary(*o);
                Ok(())
            }
            Expr::Function(f) => self.function(f, true),
            Expr::Call(..) | Expr::Assign(..) | Expr::Update { .. } => Ok(()),
        }
    }

    /// The action for a binary operator whose operands are on the stack.
    fn binary(&mut self, o: BinOp) {
        match o {
            BinOp::Add => self.simple(op::ADD2),
            BinOp::Sub => self.simple(op::SUBTRACT),
            BinOp::Mul => self.simple(op::MULTIPLY),
            BinOp::Div => self.simple(op::DIVIDE),
            BinOp::Mod => self.simple(op::MODULO),
            BinOp::Eq => self.simple(op::EQUALS2),
            BinOp::Ne => {
                self.simple(op::EQUALS2);
                self.simple(op::NOT);
            }
            BinOp::StrictEq => self.simple(op::STRICT_EQUALS),
            BinOp::StrictNe => {
                self.simple(op::STRICT_EQUALS);
                self.simple(op::NOT);
            }
            BinOp::Lt => self.simple(op::LESS2),
            BinOp::Gt => self.simple(op::GREATER),
            BinOp::Le => {
                self.simple(op::GREATER);
                self.simple(op::NOT);
            }
            BinOp::Ge => {
                self.simple(op::LESS2);
                self.simple(op::NOT);
            }
            // Short-circuit operators are handled where the operands are compiled.
            BinOp::And | BinOp::Or => {}
        }
    }

    fn assign(&mut self, target: &Expr, compound: Option<BinOp>, value: &Expr, want: bool) -> Result<(), SwfError> {
        match target {
            Expr::Ident(name) => {
                self.push_str(name)?;
                self.assigned_value(target, compound, value)?;
                if want {
                    self.action(op::STORE_REGISTER, &[0])?;
                }
                self.simple(op::SET_VARIABLE);
            }
            Expr::Member(base, name) => {
                self.expr(base, true)?;
                self.push_str(name)?;
                self.assigned_value(target, compound, value)?;
                if want {
                    self.action(op::STORE_REGISTER, &[0])?;
                }
                self.simple(op::SET_MEMBER);
            }
            _ => {
                self.notes.push("an assignment to something that is not a variable or a property was not exported".to_owned());
                return if want { self.push(ty::UNDEFINED, &[]) } else { Ok(()) };
            }
        }
        if want {
            self.push_register(0)?;
        }
        Ok(())
    }

    /// The value an assignment stores: the right side, or the target combined with it for `+=` and friends.
    fn assigned_value(&mut self, target: &Expr, compound: Option<BinOp>, value: &Expr) -> Result<(), SwfError> {
        if let Some(o) = compound {
            self.expr(target, true)?;
            self.expr(value, true)?;
            self.binary(o);
            Ok(())
        } else {
            self.expr(value, true)
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], want: bool) -> Result<(), SwfError> {
        if let Expr::Ident(name) = callee
            && self.timeline_call(name, args)?
        {
            return if want { self.push(ty::UNDEFINED, &[]) } else { Ok(()) };
        }
        match callee {
            Expr::Ident(name) if name == "trace" && args.len() == 1 => {
                if let Some(a) = args.first() {
                    self.expr(a, true)?;
                }
                self.simple(op::TRACE);
                return if want { self.push(ty::UNDEFINED, &[]) } else { Ok(()) };
            }
            Expr::Ident(name) => {
                self.args(args)?;
                self.push_str(name)?;
                self.simple(op::CALL_FUNCTION);
            }
            Expr::Member(base, name) => {
                self.args(args)?;
                self.expr(base, true)?;
                self.push_str(name)?;
                self.simple(op::CALL_METHOD);
            }
            _ => {
                self.notes.push("a call of something that is not a function name or a method was not exported".to_owned());
                return if want { self.push(ty::UNDEFINED, &[]) } else { Ok(()) };
            }
        }
        if !want {
            self.simple(op::POP);
        }
        Ok(())
    }

    /// Pushes the arguments last to first, then their count.
    fn args(&mut self, args: &[Expr]) -> Result<(), SwfError> {
        for a in args.iter().rev() {
            self.expr(a, true)?;
        }
        self.push_num(f64::from(u32::try_from(args.len()).unwrap_or(u32::MAX)))
    }

    /// The dedicated actions for a bare timeline-control call with literal arguments. False when `name(args)` is not
    /// such a call.
    fn timeline_call(&mut self, name: &str, args: &[Expr]) -> Result<bool, SwfError> {
        match (name, args) {
            ("stop", []) => self.simple(op::STOP),
            ("play", []) => self.simple(op::PLAY),
            ("nextFrame", []) => self.simple(op::NEXT_FRAME),
            ("prevFrame", []) => self.simple(op::PREV_FRAME),
            ("gotoAndPlay" | "gotoAndStop", [.., Expr::Num(n)]) => {
                let frame = u16::try_from(crate::script::frame_number(*n).saturating_sub(1)).unwrap_or(u16::MAX);
                self.action(op::GOTO_FRAME, &frame.to_le_bytes())?;
                self.simple(if name == "gotoAndPlay" { op::PLAY } else { op::STOP });
            }
            ("gotoAndPlay" | "gotoAndStop", [.., Expr::Str(label)]) => {
                let mut body = Vec::new();
                body.put_cstr(&label.replace('\0', ""))?;
                self.action(op::GOTO_LABEL, &body)?;
                self.simple(if name == "gotoAndPlay" { op::PLAY } else { op::STOP });
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
