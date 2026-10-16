//! Compiles the script subset to an `ActionScript` Byte Code (ABC) file for a `DoABC` tag.
//!
//! The file declares one class, `Main`, that extends `flash.display.MovieClip` and is bound to the main timeline by a
//! `SymbolClass` tag. Its constructor registers every frame script with `addFrameScript`, exactly as the Flash IDE's
//! compiler does for a document without a document class:
//!
//! - each frame script becomes a method `frameN` (N is the 1-based frame number);
//! - the `var`s of frame scripts become fields of the class and the `function`s become methods, so frame scripts share
//!   them as they do in the IDE;
//! - function literals become closures; they reach the timeline's members and instances through the scope chain, but
//!   do not capture the locals of an enclosing function;
//! - names are looked up with a namespace set that holds the public namespace and the `flash.*` packages, so `stop`,
//!   `MouseEvent`, `MovieClip(root)` and the like resolve without `import`s.

use std::collections::HashMap;

use anim_core::num::round_f64_to_i32;
use anim_script::{BinOp, Expr, Func, Stmt, UnOp};

use crate::bits::PutBytes;

mod op {
    pub const JUMP: u8 = 0x10;
    pub const IF_TRUE: u8 = 0x11;
    pub const IF_FALSE: u8 = 0x12;
    pub const POP_SCOPE: u8 = 0x1D;
    pub const PUSH_NULL: u8 = 0x20;
    pub const PUSH_UNDEFINED: u8 = 0x21;
    pub const PUSH_BYTE: u8 = 0x24;
    pub const PUSH_TRUE: u8 = 0x26;
    pub const PUSH_FALSE: u8 = 0x27;
    pub const PUSH_NAN: u8 = 0x28;
    pub const POP: u8 = 0x29;
    pub const DUP: u8 = 0x2A;
    pub const PUSH_SCOPE: u8 = 0x30;
    pub const PUSH_STRING: u8 = 0x2C;
    pub const PUSH_INT: u8 = 0x2D;
    pub const PUSH_DOUBLE: u8 = 0x2F;
    pub const NEW_FUNCTION: u8 = 0x40;
    pub const CALL: u8 = 0x41;
    pub const CALL_PROPERTY: u8 = 0x46;
    pub const RETURN_VOID: u8 = 0x47;
    pub const RETURN_VALUE: u8 = 0x48;
    pub const CONSTRUCT_SUPER: u8 = 0x49;
    pub const CALL_PROP_VOID: u8 = 0x4F;
    pub const NEW_CLASS: u8 = 0x58;
    pub const GET_LEX: u8 = 0x60;
    pub const SET_PROPERTY: u8 = 0x61;
    pub const GET_LOCAL: u8 = 0x62;
    pub const SET_LOCAL: u8 = 0x63;
    pub const FIND_PROP_STRICT: u8 = 0x5D;
    pub const GET_SCOPE_OBJECT: u8 = 0x65;
    pub const GET_PROPERTY: u8 = 0x66;
    pub const INIT_PROPERTY: u8 = 0x68;
    pub const NEGATE: u8 = 0x90;
    pub const NOT: u8 = 0x96;
    pub const ADD: u8 = 0xA0;
    pub const SUBTRACT: u8 = 0xA1;
    pub const MULTIPLY: u8 = 0xA2;
    pub const DIVIDE: u8 = 0xA3;
    pub const MODULO: u8 = 0xA4;
    pub const EQUALS: u8 = 0xAB;
    pub const STRICT_EQUALS: u8 = 0xAC;
    pub const LESS_THAN: u8 = 0xAD;
    pub const LESS_EQUALS: u8 = 0xAE;
    pub const GREATER_THAN: u8 = 0xAF;
    pub const GREATER_EQUALS: u8 = 0xB0;
    pub const GET_LOCAL_0: u8 = 0xD0;
}

/// The packages whose classes frame scripts may name without importing them.
const PACKAGES: [&str; 9] =
    ["flash.display", "flash.events", "flash.text", "flash.utils", "flash.geom", "flash.media", "flash.net", "flash.ui", "flash.system"];

/// The kind byte of a package namespace.
const PACKAGE_NAMESPACE: u8 = 0x16;

/// The class the main timeline is bound to.
pub const CLASS_NAME: &str = "Main";

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Multiname {
    /// A fully qualified name: namespace index, then string index.
    Qualified(u32, u32),
    /// An unqualified name looked up in namespace set 1.
    Lookup(u32),
}

struct MethodDef {
    params: u32,
    code: Vec<u8>,
    max_stack: u32,
    locals: u32,
    max_scope: u32,
}

/// The tables of the file under construction.
#[derive(Default)]
struct Abc {
    strings: Vec<String>,
    string_ids: HashMap<String, u32>,
    ints: Vec<i32>,
    doubles: Vec<f64>,
    multinames: Vec<Multiname>,
    multiname_ids: HashMap<Multiname, u32>,
    methods: Vec<MethodDef>,
    notes: Vec<String>,
}

impl Abc {
    fn string(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(i) = self.string_ids.get(s) {
            return *i;
        }
        self.strings.push(s.to_owned());
        let i = count(self.strings.len());
        self.string_ids.insert(s.to_owned(), i);
        i
    }

    fn int(&mut self, v: i32) -> u32 {
        if let Some(at) = self.ints.iter().position(|x| *x == v) {
            return count(at.saturating_add(1));
        }
        self.ints.push(v);
        count(self.ints.len())
    }

    fn double(&mut self, v: f64) -> u32 {
        if let Some(at) = self.doubles.iter().position(|x| x.to_bits() == v.to_bits()) {
            return count(at.saturating_add(1));
        }
        self.doubles.push(v);
        count(self.doubles.len())
    }

    fn multiname(&mut self, m: Multiname) -> u32 {
        if let Some(i) = self.multiname_ids.get(&m) {
            return *i;
        }
        self.multinames.push(m.clone());
        let i = count(self.multinames.len());
        self.multiname_ids.insert(m, i);
        i
    }

    /// A name looked up through the namespace set.
    fn lookup(&mut self, name: &str) -> u32 {
        let s = self.string(name);
        self.multiname(Multiname::Lookup(s))
    }

    /// A name in the public namespace, for definitions.
    fn public(&mut self, name: &str) -> u32 {
        let s = self.string(name);
        self.multiname(Multiname::Qualified(1, s))
    }

    /// A name in the namespace of the package at position `package` of [`PACKAGES`].
    fn packaged(&mut self, package: usize, name: &str) -> u32 {
        let s = self.string(name);
        // Namespace 1 is the public one; the packages follow it.
        self.multiname(Multiname::Qualified(count(package.saturating_add(2)), s))
    }

    fn add_method(&mut self, m: MethodDef) -> u32 {
        self.methods.push(m);
        count(self.methods.len().saturating_sub(1))
    }
}

/// Instruction writer that tracks the operand stack depth.
struct Code {
    bytes: Vec<u8>,
    depth: i32,
    max: i32,
}

impl Code {
    const fn new() -> Self {
        Self { bytes: Vec::new(), depth: 0, max: 0 }
    }

    fn adjust(&mut self, delta: i32) {
        self.depth = self.depth.saturating_add(delta);
        self.max = self.max.max(self.depth);
    }

    fn op(&mut self, code: u8, delta: i32) {
        self.bytes.push(code);
        self.adjust(delta);
    }

    fn op_u30(&mut self, code: u8, v: u32, delta: i32) {
        self.bytes.push(code);
        self.bytes.put_encoded_u32(v);
        self.adjust(delta);
    }

    /// A branch with an unknown target; returns where the offset sits.
    fn branch(&mut self, code: u8, delta: i32) -> usize {
        self.bytes.push(code);
        let at = self.bytes.len();
        self.bytes.extend_from_slice(&[0, 0, 0]);
        self.adjust(delta);
        at
    }

    /// Points the branch whose offset sits at `at` to the current end of the code.
    fn patch(&mut self, at: usize) {
        let offset = i32::try_from(self.bytes.len().saturating_sub(at.saturating_add(3))).unwrap_or(0);
        let le = offset.to_le_bytes();
        if let (Some(slot), Some(src)) = (self.bytes.get_mut(at..at.saturating_add(3)), le.get(..3)) {
            slot.copy_from_slice(src);
        }
    }
}

/// What a name in a function body refers to.
struct Scope {
    /// Parameters and `var`s of the function being compiled, with their registers. Empty in frame scripts, whose
    /// variables are fields of the timeline.
    locals: Vec<(String, u32)>,
    /// The register that holds the value of an assignment while it is stored.
    tmp: u32,
}

struct Body<'a> {
    abc: &'a mut Abc,
    code: Code,
    scope: Scope,
}

/// The names declared by `var` in `stmts` and the blocks inside them (not inside functions).
fn declared_vars(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Var { name, .. } => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Stmt::If { then, otherwise, .. } => {
                declared_vars(then, out);
                declared_vars(otherwise, out);
            }
            Stmt::Block(b) => declared_vars(b, out),
            _ => {}
        }
    }
}

/// The function declarations in `stmts` and the blocks inside them.
fn declared_functions<'a>(stmts: &'a [Stmt], out: &mut Vec<&'a Func>) {
    for s in stmts {
        match s {
            Stmt::Function(f) => out.push(f),
            Stmt::If { then, otherwise, .. } => {
                declared_functions(then, out);
                declared_functions(otherwise, out);
            }
            Stmt::Block(b) => declared_functions(b, out),
            _ => {}
        }
    }
}

impl<'a> Body<'a> {
    /// A method body whose first instructions make `this` the innermost scope.
    fn new(abc: &'a mut Abc, scope: Scope) -> Self {
        let mut code = Code::new();
        code.op(op::GET_LOCAL_0, 1);
        code.op(op::PUSH_SCOPE, -1);
        Self { abc, code, scope }
    }

    fn local(&self, name: &str) -> Option<u32> {
        self.scope.locals.iter().find(|(n, _)| n == name).map(|(_, r)| *r)
    }

    fn get_local(&mut self, r: u32) {
        self.code.op_u30(op::GET_LOCAL, r, 1);
    }

    fn set_local(&mut self, r: u32) {
        self.code.op_u30(op::SET_LOCAL, r, -1);
    }

    fn block(&mut self, stmts: &[Stmt], toplevel: bool) {
        for s in stmts {
            self.stmt(s, toplevel);
        }
    }

    fn stmt(&mut self, s: &Stmt, toplevel: bool) {
        match s {
            Stmt::Expr(e) => self.expr(e, false),
            Stmt::Var { name, init } => {
                if let Some(init) = init {
                    self.assign(&Expr::Ident(name.clone()), None, init, false);
                }
            }
            Stmt::If { cond, then, otherwise } => {
                self.expr(cond, true);
                let to_else = self.code.branch(op::IF_FALSE, -1);
                self.block(then, toplevel);
                if otherwise.is_empty() {
                    self.code.patch(to_else);
                } else {
                    let to_end = self.code.branch(op::JUMP, 0);
                    self.code.patch(to_else);
                    self.block(otherwise, toplevel);
                    self.code.patch(to_end);
                }
            }
            Stmt::Block(b) => self.block(b, toplevel),
            Stmt::Function(f) => {
                // Declarations in frame scripts are methods of the class; inside a function they are local closures.
                if !toplevel {
                    self.function_value(f);
                    if let Some(r) = f.name.as_deref().and_then(|n| self.local(n)) {
                        self.set_local(r);
                    } else {
                        self.code.op(op::POP, -1);
                    }
                }
            }
            Stmt::Return(value) => match value {
                Some(e) => {
                    self.expr(e, true);
                    self.code.op(op::RETURN_VALUE, -1);
                }
                None => self.code.op(op::RETURN_VOID, 0),
            },
            Stmt::Unsupported { line, text } => {
                self.abc.notes.push(format!("line {line}: `{text}` is outside the supported subset and was not exported"));
            }
        }
    }

    /// Compiles a function literal to a method and pushes it as a closure.
    fn function_value(&mut self, f: &Func) {
        let index = compile_function(self.abc, f);
        self.code.op_u30(op::NEW_FUNCTION, index, 1);
    }

    fn expr(&mut self, e: &Expr, want: bool) {
        match e {
            Expr::Call(callee, args) => return self.call(callee, args, want),
            Expr::Assign(target, o, value) => return self.assign(target, *o, value, want),
            Expr::Update { target, delta, prefix } => {
                let step = Expr::Num(*delta);
                if want && !*prefix {
                    self.expr(target, true);
                    self.assign(target, Some(BinOp::Add), &step, false);
                } else {
                    self.assign(target, Some(BinOp::Add), &step, want);
                }
                return;
            }
            _ => {}
        }
        if !want {
            match e {
                Expr::Unary(_, a) | Expr::Member(a, _) => self.expr(a, false),
                Expr::Binary(_, a, b) => {
                    self.expr(a, false);
                    self.expr(b, false);
                }
                _ => {}
            }
            return;
        }
        match e {
            Expr::Num(v) => self.number(*v),
            Expr::Str(s) => {
                let i = self.abc.string(s);
                self.code.op_u30(op::PUSH_STRING, i, 1);
            }
            Expr::Bool(b) => self.code.op(if *b { op::PUSH_TRUE } else { op::PUSH_FALSE }, 1),
            Expr::Null => self.code.op(op::PUSH_NULL, 1),
            Expr::This => self.code.op(op::GET_LOCAL_0, 1),
            Expr::Ident(name) if name == "undefined" => self.code.op(op::PUSH_UNDEFINED, 1),
            Expr::Ident(name) if name == "NaN" => self.code.op(op::PUSH_NAN, 1),
            Expr::Ident(name) => {
                if let Some(r) = self.local(name) {
                    self.get_local(r);
                } else {
                    let mn = self.abc.lookup(name);
                    self.code.op_u30(op::GET_LEX, mn, 1);
                }
            }
            Expr::Member(base, name) => {
                self.expr(base, true);
                let mn = self.abc.lookup(name);
                self.code.op_u30(op::GET_PROPERTY, mn, 0);
            }
            Expr::Unary(UnOp::Not, a) => {
                self.expr(a, true);
                self.code.op(op::NOT, 0);
            }
            Expr::Unary(UnOp::Neg, a) => {
                self.expr(a, true);
                self.code.op(op::NEGATE, 0);
            }
            Expr::Binary(BinOp::And, a, b) => {
                self.expr(a, true);
                self.code.op(op::DUP, 1);
                let end = self.code.branch(op::IF_FALSE, -1);
                self.code.op(op::POP, -1);
                self.expr(b, true);
                self.code.patch(end);
            }
            Expr::Binary(BinOp::Or, a, b) => {
                self.expr(a, true);
                self.code.op(op::DUP, 1);
                let end = self.code.branch(op::IF_TRUE, -1);
                self.code.op(op::POP, -1);
                self.expr(b, true);
                self.code.patch(end);
            }
            Expr::Binary(o, a, b) => {
                self.expr(a, true);
                self.expr(b, true);
                self.binary(*o);
            }
            Expr::Function(f) => self.function_value(f),
            Expr::Call(..) | Expr::Assign(..) | Expr::Update { .. } => {}
        }
    }

    fn number(&mut self, v: f64) {
        if v.is_nan() {
            self.code.op(op::PUSH_NAN, 1);
            return;
        }
        let rounded = round_f64_to_i32(v);
        if (f64::from(rounded) - v).abs() < f64::EPSILON && v.abs() < 2e9 {
            if let Ok(b) = i8::try_from(rounded) {
                self.code.op(op::PUSH_BYTE, 1);
                self.code.bytes.extend_from_slice(&b.to_le_bytes());
            } else {
                let i = self.abc.int(rounded);
                self.code.op_u30(op::PUSH_INT, i, 1);
            }
        } else {
            let i = self.abc.double(v);
            self.code.op_u30(op::PUSH_DOUBLE, i, 1);
        }
    }

    /// The instruction for a binary operator whose operands are on the stack.
    fn binary(&mut self, o: BinOp) {
        let (code, negate) = match o {
            BinOp::Add => (op::ADD, false),
            BinOp::Sub => (op::SUBTRACT, false),
            BinOp::Mul => (op::MULTIPLY, false),
            BinOp::Div => (op::DIVIDE, false),
            BinOp::Mod => (op::MODULO, false),
            BinOp::Eq => (op::EQUALS, false),
            BinOp::Ne => (op::EQUALS, true),
            BinOp::StrictEq => (op::STRICT_EQUALS, false),
            BinOp::StrictNe => (op::STRICT_EQUALS, true),
            BinOp::Lt => (op::LESS_THAN, false),
            BinOp::Le => (op::LESS_EQUALS, false),
            BinOp::Gt => (op::GREATER_THAN, false),
            BinOp::Ge => (op::GREATER_EQUALS, false),
            // Short-circuit operators are compiled where their operands are.
            BinOp::And | BinOp::Or => return,
        };
        self.code.op(code, -1);
        if negate {
            self.code.op(op::NOT, 0);
        }
    }

    fn assign(&mut self, target: &Expr, compound: Option<BinOp>, value: &Expr, want: bool) {
        let tmp = self.scope.tmp;
        match target {
            Expr::Ident(name) => {
                if let Some(r) = self.local(name) {
                    self.assigned_value(target, compound, value);
                    if want {
                        self.code.op(op::DUP, 1);
                    }
                    self.set_local(r);
                    return;
                }
                let mn = self.abc.lookup(name);
                self.code.op_u30(op::FIND_PROP_STRICT, mn, 1);
                self.assigned_value(target, compound, value);
                if want {
                    self.code.op(op::DUP, 1);
                    self.set_local(tmp);
                }
                self.code.op_u30(op::SET_PROPERTY, mn, -2);
            }
            Expr::Member(base, name) => {
                self.expr(base, true);
                let mn = self.abc.lookup(name);
                self.assigned_value(target, compound, value);
                if want {
                    self.code.op(op::DUP, 1);
                    self.set_local(tmp);
                }
                self.code.op_u30(op::SET_PROPERTY, mn, -2);
            }
            _ => {
                self.abc.notes.push("an assignment to something that is not a variable or a property was not exported".to_owned());
                if want {
                    self.code.op(op::PUSH_UNDEFINED, 1);
                }
                return;
            }
        }
        if want {
            self.get_local(tmp);
        }
    }

    fn assigned_value(&mut self, target: &Expr, compound: Option<BinOp>, value: &Expr) {
        if let Some(o) = compound {
            self.expr(target, true);
            self.expr(value, true);
            self.binary(o);
        } else {
            self.expr(value, true);
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], want: bool) {
        let arity = count(args.len());
        let delta = i32::try_from(args.len()).unwrap_or(i32::MAX);
        match callee {
            Expr::Ident(name) if self.local(name).is_none() => {
                let mn = self.abc.lookup(name);
                self.code.op_u30(op::FIND_PROP_STRICT, mn, 1);
                for a in args {
                    self.expr(a, true);
                }
                self.property_call(mn, arity, delta, want);
            }
            Expr::Member(base, name) => {
                self.expr(base, true);
                for a in args {
                    self.expr(a, true);
                }
                let mn = self.abc.lookup(name);
                self.property_call(mn, arity, delta, want);
            }
            other => {
                // A function value: function, receiver, arguments.
                self.expr(other, true);
                self.code.op(op::PUSH_UNDEFINED, 1);
                for a in args {
                    self.expr(a, true);
                }
                self.code.op_u30(op::CALL, arity, delta.saturating_add(1).saturating_neg());
                if !want {
                    self.code.op(op::POP, -1);
                }
            }
        }
    }

    fn property_call(&mut self, mn: u32, arity: u32, delta: i32, want: bool) {
        if want {
            self.code.bytes.push(op::CALL_PROPERTY);
            self.code.bytes.put_encoded_u32(mn);
            self.code.bytes.put_encoded_u32(arity);
            self.code.adjust(delta.saturating_neg());
        } else {
            self.code.bytes.push(op::CALL_PROP_VOID);
            self.code.bytes.put_encoded_u32(mn);
            self.code.bytes.put_encoded_u32(arity);
            self.code.adjust(delta.saturating_add(1).saturating_neg());
        }
    }

    fn finish(mut self, params: u32) -> MethodDef {
        self.code.op(op::RETURN_VOID, 0);
        MethodDef {
            params,
            code: self.code.bytes,
            max_stack: u32::try_from(self.code.max).unwrap_or(0).saturating_add(1),
            locals: self.scope.tmp.saturating_add(1),
            max_scope: 1,
        }
    }
}

/// Compiles a function (declaration or literal) to a method and returns its index.
fn compile_function(abc: &mut Abc, f: &Func) -> u32 {
    let mut locals: Vec<(String, u32)> = Vec::new();
    let mut next = 1u32;
    for p in &f.params {
        locals.push((p.clone(), next));
        next = next.saturating_add(1);
    }
    let mut vars = Vec::new();
    declared_vars(&f.body, &mut vars);
    let mut nested = Vec::new();
    declared_functions(&f.body, &mut nested);
    vars.extend(nested.iter().filter_map(|n| n.name.clone()));
    for v in vars {
        if !locals.iter().any(|(n, _)| *n == v) {
            locals.push((v, next));
            next = next.saturating_add(1);
        }
    }
    let params = count(f.params.len());
    let mut body = Body::new(abc, Scope { locals, tmp: next });
    body.block(&f.body, false);
    let def = body.finish(params);
    abc.add_method(def)
}

/// One slot or method of the class.
enum Member {
    Field(u32),
    Method(u32, u32),
}

/// Compiles frame scripts, as `(0-based frame, statements)`, to an ABC file.
///
/// Returns the file and the notes about what could not be exported.
///
#[must_use]
pub fn compile(frames: &[(u32, &[Stmt])]) -> (Vec<u8>, Vec<String>) {
    let mut abc = Abc::default();

    // Fields and methods shared by the frame scripts.
    let mut field_names: Vec<String> = Vec::new();
    let mut functions: Vec<&Func> = Vec::new();
    for (_, stmts) in frames {
        declared_vars(stmts, &mut field_names);
        declared_functions(stmts, &mut functions);
    }
    let mut members: Vec<Member> = Vec::new();
    for name in &field_names {
        let mn = abc.public(name);
        members.push(Member::Field(mn));
    }
    let mut seen: Vec<&str> = Vec::new();
    for f in functions {
        let Some(name) = f.name.as_deref() else { continue };
        if seen.contains(&name) {
            abc.notes.push(format!("function {name} is declared more than once; the first declaration is used"));
            continue;
        }
        seen.push(name);
        let index = compile_function(&mut abc, f);
        let mn = abc.public(name);
        members.push(Member::Method(mn, index));
    }

    // One method per frame script.
    let mut frame_methods: Vec<(u32, u32)> = Vec::new();
    for (frame, stmts) in frames {
        let mut body = Body::new(&mut abc, Scope { locals: Vec::new(), tmp: 1 });
        body.block(stmts, true);
        let def = body.finish(0);
        let index = abc.add_method(def);
        let name = format!("frame{}", frame.saturating_add(1));
        let mn = abc.public(&name);
        frame_methods.push((*frame, mn));
        members.push(Member::Method(mn, index));
    }

    // Constructor: super(), then register the frame scripts.
    let add_frame_script = abc.lookup("addFrameScript");
    let mut ctor = Body::new(&mut abc, Scope { locals: Vec::new(), tmp: 1 });
    ctor.code.op(op::GET_LOCAL_0, 1);
    ctor.code.op_u30(op::CONSTRUCT_SUPER, 0, -1);
    for (frame, mn) in &frame_methods {
        ctor.code.op(op::GET_LOCAL_0, 1);
        ctor.number(f64::from(*frame));
        ctor.code.op(op::GET_LOCAL_0, 1);
        ctor.code.op_u30(op::GET_PROPERTY, *mn, 0);
        ctor.code.bytes.push(op::CALL_PROP_VOID);
        ctor.code.bytes.put_encoded_u32(add_frame_script);
        ctor.code.bytes.put_encoded_u32(2);
        ctor.code.adjust(-3);
    }
    let ctor = ctor.finish(0);
    let iinit = abc.add_method(ctor);

    // Static initialiser.
    let cinit = abc.add_method(MethodDef { params: 0, code: vec![op::GET_LOCAL_0, op::PUSH_SCOPE, op::RETURN_VOID], max_stack: 1, locals: 1, max_scope: 1 });

    // Script initialiser: declare the class.
    let class_name = abc.public(CLASS_NAME);
    let movie_clip = abc.packaged(0, "MovieClip");
    let mut boot = Code::new();
    boot.op(op::GET_LOCAL_0, 1);
    boot.op(op::PUSH_SCOPE, -1);
    boot.op(op::GET_SCOPE_OBJECT, 1);
    boot.bytes.push(0);
    boot.op_u30(op::GET_LEX, movie_clip, 1);
    boot.op(op::PUSH_SCOPE, -1);
    boot.op_u30(op::GET_LEX, movie_clip, 1);
    boot.op_u30(op::NEW_CLASS, 0, 0);
    boot.op(op::POP_SCOPE, 0);
    boot.op_u30(op::INIT_PROPERTY, class_name, -2);
    boot.op(op::RETURN_VOID, 0);
    let script_init =
        abc.add_method(MethodDef { params: 0, code: boot.bytes, max_stack: u32::try_from(boot.max).unwrap_or(0).saturating_add(1), locals: 1, max_scope: 2 });

    let package_strings: Vec<u32> = PACKAGES.iter().map(|p| abc.string(p)).collect();
    let bytes = write(&abc, &package_strings, &members, [class_name, movie_clip], [iinit, cinit, script_init]);
    (bytes, abc.notes)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.put_encoded_u32(count(s.len()));
    out.extend_from_slice(s.as_bytes());
}

/// The constant pool.
fn write_pool(abc: &Abc, package_strings: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let table = |out: &mut Vec<u8>, n: usize| out.put_encoded_u32(if n == 0 { 0 } else { count(n.saturating_add(1)) });
    table(&mut out, abc.ints.len());
    for v in &abc.ints {
        // A signed 32-bit integer is stored as the unsigned bit pattern.
        out.put_encoded_u32(u32::from_le_bytes(v.to_le_bytes()));
    }
    out.put_encoded_u32(0); // unsigned integers
    table(&mut out, abc.doubles.len());
    for v in &abc.doubles {
        out.extend_from_slice(&v.to_le_bytes());
    }
    table(&mut out, abc.strings.len());
    for s in &abc.strings {
        put_str(&mut out, s);
    }
    // Namespaces: the public one, then the packages. The public namespace is a package namespace with an empty name.
    out.put_encoded_u32(count(PACKAGES.len().saturating_add(2)));
    out.push(PACKAGE_NAMESPACE);
    out.put_encoded_u32(0);
    for name in package_strings {
        out.push(PACKAGE_NAMESPACE);
        out.put_encoded_u32(*name);
    }
    // One namespace set: all of them.
    out.put_encoded_u32(2);
    out.put_encoded_u32(count(PACKAGES.len().saturating_add(1)));
    for i in 0..=PACKAGES.len() {
        out.put_encoded_u32(count(i.saturating_add(1)));
    }
    // Multinames.
    out.put_encoded_u32(count(abc.multinames.len().saturating_add(1)));
    for m in &abc.multinames {
        match m {
            Multiname::Qualified(ns, name) => {
                out.push(0x07);
                out.put_encoded_u32(*ns);
                out.put_encoded_u32(*name);
            }
            Multiname::Lookup(name) => {
                out.push(0x09);
                out.put_encoded_u32(*name);
                out.put_encoded_u32(1);
            }
        }
    }

    out
}

/// The bytes of the file.
fn write(abc: &Abc, package_strings: &[u32], members: &[Member], [class_name, super_name]: [u32; 2], [iinit, cinit, script_init]: [u32; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_u16(16);
    out.put_u16(46);
    out.extend_from_slice(&write_pool(abc, package_strings));

    // Methods.
    out.put_encoded_u32(count(abc.methods.len()));
    for m in &abc.methods {
        out.put_encoded_u32(m.params);
        out.put_encoded_u32(0);
        for _ in 0..m.params {
            out.put_encoded_u32(0);
        }
        out.put_encoded_u32(0);
        out.push(0);
    }
    out.put_encoded_u32(0); // metadata

    // One class.
    out.put_encoded_u32(1);
    out.put_encoded_u32(class_name);
    out.put_encoded_u32(super_name);
    out.push(0); // dynamic, not final, not an interface
    out.put_encoded_u32(0); // interfaces
    out.put_encoded_u32(iinit);
    out.put_encoded_u32(count(members.len()));
    for m in members {
        match m {
            Member::Field(name) => {
                out.put_encoded_u32(*name);
                out.push(0); // Trait_Slot
                out.put_encoded_u32(0); // slot id
                out.put_encoded_u32(0); // type: any
                out.put_encoded_u32(0); // no initial value
            }
            Member::Method(name, method) => {
                out.put_encoded_u32(*name);
                out.push(1); // Trait_Method
                out.put_encoded_u32(0);
                out.put_encoded_u32(*method);
            }
        }
    }
    out.put_encoded_u32(cinit);
    out.put_encoded_u32(0);

    // One script holding the class.
    out.put_encoded_u32(1);
    out.put_encoded_u32(script_init);
    out.put_encoded_u32(1);
    out.put_encoded_u32(class_name);
    out.push(4); // Trait_Class
    out.put_encoded_u32(0);
    out.put_encoded_u32(0);

    // Method bodies.
    out.put_encoded_u32(count(abc.methods.len()));
    for (i, m) in abc.methods.iter().enumerate() {
        out.put_encoded_u32(count(i));
        out.put_encoded_u32(m.max_stack);
        out.put_encoded_u32(m.locals);
        out.put_encoded_u32(0);
        out.put_encoded_u32(m.max_scope);
        out.put_encoded_u32(count(m.code.len()));
        out.extend_from_slice(&m.code);
        out.put_encoded_u32(0); // exceptions
        out.put_encoded_u32(0); // traits
    }
    out
}
