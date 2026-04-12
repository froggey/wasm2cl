use clap::Parser as ClapParser;
use std::{fs, path::PathBuf, fmt::Write};

use anyhow::{bail, Context, Result};
use wasmparser::Parser;

const WASM_PAGE_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Type {
    I32, I64, F32, F64, V128, FuncRef, ExternRef,
}

#[derive(Debug, Clone)]
struct FuncType {
    params: Vec<Type>,
    results: Vec<Type>,
}

#[derive(Debug)]
struct Function {
    index: usize,
    ty: FuncType,
    name: Option<(String, String)>,
    body: Option<Body>,
    internal_name: Option<String>,
}

#[derive(Debug)]
struct Body {
    locals: Vec<Type>,
    code_bytes: Vec<u8>,
    code_offset: usize,
}

#[derive(Debug)]
struct Export {
    name: String,
    func_idx: usize,
}

#[derive(Debug)]
struct ActiveData {
    address: usize,
    data: Vec<u8>,
}

#[derive(Debug)]
struct Global {
    initializer: String,
}

#[derive(Debug)]
struct ActiveElement {
    address: usize,
    data: Vec<usize>, // function indices
}

#[derive(Debug)]
struct Module {
    memory_initial_size: usize,
    table_initial_size: usize,
    types: Vec<FuncType>,
    functions: Vec<Function>,
    exports: Vec<Export>,
    active_data: Vec<ActiveData>,
    active_elements: Vec<ActiveElement>,
    globals: Vec<Global>,
}

fn parse_type(ty: &wasmparser::ValType) -> Result<Type> {
    Ok(match ty {
        wasmparser::ValType::I32 => Type::I32,
        wasmparser::ValType::I64 => Type::I64,
        wasmparser::ValType::F32 => Type::F32,
        wasmparser::ValType::F64 => Type::F64,
        wasmparser::ValType::V128   => Type::V128,
        wasmparser::ValType::Ref(r) if r.is_func_ref()   => Type::FuncRef,
        wasmparser::ValType::Ref(r) if r.is_extern_ref() => Type::ExternRef,
        other => bail!("unsupported valtype: {other:?}"),
    })
}

fn parse(bytes: &[u8]) -> Result<Module> {
    use wasmparser::Payload::*;

    let mut types = vec![];
    let mut functions = vec![];
    let mut current_function = 0;
    let mut exports = vec![];
    let mut active_data = vec![];
    let mut active_elements = vec![];
    let mut memory_initial_size = 0;
    let mut globals = vec![];
    let mut table_initial_size = 0;

    for payload in Parser::new(0).parse_all(bytes) {
        match payload.context("malformed wasm payload")? {
            TypeSection(reader) => {
                println!("TypeSection");
                for group in reader {
                    let group = group?;
                    // wasmparser 0.219 wraps types in a SubType/RecGroup
                    for ty in group.types() {
                        if let wasmparser::CompositeInnerType::Func(ft) = &ty.composite_type.inner {
                            let parsed_ty = FuncType {
                                params:  ft.params().iter().map(parse_type).collect::<Result<_>>()?,
                                results: ft.results().iter().map(parse_type).collect::<Result<_>>()?,
                            };
                            println!(" {ty:?} => {parsed_ty:?}");
                            types.push(parsed_ty);
                        } else {
                            bail!("Unsupported type {ty:?}");
                        }
                    }
                }
            }
            ImportSection(reader) => {
                println!("ImportSection");
                for import in reader {
                    let import = import?;
                    println!(" {import:?}");
                    match import {
                        wasmparser::Imports::Single(_, wasmparser::Import { module, name, ty: wasmparser::TypeRef::Func(ty) }) => {
                            functions.push(Function {
                                index: functions.len(),
                                ty: types[ty as usize].clone(),
                                name: Some((module.to_string(), name.to_string())),
                                body: None,
                                internal_name: None,
                            });
                        }
                        import => bail!("Unsupported import {import:?}"),
                    }
                }
                println!("Imports: {functions:#?}");
            }
            FunctionSection(reader) => {
                //println!("FunctionSection");
                current_function = functions.len(); // Skip over imports
                for type_idx in reader {
                    let type_idx = type_idx?;
                    //println!(" {type_idx:?}");
                    functions.push(Function {
                        index: functions.len(),
                        ty: types[type_idx as usize].clone(),
                        name: None,
                        body: None,
                        internal_name: None,
                    });
                }
                //println!("Functions: {functions:#?}");
            }
            TableSection(reader) => {
                println!("TableSection");
                for table in reader {
                    let table = table?;
                    println!("  {table:?}");
                    table_initial_size = table.ty.maximum.unwrap_or(table.ty.initial) as usize;
                }
            }
            MemorySection(reader) => {
                println!("MemorySection");
                for mem in reader {
                    let mem = mem?;
                    assert!(!mem.memory64);
                    assert!(!mem.shared);
                    assert!(mem.page_size_log2.is_none());
                    println!(" {mem:?}");
                    memory_initial_size = (mem.initial as usize) * WASM_PAGE_SIZE;
                }
            }
            GlobalSection(reader) => {
                println!("GlobalSection");
                for global in reader {
                    let global = global?;
                    println!(" {global:?}");
                    let initform;
                    match global.ty.content_type {
                        wasmparser::ValType::I32 => {
                            let mut init_value = 0;
                            for op in global.init_expr.get_operators_reader() {
                                use wasmparser::Operator::*;
                                match op? {
                                    I32Const { value } => init_value = value,
                                    End => (),
                                    op => bail!("  Unsupported operator in offset_expr {op:?}!"),
                                }
                            }
                            initform = format!("{init_value}");
                        }
                        ty => bail!("Unsupported global type {ty:?}"),
                    }
                    globals.push(Global {
                        initializer: initform,
                    });
                }
            }
            ElementSection(reader) => {
                println!("ElementSection");
                for seg in reader {
                    let seg = seg?;
                    if let wasmparser::ElementKind::Active { table_index, offset_expr } = seg.kind {
                        if table_index.unwrap_or(0) != 0 {
                            bail!("Unsupported memory index {table_index:?}");
                        }
                        // Assume it's just i32const, end for now.
                        let mut address = 0;
                        for op in offset_expr.get_operators_reader() {
                            use wasmparser::Operator::*;
                            match op? {
                                I32Const { value } => address = value as usize,
                                End => (),
                                op => bail!("  Unsupported operator in offset_expr {op:?}!"),
                            }
                        }
                        if let wasmparser::ElementItems::Functions(reader) = seg.items {
                            let mut values = vec![];
                            for val in reader {
                                let val = val?;
                                values.push(val as usize);
                            }
                            active_elements.push(ActiveElement {
                                address,
                                data: values,
                            });
                        } else {
                            bail!("Unsupported element segment kind");
                        }
                        //active_data.push(ActiveData { address, data: seg.data.to_owned() });
                    } else {
                        bail!("Unsupported element segment kind");
                    }
                    //println!(" {seg:?}");
                }
            }
            ExportSection(reader) => {
                //println!("ExportSection");
                for export in reader {
                    let export = export?;
                    let idx = export.index as usize;
                    if export.kind == wasmparser::ExternalKind::Func {
                        if functions[idx].name.is_none() {
                            functions[idx].name = Some((String::new(), export.name.to_string()));
                        }
                        exports.push(Export {
                            name: export.name.to_string(),
                            func_idx: idx,
                        });
                    }
                    //println!(" {export:?}");
                }
                //println!("Exports: {exports:#?}");
            }
            StartSection { func, .. } => {
                println!("Start Section {func:?}");
            }
            CodeSectionEntry(body) => {
                let mut locals = vec![];
                let locals_reader = body.get_locals_reader()?;
                for local in locals_reader {
                    let (count, ty) = local?;
                    let ty = parse_type(&ty)?;
                    for _i in 0..count {
                        locals.push(ty);
                    }
                }

                let ops_reader = body.get_operators_reader()?;
                let mut ops_bin_reader = ops_reader.get_binary_reader();
                let code_offset = ops_bin_reader.original_position();
                let code_bytes = ops_bin_reader.read_bytes(
                    ops_bin_reader.bytes_remaining()
                )?.to_vec();

                functions[current_function].body = Some(Body {
                    locals,
                    code_bytes,
                    code_offset,
                });
                current_function += 1;
            }
            DataSection(reader) => {
                println!("DataSection");
                for seg in reader {
                    let seg = seg?;
                    if let wasmparser::DataKind::Active { memory_index, offset_expr } = seg.kind {
                        println!("ActiveSeg {} bytes", seg.data.len());
                        if memory_index != 0 {
                            bail!("Unsupported memory index {memory_index}");
                        }
                        // Assume it's just i32const, end for now.
                        let mut address = 0;
                        for op in offset_expr.get_operators_reader() {
                            use wasmparser::Operator::*;
                            match op? {
                                I32Const { value } => address = value as usize,
                                End => (),
                                op => bail!("  Unsupported operator in offset_expr {op:?}!"),
                            }
                        }
                        active_data.push(ActiveData { address, data: seg.data.to_owned() });
                    } else {
                        bail!("Unsupported data segment kind {:?}", seg.kind);
                    }
                    //println!(" {seg:?}");
                }
            }
            End(_) => break,
            CustomSection(r) => {
                match r.as_known() {
                    wasmparser::KnownCustom::Name(reader) => {
                        for subsection in reader {
                            match subsection? {
                                wasmparser::Name::Function(map) => {
                                    println!("FunctionNameSection");
                                    for naming in map {
                                        let naming = naming?;
                                        functions[naming.index as usize].internal_name = Some(naming.name.to_string());
                                    }
                                }
                                _ => println!("Unknown name section"),
                            }
                        }
                    }
                    _ => println!("Unknown custom section {r:?}"),
                }
            }
            s => {
                // version header, custom sections, etc.
                println!("Unknown section {s:?}");
            }
        }
    }

    Ok(Module {
        memory_initial_size,
        table_initial_size,
        types,
        functions,
        exports,
        active_data,
        active_elements,
        globals,
    })
}

fn symbolicate(s: &str) -> String {
    format!("|{s}|")
}

fn convert_type(t: Type) -> &'static str {
    match t {
        Type::I32 => "i32",
        Type::I64 => "i64",
        Type::F32 => "f32",
        Type::F64 => "f64",
        Type::V128 => "v128",
        Type::FuncRef => "func-ref",
        Type::ExternRef => "extern-ref",
    }
}

fn initializer_for_type(t: Type) -> &'static str {
    match t {
        Type::I32 => "0",
        Type::I64 => "0",
        Type::F32 => "0.0f0",
        Type::F64 => "0.0d0",
        Type::V128 => "0",
        Type::FuncRef => "nil",
        Type::ExternRef => "nil",
    }
}

fn convert_parameters(params: &[Type]) -> String {
    let mut output = String::new();
    for (i, p) in params.iter().enumerate() {
        if !output.is_empty() {
            output.push_str(" ");
        }
        output.push_str(&format!("(param-{i} {})", convert_type(*p)));
    }
    output
}

impl Function {
    fn name(&self) -> String {
        if let Some(name) = self.internal_name.as_ref() {
            format!("wasm-{}-{}", self.index, symbolicate(name))
        } else if let Some((module, name)) = self.name.as_ref() {
            // Import
            format!("wasm-import-{}-{}-{}",
                    symbolicate(&module), symbolicate(&name), self.index)
        } else {
            // Internal function
            format!("wasm-function-{}", self.index)
        }
    }
}

#[derive(Debug)]
enum Primitive {
    Unreachable,
    I32Eq,
    I32Ne,
    I32Eqz,
    I32LeU,
    I32LtU,
    I32GeU,
    I32GtU,
    I32LeS,
    I32LtS,
    I32GeS,
    I32GtS,
    I32Add,
    I32Sub,
    I32Mul,
    I32DivU,
    I32DivS,
    I32RemU,
    I32RemS,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrU,
    I32ShrS,
    I32Rotl,
    I32Rotr,
    I32Clz,
    I32Ctz,
    I32WrapI64,
    I32Extend8S,
    I32Extend16S,
    I64Eq,
    I64Ne,
    I64Eqz,
    I64LeU,
    I64LtU,
    I64GeU,
    I64GtU,
    I64LeS,
    I64LtS,
    I64GeS,
    I64GtS,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivU,
    I64DivS,
    I64RemU,
    I64RemS,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrU,
    I64ShrS,
    I64Rotl,
    I64Rotr,
    I64Clz,
    I64Ctz,
    I64Extend8S,
    I64Extend16S,
    I64Extend32S,
    F32Eq,
    F32Ne,
    F32Le,
    F32Lt,
    F32Ge,
    F32Gt,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32ConvertI32U,
    F32ConvertI32S,
    F32ConvertI64U,
    F32ConvertI64S,
    F64Eq,
    F64Ne,
    F64Le,
    F64Lt,
    F64Ge,
    F64Gt,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64ConvertI32U,
    F64ConvertI32S,
    F64ConvertI64U,
    F64ConvertI64S,
}

#[derive(Debug)]
enum Expr {
    Const(String),
    Global(usize),
    GlobalSet(usize, Box<Expr>),
    Local(String),
    Setf(String, Box<Expr>),
    Call(String, Vec<Expr>),
    CallIndirect(Box<Expr>, Vec<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Select(Box<Expr>, Box<Expr>, Box<Expr>),
    Progn(Vec<Expr>),
    Prog1(Box<Expr>, Vec<Expr>),
    Prim(Primitive, Vec<Expr>),
    Block(String, Box<Expr>),
    ReturnFrom(String, Box<Expr>),
    Tagbody(String, Box<Expr>),
    Go(String),
    Switch(Box<Expr>, Box<Expr>, Vec<Expr>),
    I32Load(Option<(usize, bool)>, Box<Expr>, usize),
    I32Store(Option<usize>, Box<Expr>, Box<Expr>, usize),
    I64Load(Option<(usize, bool)>, Box<Expr>, usize),
    I64Store(Option<usize>, Box<Expr>, Box<Expr>, usize),
    F32Load(Box<Expr>, usize),
    F32Store(Box<Expr>, Box<Expr>, usize),
    F64Load(Box<Expr>, usize),
    F64Store(Box<Expr>, Box<Expr>, usize),
}

impl Expr {
    fn fused_pred(&self) -> Option<(&'static str, &[Expr])> {
        match self {
            Expr::Prim(Primitive::I32Eqz, args) => Some(("i32eqz.fused", args)),
            Expr::Prim(Primitive::I32Eq, args) => Some(("i32eq.fused", args)),
            Expr::Prim(Primitive::I32Ne, args) => Some(("i32ne.fused", args)),
            Expr::Prim(Primitive::I32LeU, args) => Some(("i32leu.fused", args)),
            Expr::Prim(Primitive::I32LtU, args) => Some(("i32ltu.fused", args)),
            Expr::Prim(Primitive::I32GeU, args) => Some(("i32geu.fused", args)),
            Expr::Prim(Primitive::I32GtU, args) => Some(("i32gtu.fused", args)),
            Expr::Prim(Primitive::I32LeS, args) => Some(("i32les.fused", args)),
            Expr::Prim(Primitive::I32LtS, args) => Some(("i32lts.fused", args)),
            Expr::Prim(Primitive::I32GeS, args) => Some(("i32ges.fused", args)),
            Expr::Prim(Primitive::I32GtS, args) => Some(("i32gts.fused", args)),
            Expr::Prim(Primitive::I64Eqz, args) => Some(("i64eqz.fused", args)),
            Expr::Prim(Primitive::I64Eq, args) => Some(("i64eq.fused", args)),
            Expr::Prim(Primitive::I64Ne, args) => Some(("i64ne.fused", args)),
            Expr::Prim(Primitive::I64LeU, args) => Some(("i64leu.fused", args)),
            Expr::Prim(Primitive::I64LtU, args) => Some(("i64ltu.fused", args)),
            Expr::Prim(Primitive::I64GeU, args) => Some(("i64geu.fused", args)),
            Expr::Prim(Primitive::I64GtU, args) => Some(("i64gtu.fused", args)),
            Expr::Prim(Primitive::I64LeS, args) => Some(("i64les.fused", args)),
            Expr::Prim(Primitive::I64LtS, args) => Some(("i64lts.fused", args)),
            Expr::Prim(Primitive::I64GeS, args) => Some(("i64ges.fused", args)),
            Expr::Prim(Primitive::I64GtS, args) => Some(("i64gts.fused", args)),
            Expr::Prim(Primitive::F32Eq, args) => Some(("f32eq.fused", args)),
            Expr::Prim(Primitive::F32Ne, args) => Some(("f32ne.fused", args)),
            Expr::Prim(Primitive::F32Le, args) => Some(("f32le.fused", args)),
            Expr::Prim(Primitive::F32Lt, args) => Some(("f32lt.fused", args)),
            Expr::Prim(Primitive::F32Ge, args) => Some(("f32ge.fused", args)),
            Expr::Prim(Primitive::F32Gt, args) => Some(("f32gt.fused", args)),
            Expr::Prim(Primitive::F64Eq, args) => Some(("f64eq.fused", args)),
            Expr::Prim(Primitive::F64Ne, args) => Some(("f64ne.fused", args)),
            Expr::Prim(Primitive::F64Le, args) => Some(("f64le.fused", args)),
            Expr::Prim(Primitive::F64Lt, args) => Some(("f64lt.fused", args)),
            Expr::Prim(Primitive::F64Ge, args) => Some(("f64ge.fused", args)),
            Expr::Prim(Primitive::F64Gt, args) => Some(("f64gt.fused", args)),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum BlockKind {
    Block,
    If,
    Loop,
}

#[derive(Debug)]
struct ActiveBlock {
    kind: BlockKind,
    blockty: wasmparser::BlockType,
    old_exprs: Vec<Expr>,
    then: Option<Vec<Expr>>,
    name: String,
    targeted: bool,
    stack: Vec<Expr>,
}

fn append_side_effect(exprs: &mut Vec<Expr>, stack: &mut Vec<Expr>, new: Expr) {
    if stack.is_empty() {
        exprs.push(new);
    } else {
        let last = stack.len()-1;
        if let Expr::Prog1(_, exprs) = &mut stack[last] {
            exprs.push(new);
        } else {
            let value = stack.pop().unwrap();
            stack.push(Expr::Prog1(Box::new(value), vec![new]));
        }
    }
}

fn prim_op1(prim: Primitive, stack: &mut Vec<Expr>) {
    let val = stack.pop().unwrap();
    stack.push(Expr::Prim(prim, vec![val]));
}

fn prim_op2(prim: Primitive, stack: &mut Vec<Expr>) {
    let rhs = stack.pop().unwrap();
    let lhs = stack.pop().unwrap();
    stack.push(Expr::Prim(prim, vec![lhs, rhs]));
}

// This tries very hard to reconstruct expression trees from the bytecode.
// We could just turn the stack into a sequence of assignments to stack slots,
// but CL compilers don't really like that.
fn expressionify_function_body(module: &Module, func: &Function, all_locals: &[(String, Type)], ops: &mut wasmparser::OperatorsReader) -> Result<Vec<Expr>> {
    use wasmparser::Operator::*;

    let mut unreachable = false;
    let mut unreachable_depth = 0;
    let mut pc = 0;
    let mut stack = Vec::new();
    let mut exprs = Vec::new();
    let mut block_stack = vec![];

    loop {
        let op = ops.read()?;
        //println!("{op:?}  {stack:?}  {exprs:?}  {block_stack:?}  {unreachable}");
        match op {
            If { .. } if unreachable => {
                unreachable_depth += 1;
            }
            Block { .. } if unreachable => {
                unreachable_depth += 1;
            }
            Loop { .. } if unreachable => {
                unreachable_depth += 1;
            }
            Else if unreachable && unreachable_depth != 0 => { }
            End if unreachable && unreachable_depth != 0 => {
                unreachable_depth -= 1;
            }

            Block { blockty } => {
                block_stack.push(ActiveBlock {
                    kind: BlockKind::Block,
                    blockty,
                    old_exprs: std::mem::take(&mut exprs),
                    then: None,
                    name: format!("block-{pc}"),
                    targeted: false,
                    stack: std::mem::take(&mut stack),
                });
            }
            Loop { blockty } => {
                block_stack.push(ActiveBlock {
                    kind: BlockKind::Loop,
                    blockty,
                    old_exprs: std::mem::take(&mut exprs),
                    then: None,
                    name: format!("loop-{pc}"),
                    targeted: false,
                    stack: std::mem::take(&mut stack),
                });
            }
            If { blockty } => {
                block_stack.push(ActiveBlock {
                    kind: BlockKind::If,
                    blockty,
                    old_exprs: std::mem::take(&mut exprs),
                    then: None,
                    name: format!("if-{pc}"),
                    targeted: false,
                    stack: std::mem::take(&mut stack),
                });
            }
            Else => {
                unreachable = false;
                let idx = block_stack.len()-1;
                assert!(matches!(block_stack[idx].kind, BlockKind::If));
                assert!(block_stack[idx].then.is_none());
                if !matches!(block_stack[idx].blockty, wasmparser::BlockType::Empty) {
                    exprs.push(stack.pop().unwrap());
                }
                assert!(stack.is_empty());
                block_stack[idx].then = Some(std::mem::take(&mut exprs));
            }
            End => {
                let Some(entry) = block_stack.pop() else {
                    // End of the function
                    break;
                };
                unreachable = false;
                // End of the current block.
                if !matches!(entry.blockty, wasmparser::BlockType::Empty) {
                    exprs.push(stack.pop().unwrap());
                }
                assert!(stack.is_empty());
                stack = entry.stack;
                let mut final_expr = match entry.kind {
                    BlockKind::If => {
                        let (mut then, mut els) = if let Some(then_exprs) = entry.then {
                            (then_exprs, std::mem::replace(&mut exprs, entry.old_exprs))
                        } else {
                            (std::mem::replace(&mut exprs, entry.old_exprs), vec![])
                        };
                        let test = stack.pop().unwrap();
                        Expr::If(
                            Box::new(test),
                            Box::new(if then.len() == 1 {
                                then.pop().unwrap()
                            } else {
                                Expr::Progn(then)
                            }),
                            Box::new(if els.len() == 1 {
                                els.pop().unwrap()
                            } else {
                                Expr::Progn(els)
                            }))
                    }
                    BlockKind::Block => {
                        Expr::Progn(std::mem::replace(&mut exprs, entry.old_exprs))
                    }
                    BlockKind::Loop => {
                        Expr::Progn(std::mem::replace(&mut exprs, entry.old_exprs))
                    }
                };
                if entry.targeted {
                    if matches!(entry.kind, BlockKind::Loop) {
                        final_expr = Expr::Tagbody(
                            entry.name,
                            Box::new(final_expr));
                    } else {
                        final_expr = Expr::Block(
                            entry.name,
                            Box::new(final_expr));
                    }
                }
                if matches!(entry.blockty, wasmparser::BlockType::Empty) {
                    append_side_effect(&mut exprs, &mut stack, final_expr);
                } else {
                    stack.push(final_expr);
                }
            }

            _ if unreachable => { }, // Nothing

            // Normal execution.
            I32Const { value } => {
                stack.push(Expr::Const(format!("{}", value as u32)));
            }
            I64Const { value } => {
                stack.push(Expr::Const(format!("{}", value as u64)));
            }
            F32Const { value } => {
                stack.push(Expr::Const(format!("(f32const {})", value.bits())));
            }
            F64Const { value } => {
                stack.push(Expr::Const(format!("(f64const {})", value.bits())));
            }
            GlobalGet { global_index } => {
                stack.push(Expr::Global(global_index as usize));
            }
            GlobalSet { global_index } => {
                let value = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::GlobalSet(global_index as usize,
                                                   Box::new(value)));
            }
            LocalGet { local_index } => {
                stack.push(Expr::Local(all_locals[local_index as usize].0.clone()));
            }
            LocalSet { local_index } => {
                let value = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::Setf(all_locals[local_index as usize].0.clone(), Box::new(value)));
            }
            LocalTee { local_index } => {
                let value = stack.pop().unwrap();
                stack.push(Expr::Setf(all_locals[local_index as usize].0.clone(), Box::new(value)));
            }
            Drop => {
                let value = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack, value);
            }
            Unreachable => {
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::Prim(Primitive::Unreachable, vec![]));
                unreachable = true;
            }
            Return => {
                let value = if func.ty.results.is_empty() {
                    Expr::Progn(vec![])
                } else {
                    stack.pop().unwrap()
                };
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::ReturnFrom("nil".to_string(),
                                                    Box::new(value)));
                unreachable = true;
            }
            Call { function_index } => {
                let target = &module.functions[function_index as usize];
                let args = stack.split_off(stack.len()-target.ty.params.len());
                match target.ty.results.len() {
                    // call for effect
                    0 => append_side_effect(&mut exprs, &mut stack, Expr::Call(target.name(), args)),
                    // call for single value
                    1 => stack.push(Expr::Call(target.name(), args)),
                    _ => unimplemented!("call producing multiple values"),
                }
            }
            CallIndirect { type_index, table_index } => {
                assert!(table_index == 0);
                let ty = &module.types[type_index as usize];
                let idx = stack.pop().unwrap();
                let args = stack.split_off(stack.len()-ty.params.len());
                match ty.results.len() {
                    // call for effect
                    0 => append_side_effect(&mut exprs, &mut stack, Expr::CallIndirect(Box::new(idx), args)),
                    // call for single value
                    1 => stack.push(Expr::CallIndirect(Box::new(idx), args)),
                    _ => unimplemented!("call producing multiple values"),
                }
            }
            Br { relative_depth } => {
                let target = block_stack.len()-1-(relative_depth as usize);
                unreachable = true;
                block_stack[target].targeted = true;
                append_side_effect(&mut exprs, &mut stack,
                                   if matches!(block_stack[target].kind, BlockKind::Loop) {
                                       Expr::Go(block_stack[target].name.clone())
                                   } else {
                                       Expr::ReturnFrom(block_stack[target].name.clone(),
                                                        Box::new(Expr::Progn(vec![])))
                                   });
            }
            BrIf { relative_depth } => {
                let target = block_stack.len()-1-(relative_depth as usize);
                let test = stack.pop().unwrap();
                block_stack[target].targeted = true;
                let value = if !matches!(block_stack[target].blockty, wasmparser::BlockType::Empty) {
                    stack.pop().unwrap()
                } else {
                    Expr::Progn(vec![])
                };
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::If(
                                       Box::new(test),
                                       Box::new(
                                           if matches!(block_stack[target].kind, BlockKind::Loop) {
                                               Expr::Go(block_stack[target].name.clone())
                                           } else {
                                               Expr::ReturnFrom(block_stack[target].name.clone(),
                                                                Box::new(value))
                                           }),
                                       Box::new(Expr::Progn(vec![]))));
            }
            BrTable { targets } => {
                let idx = stack.pop().unwrap();
                let mut target_code = vec![];
                for target_idx in targets.targets() {
                    let target_idx = target_idx?;
                    let target = block_stack.len()-1-(target_idx as usize);
                    assert!(matches!(block_stack[target].blockty, wasmparser::BlockType::Empty));
                    block_stack[target].targeted = true;
                    target_code.push(
                        if matches!(block_stack[target].kind, BlockKind::Loop) {
                            Expr::Go(block_stack[target].name.clone())
                        } else {
                            Expr::ReturnFrom(block_stack[target].name.clone(),
                                             Box::new(Expr::Progn(vec![])))
                        });
                }
                let default_target = block_stack.len()-1-(targets.default() as usize);
                assert!(matches!(block_stack[default_target].blockty, wasmparser::BlockType::Empty));
                block_stack[default_target].targeted = true;
                let default_code = if matches!(block_stack[default_target].kind, BlockKind::Loop) {
                    Expr::Go(block_stack[default_target].name.clone())
                } else {
                    Expr::ReturnFrom(block_stack[default_target].name.clone(),
                                     Box::new(Expr::Progn(vec![])))
                };
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::Switch(Box::new(idx),
                                                Box::new(default_code),
                                                target_code));
            }
            Select => {
                let cond = stack.pop().unwrap();
                let rhs = stack.pop().unwrap();
                let lhs = stack.pop().unwrap();
                stack.push(Expr::Select(
                    Box::new(lhs),
                    Box::new(rhs),
                    Box::new(cond)));
            }
            MemoryCopy { dst_mem, src_mem } => {
                assert!(dst_mem == 0);
                assert!(src_mem == 0);
                let n = stack.pop().unwrap();
                let src = stack.pop().unwrap();
                let dst = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::Call("memory-copy".to_string(),
                                              vec![dst, src, n]));
            }
            MemoryFill { mem } => {
                assert!(mem == 0);
                let n = stack.pop().unwrap();
                let value = stack.pop().unwrap();
                let dst = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::Call("memory-fill".to_string(),
                                              vec![dst, value, n]));
            }
            MemorySize { mem } => {
                assert!(mem == 0);
                stack.push(Expr::Call("memory-size".to_string(), vec![]));
            }
            MemoryGrow { mem } => {
                assert!(mem == 0);
                let n = stack.pop().unwrap();
                stack.push(Expr::Call("memory-grow".to_string(), vec![n]));
            }
            /* I32 */
            I32Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(None, Box::new(addr), memarg.offset as usize));
            }
            I32Load8U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(Some((8, false)), Box::new(addr), memarg.offset as usize));
            }
            I32Load8S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(Some((8, true)), Box::new(addr), memarg.offset as usize));
            }
            I32Load16U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(Some((16, false)), Box::new(addr), memarg.offset as usize));
            }
            I32Load16S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(Some((16, true)), Box::new(addr), memarg.offset as usize));
            }
            I32Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I32Store(None, Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I32Store8 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I32Store(Some(8), Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I32Store16 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I32Store(Some(16), Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I32Eqz => prim_op1(Primitive::I32Eqz, &mut stack),
            I32Eq => prim_op2(Primitive::I32Eq, &mut stack),
            I32Ne => prim_op2(Primitive::I32Ne, &mut stack),
            I32LeU => prim_op2(Primitive::I32LeU, &mut stack),
            I32LtU => prim_op2(Primitive::I32LtU, &mut stack),
            I32GeU => prim_op2(Primitive::I32GeU, &mut stack),
            I32GtU => prim_op2(Primitive::I32GtU, &mut stack),
            I32LeS => prim_op2(Primitive::I32LeS, &mut stack),
            I32LtS => prim_op2(Primitive::I32LtS, &mut stack),
            I32GeS => prim_op2(Primitive::I32GeS, &mut stack),
            I32GtS => prim_op2(Primitive::I32GtS, &mut stack),
            I32Add => prim_op2(Primitive::I32Add, &mut stack),
            I32Sub => prim_op2(Primitive::I32Sub, &mut stack),
            I32Mul => prim_op2(Primitive::I32Mul, &mut stack),
            I32DivU => prim_op2(Primitive::I32DivU, &mut stack),
            I32DivS => prim_op2(Primitive::I32DivS, &mut stack),
            I32RemU => prim_op2(Primitive::I32RemU, &mut stack),
            I32RemS => prim_op2(Primitive::I32RemS, &mut stack),
            I32And => prim_op2(Primitive::I32And, &mut stack),
            I32Or => prim_op2(Primitive::I32Or, &mut stack),
            I32Xor => prim_op2(Primitive::I32Xor, &mut stack),
            I32Shl => prim_op2(Primitive::I32Shl, &mut stack),
            I32ShrU => prim_op2(Primitive::I32ShrU, &mut stack),
            I32ShrS => prim_op2(Primitive::I32ShrS, &mut stack),
            I32Rotl => prim_op2(Primitive::I32Rotl, &mut stack),
            I32Rotr => prim_op2(Primitive::I32Rotr, &mut stack),
            I32Clz => prim_op1(Primitive::I32Clz, &mut stack),
            I32Ctz => prim_op1(Primitive::I32Ctz, &mut stack),
            I32WrapI64 => prim_op1(Primitive::I32WrapI64, &mut stack),
            I32Extend8S => prim_op1(Primitive::I32Extend8S, &mut stack),
            I32Extend16S => prim_op1(Primitive::I32Extend16S, &mut stack),
            /* I64 */
            I64Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(None, Box::new(addr), memarg.offset as usize));
            }
            I64Load8U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((8, false)), Box::new(addr), memarg.offset as usize));
            }
            I64Load8S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((8, true)), Box::new(addr), memarg.offset as usize));
            }
            I64Load16U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((16, false)), Box::new(addr), memarg.offset as usize));
            }
            I64Load16S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((16, true)), Box::new(addr), memarg.offset as usize));
            }
            I64Load32U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((32, false)), Box::new(addr), memarg.offset as usize));
            }
            I64Load32S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(Some((32, true)), Box::new(addr), memarg.offset as usize));
            }
            I64Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I64Store(None, Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I64Store8 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I64Store(Some(8), Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I64Store16 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I64Store(Some(16), Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I64Store32 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::I64Store(Some(32), Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            I64Eqz => prim_op1(Primitive::I64Eqz, &mut stack),
            I64Eq => prim_op2(Primitive::I64Eq, &mut stack),
            I64Ne => prim_op2(Primitive::I64Ne, &mut stack),
            I64LeU => prim_op2(Primitive::I64LeU, &mut stack),
            I64LtU => prim_op2(Primitive::I64LtU, &mut stack),
            I64GeU => prim_op2(Primitive::I64GeU, &mut stack),
            I64GtU => prim_op2(Primitive::I64GtU, &mut stack),
            I64LeS => prim_op2(Primitive::I64LeS, &mut stack),
            I64LtS => prim_op2(Primitive::I64LtS, &mut stack),
            I64GeS => prim_op2(Primitive::I64GeS, &mut stack),
            I64GtS => prim_op2(Primitive::I64GtS, &mut stack),
            I64Add => prim_op2(Primitive::I64Add, &mut stack),
            I64Sub => prim_op2(Primitive::I64Sub, &mut stack),
            I64Mul => prim_op2(Primitive::I64Mul, &mut stack),
            I64DivU => prim_op2(Primitive::I64DivU, &mut stack),
            I64DivS => prim_op2(Primitive::I64DivS, &mut stack),
            I64RemU => prim_op2(Primitive::I64RemU, &mut stack),
            I64RemS => prim_op2(Primitive::I64RemS, &mut stack),
            I64And => prim_op2(Primitive::I64And, &mut stack),
            I64Or => prim_op2(Primitive::I64Or, &mut stack),
            I64Xor => prim_op2(Primitive::I64Xor, &mut stack),
            I64Shl => prim_op2(Primitive::I64Shl, &mut stack),
            I64ShrU => prim_op2(Primitive::I64ShrU, &mut stack),
            I64ShrS => prim_op2(Primitive::I64ShrS, &mut stack),
            I64Rotl => prim_op2(Primitive::I64Rotl, &mut stack),
            I64Rotr => prim_op2(Primitive::I64Rotr, &mut stack),
            I64Clz => prim_op1(Primitive::I64Clz, &mut stack),
            I64Ctz => prim_op1(Primitive::I64Ctz, &mut stack),
            I64Extend8S => prim_op1(Primitive::I64Extend8S, &mut stack),
            I64Extend16S => prim_op1(Primitive::I64Extend16S, &mut stack),
            I64Extend32S => prim_op1(Primitive::I64Extend32S, &mut stack),
            I64ExtendI32S => prim_op1(Primitive::I64Extend32S, &mut stack),
            I64ExtendI32U => { /* Noop */ }
            /* F32 */
            F32Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::F32Load(Box::new(addr), memarg.offset as usize));
            }
            F32Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::F32Store(Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            F32Eq => prim_op2(Primitive::F32Eq, &mut stack),
            F32Ne => prim_op2(Primitive::F32Ne, &mut stack),
            F32Le => prim_op2(Primitive::F32Le, &mut stack),
            F32Lt => prim_op2(Primitive::F32Lt, &mut stack),
            F32Ge => prim_op2(Primitive::F32Ge, &mut stack),
            F32Gt => prim_op2(Primitive::F32Gt, &mut stack),
            F32Add => prim_op2(Primitive::F32Add, &mut stack),
            F32Sub => prim_op2(Primitive::F32Sub, &mut stack),
            F32Mul => prim_op2(Primitive::F32Mul, &mut stack),
            F32Div => prim_op2(Primitive::F32Div, &mut stack),
            F32ConvertI32U => prim_op1(Primitive::F32ConvertI32U, &mut stack),
            F32ConvertI32S => prim_op1(Primitive::F32ConvertI32S, &mut stack),
            F32ConvertI64U => prim_op1(Primitive::F32ConvertI64U, &mut stack),
            F32ConvertI64S => prim_op1(Primitive::F32ConvertI64S, &mut stack),
            /* F64 */
            F64Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::F64Load(Box::new(addr), memarg.offset as usize));
            }
            F64Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack,
                                   Expr::F64Store(Box::new(addr), Box::new(value), memarg.offset as usize));
            }
            F64Eq => prim_op2(Primitive::F64Eq, &mut stack),
            F64Ne => prim_op2(Primitive::F64Ne, &mut stack),
            F64Le => prim_op2(Primitive::F64Le, &mut stack),
            F64Lt => prim_op2(Primitive::F64Lt, &mut stack),
            F64Ge => prim_op2(Primitive::F64Ge, &mut stack),
            F64Gt => prim_op2(Primitive::F64Gt, &mut stack),
            F64Add => prim_op2(Primitive::F64Add, &mut stack),
            F64Sub => prim_op2(Primitive::F64Sub, &mut stack),
            F64Mul => prim_op2(Primitive::F64Mul, &mut stack),
            F64Div => prim_op2(Primitive::F64Div, &mut stack),
            F64ConvertI32U => prim_op1(Primitive::F64ConvertI32U, &mut stack),
            F64ConvertI32S => prim_op1(Primitive::F64ConvertI32S, &mut stack),
            F64ConvertI64U => prim_op1(Primitive::F64ConvertI64U, &mut stack),
            F64ConvertI64S => prim_op1(Primitive::F64ConvertI64S, &mut stack),
            op => unimplemented!("{op:?}"),
        }
        pc += 1;
    }

    if !unreachable {
        if !func.ty.results.is_empty() {
            exprs.push(stack.pop().unwrap())
        }

        assert!(stack.is_empty());
    }

    return Ok(exprs);
}

fn make_indent(indent: usize) -> String {
    let mut result = String::new();
    for _ in 0..indent {
        result.push(' ');
    }
    result
}

fn convert_expr(expr: &Expr, indent: usize) -> String {
    use Expr::*;

    match expr {
        Const(s) => s.clone(),
        Global(idx) => format!("(global context {idx})"),
        GlobalSet(idx, value) =>
            format!("(setf (global context {idx}) {})",
                    convert_expr(value, indent+(format!("(setf (global context {idx}) ").len()))),
        Local(s) => s.clone(),
        Setf(name, value) =>
            format!("(setf {name} {})",
                    convert_expr(value, indent+(format!("(setf {name} ").len()))),
        Call(name, args) => {
            let mut result = String::new();
            let mut indent = indent;
            result.push_str("(");
            result.push_str(name);
            result.push_str(" context");
            indent += result.len();
            for e in args.iter() {
                indent += 1;
                result.push_str(" ");
                let s = convert_expr(e, indent);
                indent += s.len();
                result.push_str(&s);
            }
            result.push_str(")");
            result
        }
        CallIndirect(idx, args) => {
            let mut result = String::new();
            let mut indent = indent;
            // Need to be careful with order-of-evaluation here.
            // Argument index is evaluated first, not last, which
            // messes with the natural order we want arguments to be
            // in.
            let mut temps = vec![];
            for i in 0..args.len() {
                temps.push(format!("call-temp-{i}"));
            }
            result.push_str("(let (");
            indent += 6;
            for (i, (temp, val)) in std::iter::zip(temps.iter(), args.iter()).enumerate() {
                if i != 0 {
                    result.push_str("\n");
                    result.push_str(&make_indent(indent));
                }
                result.push_str(&format!("({temp} "));
                result.push_str(&convert_expr(val, indent + 2 + temp.len()));
                result.push_str(")");
            }
            result.push_str(")\n");
            result.push_str(&make_indent(indent+2));
            result.push_str(" (call-indirect ");
            result.push_str(&convert_expr(idx, indent+4));
            result.push_str(" context");
            for e in temps.iter() {
                result.push_str(" ");
                result.push_str(&e);
            }
            result.push_str("))");
            result
        }
        Prim(name, args) => {
            let mut result = String::new();
            let mut indent = indent;
            result.push_str("(");
            result.push_str(&format!("{name:?}"));
            indent += result.len();
            for e in args.iter() {
                indent += 1;
                result.push_str(" ");
                let s = convert_expr(e, indent);
                indent += s.len();
                result.push_str(&s);
            }
            result.push_str(")");
            result
        }
        If(test, tru, fals) => {
            let mut result = String::new();
            result.push_str("(if ");
            if let Some((fused_name, fused_args)) = test.fused_pred() {
                result.push_str("(");
                result.push_str(fused_name);
                for a in fused_args.iter() {
                    result.push_str(" ");
                    result.push_str(&convert_expr(a, indent));
                }
                result.push_str(")\n");
            } else {
                result.push_str("(not (zerop ");
                result.push_str(&convert_expr(test, indent));
                result.push_str("))\n");
            }
            result.push_str(&make_indent(indent+4));
            result.push_str(&convert_expr(tru, indent+4));
            result.push_str("\n");
            result.push_str(&make_indent(indent+4));
            result.push_str(&convert_expr(fals, indent+4));
            result.push_str(")");
            result
        }
        Select(tru, fals, test) => {
            let mut result = String::new();
            result.push_str("(select ");
            result.push_str(&convert_expr(tru, indent));
            result.push_str(" ");
            result.push_str(&convert_expr(fals, indent));
            result.push_str(" ");
            if let Some((fused_name, fused_args)) = test.fused_pred() {
                result.push_str("(");
                result.push_str(fused_name);
                for a in fused_args.iter() {
                    result.push_str(" ");
                    result.push_str(&convert_expr(a, indent));
                }
                result.push_str("))");
            } else {
                result.push_str("(not (zerop ");
                result.push_str(&convert_expr(test, indent));
                result.push_str(")))");
            }
            result
        }
        Progn(exprs) => {
            match exprs.len() {
                0 => "()".to_string(),
                1 => convert_expr(&exprs[0], indent),
                _ => {
                    let mut result = String::new();
                    result.push_str("(progn");
                    for e in exprs {
                        result.push_str("\n");
                        result.push_str(&make_indent(indent+2));
                        result.push_str(&convert_expr(e, indent+2));
                    }
                    result.push_str(")");
                    result
                }
            }
        }
        Prog1(value, exprs) => {
            match exprs.len() {
                0 => convert_expr(value, indent),
                _ => {
                    let mut result = String::new();
                    result.push_str("(prog1 ");
                    result.push_str(&convert_expr(value, indent+2));
                    for e in exprs {
                        result.push_str("\n");
                        result.push_str(&make_indent(indent+2));
                        result.push_str(&convert_expr(e, indent+2));
                    }
                    result.push_str(")");
                    result
                }
            }
        }
        Block(name, e) => {
            let mut result = String::new();
            result.push_str("(block ");
            result.push_str(&name);
            result.push_str("\n");
            result.push_str(&make_indent(indent+2));
            result.push_str(&convert_expr(e, indent+2));
            result.push_str(")");
            result
        }
        ReturnFrom(name, value) => {
            let mut result = String::new();
            result.push_str("(return-from ");
            result.push_str(&name);
            result.push_str(" ");
            result.push_str(&convert_expr(value, indent+2));
            result.push_str(")");
            result
        }
        Tagbody(name, e) => {
            let mut result = String::new();
            result.push_str("(tagbody ");
            result.push_str(&name);
            result.push_str("\n");
            result.push_str(&make_indent(indent+2));
            result.push_str(&convert_expr(e, indent+2));
            result.push_str(")");
            result
        }
        Go(name) => {
            let mut result = String::new();
            result.push_str("(go ");
            result.push_str(&name);
            result.push_str(")");
            result
        }
        Switch(index, default_target, targets) => {
            let mut result = String::new();
            result.push_str("(case ");
            result.push_str(&convert_expr(index, indent+2));
            result.push_str("\n");
            for (i, e) in targets.iter().enumerate() {
                result.push_str(&make_indent(indent+2));
                result.push_str(&format!("({i} "));
                result.push_str(&convert_expr(e, indent+2));
                result.push_str(&format!(")\n"));
            }
            result.push_str(&make_indent(indent+2));
            result.push_str(&format!("(otherwise "));
            result.push_str(&convert_expr(default_target, indent+2));
            result.push_str("))");
            result
        }
        I32Load(info, addr, addend) => {
            let mut result = String::new();
            result.push_str("(i32load");
            if let Some((width, signed)) = info {
                result.push_str(&format!("{}{}", width, if *signed { 's' } else { 'u' }));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(")");
            result
        }
        I32Store(info, addr, value, addend) => {
            let mut result = String::new();
            result.push_str(&format!("(i32store"));
            if let Some(width) = info {
                result.push_str(&format!("{width}"));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(" ");
            result.push_str(&convert_expr(value, indent+4));
            result.push_str(")");
            result
        }
        I64Load(info, addr, addend) => {
            let mut result = String::new();
            result.push_str("(i64load");
            if let Some((width, signed)) = info {
                result.push_str(&format!("{}{}", width, if *signed { 's' } else { 'u' }));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(")");
            result
        }
        I64Store(info, addr, value, addend) => {
            let mut result = String::new();
            result.push_str(&format!("(i64store"));
            if let Some(width) = info {
                result.push_str(&format!("{width}"));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(" ");
            result.push_str(&convert_expr(value, indent+4));
            result.push_str(")");
            result
        }
        F32Load(addr, addend) => {
            let mut result = String::new();
            result.push_str("(f32load context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(")");
            result
        }
        F32Store(addr, value, addend) => {
            let mut result = String::new();
            result.push_str(&format!("(f32store context "));
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(" ");
            result.push_str(&convert_expr(value, indent+4));
            result.push_str(")");
            result
        }
        F64Load(addr, addend) => {
            let mut result = String::new();
            result.push_str("(f64load context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(")");
            result
        }
        F64Store(addr, value, addend) => {
            let mut result = String::new();
            result.push_str(&format!("(f64store context "));
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent+4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent+4));
                result.push_str(&format!(" {addend})"));
            }
            result.push_str(" ");
            result.push_str(&convert_expr(value, indent+4));
            result.push_str(")");
            result
        }
    }
}

fn convert_body_exprs(exprs: &[Expr], indent: usize) -> String {
    let mut result = String::new();

    for (i, e) in exprs.iter().enumerate() {
        if i != 0 {
            result.push_str("\n");
        }
        result.push_str(&make_indent(indent));
        result.push_str(&convert_expr(&e, indent));
    }

    result
}

fn convert_function(module: &Module, func: &Function) -> Result<String> {
    let Some(body) = func.body.as_ref() else {
        let (module, name) = func.name.as_ref().unwrap();
        return Ok(format!("(define-wasm-import {} ({}) ({}) {} {})",
                          func.name(),
                          {
                              let mut result = String::new();
                              for ty in func.ty.params.iter() {
                                  if !result.is_empty() {
                                      result.push_str(" ");
                                  }
                                  result.push_str(&convert_type(*ty));
                              }
                              result
                          },
                          {
                              let mut result = String::new();
                              for ty in func.ty.results.iter() {
                                  if !result.is_empty() {
                                      result.push_str(" ");
                                  }
                                  result.push_str(&convert_type(*ty));
                              }
                              result
                          },
                          symbolicate(&module), symbolicate(&name)));
    };

    let mut output = String::new();
    let mut all_locals = Vec::new();

    for (i, ty) in func.ty.params.iter().enumerate() {
        all_locals.push((format!("param-{i}"), *ty));
    }
    for (i, ty) in body.locals.iter().enumerate() {
        all_locals.push((format!("local-{}", i+func.ty.params.len()), *ty));
    }

    output.push_str(&format!("(define-wasm-function {} ({}) ({})\n",
                             func.name(),
                             convert_parameters(&func.ty.params),
                             {
                                 let mut result = String::new();
                                 for ty in func.ty.results.iter() {
                                     if !result.is_empty() {
                                         result.push_str(" ");
                                     }
                                     result.push_str(&convert_type(*ty));
                                 }
                                 result
                             }));
    // Local variable bindings.
    // TODO: Type declarations.
    output.push_str("  (let (");
    for (i, (name, ty)) in all_locals.iter().skip(func.ty.params.len()).enumerate() {
        if i != 0 {
            output.push_str("\n        ");
        }
        output.push_str(&format!("({name} {})", initializer_for_type(*ty)));
    }
    output.push_str(")\n");

    println!("func {} {} {:?}", func.index, func.name(), func.ty);

    if func.index == 1835 {
        let op_read = wasmparser::OperatorsReader::new(
            wasmparser::BinaryReader::new(&body.code_bytes, body.code_offset));
        for (i, op) in op_read.into_iter().enumerate() {
            let op = op?;
            println!("  {i}: {op:?}");
        }
    }

    let mut op_read = wasmparser::OperatorsReader::new(
        wasmparser::BinaryReader::new(&body.code_bytes, body.code_offset));

    let exprs = expressionify_function_body(module, func, &all_locals, &mut op_read)?;

    output.push_str(&convert_body_exprs(&exprs, 4));

    output.push_str("))");
    Ok(output)
}

fn emit(module: &Module, package: &str) -> Result<String> {
    let mut result = String::new();

    writeln!(&mut result, "(in-package {package})")?;
    writeln!(&mut result)?;

    writeln!(&mut result, "(defun wasm2cl-create-context ()")?;
    writeln!(&mut result, "  (let ((memory (make-array {} :element-type '(unsigned-byte 8)))",
             module.memory_initial_size)?;
    writeln!(&mut result, "        (table (make-array {} :initial-element nil))",
             module.table_initial_size)?;
    write!(&mut result, "        (globals (vector")?;
    for global in module.globals.iter() {
        write!(&mut result, " {}", global.initializer)?;
    }
    writeln!(&mut result, ")))")?;
    for data in module.active_data.iter() {
        writeln!(&mut result, "    (replace memory #.(coerce '(")?;
        for (i, byte) in data.data.iter().enumerate() {
            if i != 0 && (i % 25) == 0 {
                writeln!(&mut result)?;
            }
            write!(&mut result, " {byte}")?;
        }
        writeln!(&mut result, ")")?;
        writeln!(&mut result, "                              '(simple-array (unsigned-byte 8) (*)))")?;
        writeln!(&mut result, "             :start1 {})", data.address)?;
    }
    for elt in module.active_elements.iter() {
        for (i, val) in elt.data.iter().enumerate() {
            writeln!(&mut result, "    (setf (svref table {}) #'{})",
                     elt.address + i,
                     module.functions[*val].name())?;
        }
    }
    writeln!(&mut result, "  (make-wasm-context :memory memory")?;
    writeln!(&mut result, "                     :globals globals")?;
    writeln!(&mut result, "                     :table table)))")?;
    writeln!(&mut result)?;

    for f in module.functions.iter() {
        writeln!(&mut result, "{}", convert_function(&module, f)?)?;
        writeln!(&mut result)?;
    }

    for e in module.exports.iter() {
        writeln!(&mut result, "(define-wasm-export {} {})",
                 symbolicate(&e.name),
                 module.functions[e.func_idx].name())?;
    }

    Ok(result)
}

#[derive(ClapParser)]
struct Cli {
    input: PathBuf,
    #[arg(short, long, default_value = "out.lisp")]
    output: PathBuf,
    package: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let bytes = fs::read(&cli.input)?;
    let module = parse(&bytes)?;

    println!("types:        {}", module.types.len());
    println!("functions:    {}", module.functions.len());
    println!("globals:      {}", module.globals.len());
    println!("exports:      {}", module.exports.len());
    println!("active datas: {}", module.active_data.len());
    println!("active elts:  {}", module.active_elements.len());

    fs::write(&cli.output, emit(&module, &cli.package)?)?;

    Ok(())
}
