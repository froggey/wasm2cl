//! Convert a function body's wasm operator stream into an `Expr` tree.

use anyhow::{Result, bail};

use crate::expr::{CatchHandler, Expr, Primitive};
use crate::module::{Function, Module, Type};

#[derive(Debug)]
enum BlockKind {
    Block,
    If,
    Loop,
    Try,
    TryTable,
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
    handlers: Vec<CatchHandler>,
    try_body: Option<Vec<Expr>>,
    current_handler: Option<(Option<u32>, Vec<String>)>,
}

// This tries very hard to reconstruct expression trees from the bytecode.
// We could just turn the stack into a sequence of assignments to stack slots,
// but CL compilers don't really like that.
struct Ctx<'m, 'o, 'r> {
    module: &'m Module,
    func: &'m Function,
    all_locals: &'m [(String, Type)],
    ops: &'r mut wasmparser::OperatorsReader<'o>,
    unreachable: bool,
    unreachable_depth: usize,
    pc: u32,
    stack: Vec<Expr>,
    exprs: Vec<Expr>,
    block_stack: Vec<ActiveBlock>,
}

impl<'m, 'o, 'r> Ctx<'m, 'o, 'r> {
    fn new(
        module: &'m Module,
        func: &'m Function,
        all_locals: &'m [(String, Type)],
        ops: &'r mut wasmparser::OperatorsReader<'o>,
    ) -> Self {
        Ctx {
            module,
            func,
            all_locals,
            ops,
            unreachable: false,
            unreachable_depth: 0,
            pc: 0,
            stack: Vec::new(),
            exprs: Vec::new(),
            block_stack: vec![],
        }
    }

    fn single_expr(mut exprs: Vec<Expr>) -> Expr {
        if exprs.len() == 1 {
            exprs.pop().unwrap()
        } else {
            Expr::Progn(exprs)
        }
    }

    fn block_result_count(&self, blockty: &wasmparser::BlockType) -> usize {
        match blockty {
            wasmparser::BlockType::Empty => 0,
            wasmparser::BlockType::Type(_) => 1,
            wasmparser::BlockType::FuncType(idx) => self.module.types[*idx as usize].results.len(),
        }
    }

    fn append_side_effect(&mut self, new: Expr) {
        if self.stack.is_empty() {
            self.exprs.push(new);
        } else {
            let last = self.stack.len() - 1;
            if let Expr::Prog1(_, exprs) = &mut self.stack[last] {
                exprs.push(new);
            } else {
                let value = self.pop1();
                self.stack.push(Expr::Prog1(Box::new(value), vec![new]));
            }
        }
    }

    fn prim_op1(&mut self, prim: Primitive) {
        let val = self.pop1();
        self.stack.push(Expr::Prim(prim, vec![val]));
    }

    fn prim_op2(&mut self, prim: Primitive) {
        let rhs = self.pop1();
        let lhs = self.pop1();
        self.stack.push(Expr::Prim(prim, vec![lhs, rhs]));
    }

    fn pop1(&mut self) -> Expr {
        self.stack.pop().unwrap()
    }

    fn pop_n(&mut self, n: usize) -> Vec<Expr> {
        self.stack.split_off(self.stack.len() - n)
    }

    fn resolve_depth(&self, depth: u32) -> usize {
        self.block_stack.len() - 1 - (depth as usize)
    }

    fn branch_expr(&self, idx: usize) -> Expr {
        if matches!(self.block_stack[idx].kind, BlockKind::Loop) {
            Expr::Go(self.block_stack[idx].name.clone())
        } else {
            Expr::ReturnFrom(
                self.block_stack[idx].name.clone(),
                Box::new(Expr::Progn(vec![])),
            )
        }
    }

    fn emit_result(&mut self, op: &str, expr: Expr, n_results: usize) -> Result<()> {
        match n_results {
            // call for effect
            0 => self.append_side_effect(expr),
            // call for single value
            1 => self.stack.push(expr),
            n => bail!("{op} producing multiple values ({n})"),
        }
        Ok(())
    }

    fn run(&mut self) -> Result<Vec<Expr>> {
        use wasmparser::Operator::*;

        loop {
            let op = self.ops.read()?;
            match op {
                If { .. } if self.unreachable => {
                    self.unreachable_depth += 1;
                }
                Block { .. } if self.unreachable => {
                    self.unreachable_depth += 1;
                }
                Loop { .. } if self.unreachable => {
                    self.unreachable_depth += 1;
                }
                Try { .. } if self.unreachable => {
                    self.unreachable_depth += 1;
                }
                TryTable { .. } if self.unreachable => {
                    self.unreachable_depth += 1;
                }
                Else if self.unreachable && self.unreachable_depth != 0 => {}
                Catch { .. } if self.unreachable && self.unreachable_depth != 0 => {}
                CatchAll if self.unreachable && self.unreachable_depth != 0 => {}
                End if self.unreachable && self.unreachable_depth != 0 => {
                    self.unreachable_depth -= 1;
                }
                Delegate { .. } if self.unreachable && self.unreachable_depth != 0 => {
                    self.unreachable_depth -= 1;
                }

                Block { blockty } => self.block_op(blockty),
                Loop { blockty } => self.loop_op(blockty),
                If { blockty } => self.if_op(blockty),
                Try { blockty } => self.try_op(blockty),
                TryTable { try_table } => self.try_table_op(try_table)?,
                Else => self.else_op()?,
                End => {
                    let Some(entry) = self.block_stack.pop() else {
                        break;
                    };
                    self.end_op(entry)?;
                }
                Catch { tag_index } => self.catch_op(tag_index)?,
                CatchAll => self.catch_all_op()?,
                Delegate { .. } => self.delegate_op()?,

                _ if self.unreachable => {} // Nothing

                // Normal execution.
                Throw { tag_index } => self.throw_op(tag_index),
                Rethrow { relative_depth } => self.rethrow_op(relative_depth)?,
                ThrowRef => self.throw_ref_op(),
                I32Const { value } => self.i32_const_op(value),
                I64Const { value } => self.i64_const_op(value),
                F32Const { value } => self.f32_const_op(value),
                F64Const { value } => self.f64_const_op(value),
                GlobalGet { global_index } => self.global_get_op(global_index),
                GlobalSet { global_index } => self.global_set_op(global_index),
                LocalGet { local_index } => self.local_get_op(local_index),
                LocalSet { local_index } => self.local_set_op(local_index),
                LocalTee { local_index } => self.local_tee_op(local_index),
                Drop => self.drop_op(),
                Unreachable => self.unreachable_op(),
                Return => self.return_op(),
                Call { function_index } => self.call_op(function_index)?,
                CallIndirect {
                    type_index,
                    table_index,
                } => self.call_indirect_op(type_index, table_index)?,
                Br { relative_depth } => self.br_op(relative_depth)?,
                BrIf { relative_depth } => self.br_if_op(relative_depth),
                BrTable { targets } => self.br_table_op(targets)?,
                Select => self.select_op(),
                MemoryCopy { dst_mem, src_mem } => self.memory_copy_op(dst_mem, src_mem)?,
                MemoryFill { mem } => self.memory_fill_op(mem)?,
                MemorySize { mem } => self.memory_size_op(mem)?,
                MemoryGrow { mem } => self.memory_grow_op(mem)?,
                /* I32 */
                I32Load { memarg } => self.i32_load_op(None, memarg),
                I32Load8U { memarg } => self.i32_load_op(Some((8, false)), memarg),
                I32Load8S { memarg } => self.i32_load_op(Some((8, true)), memarg),
                I32Load16U { memarg } => self.i32_load_op(Some((16, false)), memarg),
                I32Load16S { memarg } => self.i32_load_op(Some((16, true)), memarg),
                I32Store { memarg } => self.i32_store_op(None, memarg),
                I32Store8 { memarg } => self.i32_store_op(Some(8), memarg),
                I32Store16 { memarg } => self.i32_store_op(Some(16), memarg),
                I32Eqz => self.prim_op1(Primitive::I32Eqz),
                I32Eq => self.prim_op2(Primitive::I32Eq),
                I32Ne => self.prim_op2(Primitive::I32Ne),
                I32LeU => self.prim_op2(Primitive::I32LeU),
                I32LtU => self.prim_op2(Primitive::I32LtU),
                I32GeU => self.prim_op2(Primitive::I32GeU),
                I32GtU => self.prim_op2(Primitive::I32GtU),
                I32LeS => self.prim_op2(Primitive::I32LeS),
                I32LtS => self.prim_op2(Primitive::I32LtS),
                I32GeS => self.prim_op2(Primitive::I32GeS),
                I32GtS => self.prim_op2(Primitive::I32GtS),
                I32Add => self.prim_op2(Primitive::I32Add),
                I32Sub => self.prim_op2(Primitive::I32Sub),
                I32Mul => self.prim_op2(Primitive::I32Mul),
                I32DivU => self.prim_op2(Primitive::I32DivU),
                I32DivS => self.prim_op2(Primitive::I32DivS),
                I32RemU => self.prim_op2(Primitive::I32RemU),
                I32RemS => self.prim_op2(Primitive::I32RemS),
                I32And => self.prim_op2(Primitive::I32And),
                I32Or => self.prim_op2(Primitive::I32Or),
                I32Xor => self.prim_op2(Primitive::I32Xor),
                I32Shl => self.prim_op2(Primitive::I32Shl),
                I32ShrU => self.prim_op2(Primitive::I32ShrU),
                I32ShrS => self.prim_op2(Primitive::I32ShrS),
                I32Rotl => self.prim_op2(Primitive::I32Rotl),
                I32Rotr => self.prim_op2(Primitive::I32Rotr),
                I32Clz => self.prim_op1(Primitive::I32Clz),
                I32Ctz => self.prim_op1(Primitive::I32Ctz),
                I32Popcnt => self.prim_op1(Primitive::I32Popcnt),
                I32WrapI64 => self.prim_op1(Primitive::I32WrapI64),
                I32Extend8S => self.prim_op1(Primitive::I32Extend8S),
                I32Extend16S => self.prim_op1(Primitive::I32Extend16S),
                I32TruncF32U => self.prim_op1(Primitive::I32TruncF32U),
                I32TruncF32S => self.prim_op1(Primitive::I32TruncF32S),
                I32TruncF64U => self.prim_op1(Primitive::I32TruncF64U),
                I32TruncF64S => self.prim_op1(Primitive::I32TruncF64S),
                I32TruncSatF32U => self.prim_op1(Primitive::I32TruncSatF32U),
                I32TruncSatF32S => self.prim_op1(Primitive::I32TruncSatF32S),
                I32TruncSatF64U => self.prim_op1(Primitive::I32TruncSatF64U),
                I32TruncSatF64S => self.prim_op1(Primitive::I32TruncSatF64S),
                I32ReinterpretF32 => self.prim_op1(Primitive::I32ReinterpretF32),
                /* I64 */
                I64Load { memarg } => self.i64_load_op(None, memarg),
                I64Load8U { memarg } => self.i64_load_op(Some((8, false)), memarg),
                I64Load8S { memarg } => self.i64_load_op(Some((8, true)), memarg),
                I64Load16U { memarg } => self.i64_load_op(Some((16, false)), memarg),
                I64Load16S { memarg } => self.i64_load_op(Some((16, true)), memarg),
                I64Load32U { memarg } => self.i64_load_op(Some((32, false)), memarg),
                I64Load32S { memarg } => self.i64_load_op(Some((32, true)), memarg),
                I64Store { memarg } => self.i64_store_op(None, memarg),
                I64Store8 { memarg } => self.i64_store_op(Some(8), memarg),
                I64Store16 { memarg } => self.i64_store_op(Some(16), memarg),
                I64Store32 { memarg } => self.i64_store_op(Some(32), memarg),
                I64Eqz => self.prim_op1(Primitive::I64Eqz),
                I64Eq => self.prim_op2(Primitive::I64Eq),
                I64Ne => self.prim_op2(Primitive::I64Ne),
                I64LeU => self.prim_op2(Primitive::I64LeU),
                I64LtU => self.prim_op2(Primitive::I64LtU),
                I64GeU => self.prim_op2(Primitive::I64GeU),
                I64GtU => self.prim_op2(Primitive::I64GtU),
                I64LeS => self.prim_op2(Primitive::I64LeS),
                I64LtS => self.prim_op2(Primitive::I64LtS),
                I64GeS => self.prim_op2(Primitive::I64GeS),
                I64GtS => self.prim_op2(Primitive::I64GtS),
                I64Add => self.prim_op2(Primitive::I64Add),
                I64Sub => self.prim_op2(Primitive::I64Sub),
                I64Mul => self.prim_op2(Primitive::I64Mul),
                I64DivU => self.prim_op2(Primitive::I64DivU),
                I64DivS => self.prim_op2(Primitive::I64DivS),
                I64RemU => self.prim_op2(Primitive::I64RemU),
                I64RemS => self.prim_op2(Primitive::I64RemS),
                I64And => self.prim_op2(Primitive::I64And),
                I64Or => self.prim_op2(Primitive::I64Or),
                I64Xor => self.prim_op2(Primitive::I64Xor),
                I64Shl => self.prim_op2(Primitive::I64Shl),
                I64ShrU => self.prim_op2(Primitive::I64ShrU),
                I64ShrS => self.prim_op2(Primitive::I64ShrS),
                I64Rotl => self.prim_op2(Primitive::I64Rotl),
                I64Rotr => self.prim_op2(Primitive::I64Rotr),
                I64Clz => self.prim_op1(Primitive::I64Clz),
                I64Ctz => self.prim_op1(Primitive::I64Ctz),
                I64Popcnt => self.prim_op1(Primitive::I64Popcnt),
                I64Extend8S => self.prim_op1(Primitive::I64Extend8S),
                I64Extend16S => self.prim_op1(Primitive::I64Extend16S),
                I64Extend32S => self.prim_op1(Primitive::I64Extend32S),
                I64ExtendI32U => self.prim_op1(Primitive::I64ExtendI32U),
                I64ExtendI32S => self.prim_op1(Primitive::I64ExtendI32S),
                I64TruncF32U => self.prim_op1(Primitive::I64TruncF32U),
                I64TruncF32S => self.prim_op1(Primitive::I64TruncF32S),
                I64TruncF64U => self.prim_op1(Primitive::I64TruncF64U),
                I64TruncF64S => self.prim_op1(Primitive::I64TruncF64S),
                I64TruncSatF32U => self.prim_op1(Primitive::I64TruncSatF32U),
                I64TruncSatF32S => self.prim_op1(Primitive::I64TruncSatF32S),
                I64TruncSatF64U => self.prim_op1(Primitive::I64TruncSatF64U),
                I64TruncSatF64S => self.prim_op1(Primitive::I64TruncSatF64S),
                I64ReinterpretF64 => self.prim_op1(Primitive::I64ReinterpretF64),
                /* F32 */
                F32Load { memarg } => self.f32_load_op(memarg),
                F32Store { memarg } => self.f32_store_op(memarg),
                F32Eq => self.prim_op2(Primitive::F32Eq),
                F32Ne => self.prim_op2(Primitive::F32Ne),
                F32Le => self.prim_op2(Primitive::F32Le),
                F32Lt => self.prim_op2(Primitive::F32Lt),
                F32Ge => self.prim_op2(Primitive::F32Ge),
                F32Gt => self.prim_op2(Primitive::F32Gt),
                F32Add => self.prim_op2(Primitive::F32Add),
                F32Sub => self.prim_op2(Primitive::F32Sub),
                F32Mul => self.prim_op2(Primitive::F32Mul),
                F32Div => self.prim_op2(Primitive::F32Div),
                F32Abs => self.prim_op1(Primitive::F32Abs),
                F32Neg => self.prim_op1(Primitive::F32Neg),
                F32Sqrt => self.prim_op1(Primitive::F32Sqrt),
                F32Floor => self.prim_op1(Primitive::F32Floor),
                F32Ceil => self.prim_op1(Primitive::F32Ceil),
                F32Trunc => self.prim_op1(Primitive::F32Trunc),
                F32Nearest => self.prim_op1(Primitive::F32Nearest),
                F32Min => self.prim_op2(Primitive::F32Min),
                F32Max => self.prim_op2(Primitive::F32Max),
                F32Copysign => self.prim_op2(Primitive::F32Copysign),
                F32ConvertI32U => self.prim_op1(Primitive::F32ConvertI32U),
                F32ConvertI32S => self.prim_op1(Primitive::F32ConvertI32S),
                F32ConvertI64U => self.prim_op1(Primitive::F32ConvertI64U),
                F32ConvertI64S => self.prim_op1(Primitive::F32ConvertI64S),
                F32ReinterpretI32 => self.prim_op1(Primitive::F32ReinterpretI32),
                F32DemoteF64 => self.prim_op1(Primitive::F32DemoteF64),
                /* F64 */
                F64Load { memarg } => self.f64_load_op(memarg),
                F64Store { memarg } => self.f64_store_op(memarg),
                F64Eq => self.prim_op2(Primitive::F64Eq),
                F64Ne => self.prim_op2(Primitive::F64Ne),
                F64Le => self.prim_op2(Primitive::F64Le),
                F64Lt => self.prim_op2(Primitive::F64Lt),
                F64Ge => self.prim_op2(Primitive::F64Ge),
                F64Gt => self.prim_op2(Primitive::F64Gt),
                F64Add => self.prim_op2(Primitive::F64Add),
                F64Sub => self.prim_op2(Primitive::F64Sub),
                F64Mul => self.prim_op2(Primitive::F64Mul),
                F64Div => self.prim_op2(Primitive::F64Div),
                F64Abs => self.prim_op1(Primitive::F64Abs),
                F64Neg => self.prim_op1(Primitive::F64Neg),
                F64Sqrt => self.prim_op1(Primitive::F64Sqrt),
                F64Floor => self.prim_op1(Primitive::F64Floor),
                F64Ceil => self.prim_op1(Primitive::F64Ceil),
                F64Trunc => self.prim_op1(Primitive::F64Trunc),
                F64Nearest => self.prim_op1(Primitive::F64Nearest),
                F64Min => self.prim_op2(Primitive::F64Min),
                F64Max => self.prim_op2(Primitive::F64Max),
                F64Copysign => self.prim_op2(Primitive::F64Copysign),
                F64ConvertI32U => self.prim_op1(Primitive::F64ConvertI32U),
                F64ConvertI32S => self.prim_op1(Primitive::F64ConvertI32S),
                F64ConvertI64U => self.prim_op1(Primitive::F64ConvertI64U),
                F64ConvertI64S => self.prim_op1(Primitive::F64ConvertI64S),
                F64ReinterpretI64 => self.prim_op1(Primitive::F64ReinterpretI64),
                F64PromoteF32 => self.prim_op1(Primitive::F64PromoteF32),
                op => bail!("unsupported operator: {op:?}"),
            }
            self.pc += 1;
        }

        if !self.unreachable {
            if !self.func.ty.results.is_empty() {
                let value = self.pop1();
                self.exprs.push(value);
            }

            if !self.stack.is_empty() {
                bail!("stack not empty at end of function body");
            }
        }

        Ok(std::mem::take(&mut self.exprs))
    }

    fn push_block(&mut self, kind: BlockKind, prefix: &str, blockty: wasmparser::BlockType) {
        self.block_stack.push(ActiveBlock {
            kind,
            blockty,
            old_exprs: std::mem::take(&mut self.exprs),
            then: None,
            name: format!("{prefix}-{}", self.pc),
            targeted: false,
            stack: std::mem::take(&mut self.stack),
            handlers: vec![],
            try_body: None,
            current_handler: None,
        });
    }

    fn block_op(&mut self, blockty: wasmparser::BlockType) {
        self.push_block(BlockKind::Block, "block", blockty);
    }

    fn loop_op(&mut self, blockty: wasmparser::BlockType) {
        self.push_block(BlockKind::Loop, "loop", blockty);
    }

    fn if_op(&mut self, blockty: wasmparser::BlockType) {
        self.push_block(BlockKind::If, "if", blockty);
    }

    fn try_op(&mut self, blockty: wasmparser::BlockType) {
        self.push_block(BlockKind::Try, "try", blockty);
    }

    fn try_table_op(&mut self, try_table: wasmparser::TryTable) -> Result<()> {
        // Each catch clause branches to an outer label (resolved before
        // this try_table's own frame is pushed), delivering the payload
        // as that label's value. The payload becomes the catch clause's
        // body, so `wasm-try` evaluates to it on the catch path.
        let name = format!("try-{}", self.pc);
        let mut handlers = Vec::new();
        for catch in try_table.catches {
            let handler = match catch {
                wasmparser::Catch::One { tag, label } => {
                    let target_idx = self.resolve_depth(label);
                    let ty = &self.module.tags[tag as usize];
                    if ty.params.len() > 1 {
                        bail!("try_table catch payload with multiple values unsupported");
                    }
                    let vars: Vec<String> = (0..ty.params.len())
                        .map(|i| format!("{name}-exn-{i}"))
                        .collect();
                    let body = if matches!(self.block_stack[target_idx].kind, BlockKind::Loop) {
                        if !ty.params.is_empty() {
                            bail!("try_table catch delivering a value into a loop unsupported");
                        }
                        Expr::Go(self.block_stack[target_idx].name.clone())
                    } else if ty.params.is_empty() {
                        Expr::Progn(vec![])
                    } else {
                        Expr::Local(vars[0].clone())
                    };
                    self.block_stack[target_idx].targeted = true;
                    CatchHandler {
                        tag: Some(tag),
                        vars,
                        exnref_var: None,
                        body: Box::new(body),
                    }
                }
                wasmparser::Catch::All { label } => {
                    let target_idx = self.resolve_depth(label);
                    let body = if matches!(self.block_stack[target_idx].kind, BlockKind::Loop) {
                        Expr::Go(self.block_stack[target_idx].name.clone())
                    } else {
                        Expr::Progn(vec![])
                    };
                    self.block_stack[target_idx].targeted = true;
                    CatchHandler {
                        tag: None,
                        vars: vec![],
                        exnref_var: None,
                        body: Box::new(body),
                    }
                }
                wasmparser::Catch::OneRef { tag, label } => {
                    let target_idx = self.resolve_depth(label);
                    let ty = &self.module.tags[tag as usize];
                    if ty.params.len() > 1 {
                        bail!("try_table catch_ref payload with multiple values unsupported");
                    }
                    if matches!(self.block_stack[target_idx].kind, BlockKind::Loop) {
                        bail!("try_table catch_ref delivering a value into a loop unsupported");
                    }
                    let vars: Vec<String> = (0..ty.params.len())
                        .map(|i| format!("{name}-exn-{i}"))
                        .collect();
                    let ref_var = format!("{name}-exn-ref");
                    // Per spec, catch_ref pushes the payload values then
                    // the exnref (last) onto the target label, so the
                    // handler body produces (payload..., exnref).
                    let mut body_values: Vec<Expr> =
                        vars.iter().map(|v| Expr::Local(v.clone())).collect();
                    body_values.push(Expr::Local(ref_var.clone()));
                    let body = if body_values.len() == 1 {
                        body_values.pop().unwrap()
                    } else {
                        Expr::Values(body_values)
                    };
                    self.block_stack[target_idx].targeted = true;
                    CatchHandler {
                        tag: Some(tag),
                        vars,
                        exnref_var: Some(ref_var),
                        body: Box::new(body),
                    }
                }
                wasmparser::Catch::AllRef { label } => {
                    let target_idx = self.resolve_depth(label);
                    if matches!(self.block_stack[target_idx].kind, BlockKind::Loop) {
                        bail!("try_table catch_all_ref delivering a value into a loop unsupported");
                    }
                    let ref_var = format!("{name}-exn-ref");
                    self.block_stack[target_idx].targeted = true;
                    CatchHandler {
                        tag: None,
                        vars: vec![],
                        exnref_var: Some(ref_var.clone()),
                        body: Box::new(Expr::Local(ref_var)),
                    }
                }
            };
            handlers.push(handler);
        }
        self.block_stack.push(ActiveBlock {
            kind: BlockKind::TryTable,
            blockty: try_table.ty,
            old_exprs: std::mem::take(&mut self.exprs),
            then: None,
            name,
            targeted: false,
            stack: std::mem::take(&mut self.stack),
            handlers,
            try_body: None,
            current_handler: None,
        });
        Ok(())
    }

    fn else_op(&mut self) -> Result<()> {
        let was_unreachable = self.unreachable;
        self.unreachable = false;
        let idx = self.block_stack.len() - 1;
        if !matches!(self.block_stack[idx].kind, BlockKind::If) {
            bail!("`else` outside of an `if` block");
        }
        if self.block_stack[idx].then.is_some() {
            bail!("`else` after a previous `else`");
        }
        if !was_unreachable
            && !matches!(self.block_stack[idx].blockty, wasmparser::BlockType::Empty)
        {
            let value = self.pop1();
            self.exprs.push(value);
        }
        if !was_unreachable && !self.stack.is_empty() {
            bail!("stack not empty at `else`");
        }
        self.block_stack[idx].then = Some(std::mem::take(&mut self.exprs));
        Ok(())
    }

    fn end_op(&mut self, entry: ActiveBlock) -> Result<()> {
        let was_unreachable = self.unreachable;

        // A non-loop block targeted by Br means the End is reachable (forward jump).
        // A loop targeted by Br means the back-edge exists, but the fall-through End
        // is still unreachable.
        // A try that has a handler is reachable at its End even if the current handler
        // ended in a br/throw: the try body's normal-completion path reaches the End.
        let becomes_reachable = match entry.kind {
            BlockKind::Loop => false,
            BlockKind::Try => was_unreachable && (entry.targeted || entry.try_body.is_some()),
            BlockKind::TryTable => was_unreachable && entry.targeted,
            _ => was_unreachable && entry.targeted,
        };

        self.unreachable = was_unreachable && !becomes_reachable;

        // End of the current block.
        // Only pop the block's result from the stack on reachable fall-through.
        if !was_unreachable && !matches!(entry.blockty, wasmparser::BlockType::Empty) {
            let value = self.pop1();
            self.exprs.push(value);
        }
        if !was_unreachable && !self.stack.is_empty() {
            bail!("stack not empty at block `end`");
        }
        self.stack = entry.stack;
        let mut final_expr = match entry.kind {
            BlockKind::If => {
                let (mut then, mut els) = if let Some(then_exprs) = entry.then {
                    (
                        then_exprs,
                        std::mem::replace(&mut self.exprs, entry.old_exprs),
                    )
                } else {
                    (std::mem::replace(&mut self.exprs, entry.old_exprs), vec![])
                };
                let test = self.pop1();
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
                    }),
                )
            }
            BlockKind::Block => Expr::Progn(std::mem::replace(&mut self.exprs, entry.old_exprs)),
            BlockKind::Loop => Expr::Progn(std::mem::replace(&mut self.exprs, entry.old_exprs)),
            BlockKind::Try => {
                let try_body = match entry.try_body {
                    Some(body) => body,
                    None => std::mem::take(&mut self.exprs),
                };
                let mut handlers = entry.handlers;
                if let Some((tag, vars)) = entry.current_handler {
                    handlers.push(CatchHandler {
                        tag,
                        vars,
                        exnref_var: None,
                        body: Box::new(Self::single_expr(std::mem::replace(
                            &mut self.exprs,
                            entry.old_exprs,
                        ))),
                    });
                } else {
                    self.exprs = entry.old_exprs;
                }
                Expr::Try {
                    name: entry.name.clone(),
                    body: Box::new(Self::single_expr(try_body)),
                    handlers,
                }
            }
            BlockKind::TryTable => {
                let try_body = std::mem::take(&mut self.exprs);
                self.exprs = entry.old_exprs;
                Expr::Try {
                    name: entry.name.clone(),
                    body: Box::new(Self::single_expr(try_body)),
                    handlers: entry.handlers,
                }
            }
        };
        if entry.targeted {
            if matches!(entry.kind, BlockKind::Loop) {
                final_expr = Expr::Tagbody(entry.name, Box::new(final_expr));
            } else {
                final_expr = Expr::Block(entry.name, Box::new(final_expr));
            }
        }
        let n_results = self.block_result_count(&entry.blockty);
        if n_results > 1 {
            // Narrow multi-value: the block's results must be consumed by
            // n_results immediate consecutive `local.set` ops (the shape
            // clang emits for catch_ref landing pads). Wasm pops results
            // off the block stack top-first (the exnref that was pushed
            // last), while `(setf (values ...))` assigns first-value-first,
            // so the locals are collected in stream order then reversed.
            if self.unreachable {
                bail!("multi-value block result in unreachable code unsupported");
            }
            let mut locals = Vec::with_capacity(n_results);
            for _ in 0..n_results {
                let op = self.ops.read()?;
                match op {
                    wasmparser::Operator::LocalSet { local_index } => {
                        locals.push(self.all_locals[local_index as usize].0.clone());
                    }
                    op => bail!(
                        "multi-value block results must be consumed by consecutive \
                         local.set, got {op:?}"
                    ),
                }
            }
            locals.reverse();
            self.append_side_effect(Expr::SetfValues {
                locals,
                value: Box::new(final_expr),
            });
        } else if self.unreachable || n_results == 0 {
            self.append_side_effect(final_expr);
        } else {
            self.stack.push(final_expr);
        }
        Ok(())
    }

    fn catch_op(&mut self, tag_index: u32) -> Result<()> {
        let was_unreachable = self.unreachable;
        self.unreachable = false;
        let idx = self.block_stack.len() - 1;
        if !matches!(self.block_stack[idx].kind, BlockKind::Try) {
            bail!("`catch` outside of a `try` block");
        }
        if !was_unreachable
            && !matches!(self.block_stack[idx].blockty, wasmparser::BlockType::Empty)
        {
            let value = self.pop1();
            self.exprs.push(value);
        }
        if !was_unreachable && !self.stack.is_empty() {
            bail!("stack not empty at `catch`");
        }
        let entry = &mut self.block_stack[idx];
        if entry.try_body.is_none() {
            entry.try_body = Some(std::mem::take(&mut self.exprs));
        } else {
            let (tag, vars) = entry.current_handler.take().unwrap();
            entry.handlers.push(CatchHandler {
                tag,
                vars,
                exnref_var: None,
                body: Box::new(Self::single_expr(std::mem::take(&mut self.exprs))),
            });
        }
        self.stack.clear();
        let ty = &self.module.tags[tag_index as usize];
        let mut vars = vec![];
        for i in 0..ty.params.len() {
            let var = format!("{}-exn-{i}", entry.name);
            vars.push(var.clone());
            self.stack.push(Expr::Local(var));
        }
        entry.current_handler = Some((Some(tag_index), vars));
        Ok(())
    }

    fn catch_all_op(&mut self) -> Result<()> {
        let was_unreachable = self.unreachable;
        self.unreachable = false;
        let idx = self.block_stack.len() - 1;
        if !matches!(self.block_stack[idx].kind, BlockKind::Try) {
            bail!("`catch_all` outside of a `try` block");
        }
        if !was_unreachable
            && !matches!(self.block_stack[idx].blockty, wasmparser::BlockType::Empty)
        {
            let value = self.pop1();
            self.exprs.push(value);
        }
        if !was_unreachable && !self.stack.is_empty() {
            bail!("stack not empty at `catch_all`");
        }
        let entry = &mut self.block_stack[idx];
        if entry.try_body.is_none() {
            entry.try_body = Some(std::mem::take(&mut self.exprs));
        } else {
            let (tag, vars) = entry.current_handler.take().unwrap();
            entry.handlers.push(CatchHandler {
                tag,
                vars,
                exnref_var: None,
                body: Box::new(Self::single_expr(std::mem::take(&mut self.exprs))),
            });
        }
        self.stack.clear();
        entry.current_handler = Some((None, vec![]));
        Ok(())
    }

    fn delegate_op(&mut self) -> Result<()> {
        let entry = self.block_stack.pop().unwrap();
        if !matches!(entry.kind, BlockKind::Try) {
            bail!("`delegate` outside of a `try` block");
        }
        if !entry.handlers.is_empty() {
            bail!("`delegate` after a `catch`");
        }
        if entry.try_body.is_some() {
            bail!("`delegate` with a stashed try body");
        }
        if entry.current_handler.is_some() {
            bail!("`delegate` after a handler was started");
        }
        let was_unreachable = self.unreachable;
        if !was_unreachable && !matches!(entry.blockty, wasmparser::BlockType::Empty) {
            let value = self.pop1();
            self.exprs.push(value);
        }
        if !was_unreachable && !self.stack.is_empty() {
            bail!("stack not empty at `delegate`");
        }
        self.stack = entry.stack;
        let final_expr = Expr::Progn(std::mem::replace(&mut self.exprs, entry.old_exprs));
        if self.unreachable || matches!(entry.blockty, wasmparser::BlockType::Empty) {
            self.append_side_effect(final_expr);
        } else {
            self.stack.push(final_expr);
        }
        Ok(())
    }

    fn throw_op(&mut self, tag_index: u32) {
        let ty = &self.module.tags[tag_index as usize];
        let payload = self.pop_n(ty.params.len());
        self.append_side_effect(Expr::Throw(tag_index as usize, payload));
        self.unreachable = true;
    }

    fn rethrow_op(&mut self, relative_depth: u32) -> Result<()> {
        let target = self.resolve_depth(relative_depth);
        if !matches!(self.block_stack[target].kind, BlockKind::Try) {
            bail!("`rethrow` depth does not target a `try` block");
        }
        let exn_var = format!("{}-exn", self.block_stack[target].name);
        self.append_side_effect(Expr::Rethrow(exn_var));
        self.unreachable = true;
        Ok(())
    }

    fn throw_ref_op(&mut self) {
        let exnref = self.pop1();
        self.append_side_effect(Expr::ThrowRef(Box::new(exnref)));
        self.unreachable = true;
    }

    fn i32_const_op(&mut self, value: i32) {
        self.stack.push(Expr::Const(format!("{}", value as u32)));
    }

    fn i64_const_op(&mut self, value: i64) {
        self.stack.push(Expr::Const(format!("{}", value as u64)));
    }

    fn f32_const_op(&mut self, value: wasmparser::Ieee32) {
        self.stack
            .push(Expr::Const(format!("(f32const {})", value.bits())));
    }

    fn f64_const_op(&mut self, value: wasmparser::Ieee64) {
        self.stack
            .push(Expr::Const(format!("(f64const {})", value.bits())));
    }

    fn global_get_op(&mut self, global_index: u32) {
        self.stack.push(Expr::Global(global_index as usize));
    }

    fn global_set_op(&mut self, global_index: u32) {
        let value = self.pop1();
        self.append_side_effect(Expr::GlobalSet(global_index as usize, Box::new(value)));
    }

    fn local_get_op(&mut self, local_index: u32) {
        self.stack
            .push(Expr::Local(self.all_locals[local_index as usize].0.clone()));
    }

    fn local_set_op(&mut self, local_index: u32) {
        let value = self.pop1();
        self.append_side_effect(Expr::Setf(
            self.all_locals[local_index as usize].0.clone(),
            Box::new(value),
        ));
    }

    fn local_tee_op(&mut self, local_index: u32) {
        let value = self.pop1();
        self.stack.push(Expr::Setf(
            self.all_locals[local_index as usize].0.clone(),
            Box::new(value),
        ));
    }

    fn drop_op(&mut self) {
        let value = self.pop1();
        self.append_side_effect(value);
    }

    fn unreachable_op(&mut self) {
        self.append_side_effect(Expr::Prim(Primitive::Unreachable, vec![]));
        self.unreachable = true;
    }

    fn return_op(&mut self) {
        let value = if self.func.ty.results.is_empty() {
            Expr::Progn(vec![])
        } else {
            self.pop1()
        };
        self.append_side_effect(Expr::ReturnFrom("nil".to_string(), Box::new(value)));
        self.unreachable = true;
    }

    fn call_op(&mut self, function_index: u32) -> Result<()> {
        let target = &self.module.functions[function_index as usize];
        let args = self.pop_n(target.ty.params.len());
        self.emit_result(
            "call",
            Expr::Call(target.name(), args),
            target.ty.results.len(),
        )
    }

    fn call_indirect_op(&mut self, type_index: u32, table_index: u32) -> Result<()> {
        if table_index != 0 {
            bail!("call_indirect with non-zero table index ({table_index})");
        }
        let ty = &self.module.types[type_index as usize];
        let idx = self.pop1();
        let args = self.pop_n(ty.params.len());
        self.emit_result(
            "call_indirect",
            Expr::CallIndirect(Box::new(idx), args),
            ty.results.len(),
        )
    }

    fn br_op(&mut self, relative_depth: u32) -> Result<()> {
        let target = self.resolve_depth(relative_depth);
        self.unreachable = true;
        self.block_stack[target].targeted = true;
        if !matches!(self.block_stack[target].kind, BlockKind::Loop)
            && !matches!(
                self.block_stack[target].blockty,
                wasmparser::BlockType::Empty
            )
        {
            bail!(
                "`br` to a value-typed non-loop block unsupported (target {:?})",
                self.block_stack[target].blockty
            );
        }
        self.append_side_effect(self.branch_expr(target));
        Ok(())
    }

    fn br_if_op(&mut self, relative_depth: u32) {
        let target = self.resolve_depth(relative_depth);
        let test = self.pop1();
        self.block_stack[target].targeted = true;
        let value = if !matches!(self.block_stack[target].kind, BlockKind::Loop)
            && !matches!(
                self.block_stack[target].blockty,
                wasmparser::BlockType::Empty
            ) {
            self.pop1()
        } else {
            Expr::Progn(vec![])
        };
        self.append_side_effect(Expr::If(
            Box::new(test),
            Box::new(
                if matches!(self.block_stack[target].kind, BlockKind::Loop) {
                    Expr::Go(self.block_stack[target].name.clone())
                } else {
                    Expr::ReturnFrom(self.block_stack[target].name.clone(), Box::new(value))
                },
            ),
            Box::new(Expr::Progn(vec![])),
        ));
    }

    fn br_table_op(&mut self, targets: wasmparser::BrTable<'o>) -> Result<()> {
        let idx = self.pop1();
        let mut target_code = vec![];
        for target_idx in targets.targets() {
            let target_idx = target_idx?;
            let target = self.resolve_depth(target_idx);
            if !matches!(
                self.block_stack[target].blockty,
                wasmparser::BlockType::Empty
            ) {
                bail!("br_table target with a value-typed block unsupported");
            }
            self.block_stack[target].targeted = true;
            target_code.push(self.branch_expr(target));
        }
        let default_target = self.resolve_depth(targets.default());
        if !matches!(
            self.block_stack[default_target].blockty,
            wasmparser::BlockType::Empty
        ) {
            bail!("br_table default target with a value-typed block unsupported");
        }
        self.block_stack[default_target].targeted = true;
        let default_code = self.branch_expr(default_target);
        self.append_side_effect(Expr::Switch(
            Box::new(idx),
            Box::new(default_code),
            target_code,
        ));
        Ok(())
    }

    fn select_op(&mut self) {
        let cond = self.pop1();
        let rhs = self.pop1();
        let lhs = self.pop1();
        self.stack
            .push(Expr::Select(Box::new(lhs), Box::new(rhs), Box::new(cond)));
    }

    fn memory_copy_op(&mut self, dst_mem: u32, src_mem: u32) -> Result<()> {
        if dst_mem != 0 || src_mem != 0 {
            bail!("memory.copy with non-zero memory index unsupported");
        }
        let n = self.pop1();
        let src = self.pop1();
        let dst = self.pop1();
        self.append_side_effect(Expr::Call("memory-copy".to_string(), vec![dst, src, n]));
        Ok(())
    }

    fn memory_fill_op(&mut self, mem: u32) -> Result<()> {
        if mem != 0 {
            bail!("memory.fill with non-zero memory index unsupported");
        }
        let n = self.pop1();
        let value = self.pop1();
        let dst = self.pop1();
        self.append_side_effect(Expr::Call("memory-fill".to_string(), vec![dst, value, n]));
        Ok(())
    }

    fn memory_size_op(&mut self, mem: u32) -> Result<()> {
        if mem != 0 {
            bail!("memory.size with non-zero memory index unsupported");
        }
        self.stack
            .push(Expr::Call("memory-size".to_string(), vec![]));
        Ok(())
    }

    fn memory_grow_op(&mut self, mem: u32) -> Result<()> {
        if mem != 0 {
            bail!("memory.grow with non-zero memory index unsupported");
        }
        let n = self.pop1();
        self.stack
            .push(Expr::Call("memory-grow".to_string(), vec![n]));
        Ok(())
    }

    fn i32_load_op(&mut self, ext: Option<(usize, bool)>, memarg: wasmparser::MemArg) {
        let addr = self.pop1();
        self.stack
            .push(Expr::I32Load(ext, Box::new(addr), memarg.offset as usize));
    }

    fn i32_store_op(&mut self, width: Option<usize>, memarg: wasmparser::MemArg) {
        let value = self.pop1();
        let addr = self.pop1();
        self.append_side_effect(Expr::I32Store(
            width,
            Box::new(addr),
            Box::new(value),
            memarg.offset as usize,
        ));
    }

    fn i64_load_op(&mut self, ext: Option<(usize, bool)>, memarg: wasmparser::MemArg) {
        let addr = self.pop1();
        self.stack
            .push(Expr::I64Load(ext, Box::new(addr), memarg.offset as usize));
    }

    fn i64_store_op(&mut self, width: Option<usize>, memarg: wasmparser::MemArg) {
        let value = self.pop1();
        let addr = self.pop1();
        self.append_side_effect(Expr::I64Store(
            width,
            Box::new(addr),
            Box::new(value),
            memarg.offset as usize,
        ));
    }

    fn f32_load_op(&mut self, memarg: wasmparser::MemArg) {
        let addr = self.pop1();
        self.stack
            .push(Expr::F32Load(Box::new(addr), memarg.offset as usize));
    }

    fn f32_store_op(&mut self, memarg: wasmparser::MemArg) {
        let value = self.pop1();
        let addr = self.pop1();
        self.append_side_effect(Expr::F32Store(
            Box::new(addr),
            Box::new(value),
            memarg.offset as usize,
        ));
    }

    fn f64_load_op(&mut self, memarg: wasmparser::MemArg) {
        let addr = self.pop1();
        self.stack
            .push(Expr::F64Load(Box::new(addr), memarg.offset as usize));
    }

    fn f64_store_op(&mut self, memarg: wasmparser::MemArg) {
        let value = self.pop1();
        let addr = self.pop1();
        self.append_side_effect(Expr::F64Store(
            Box::new(addr),
            Box::new(value),
            memarg.offset as usize,
        ));
    }
}

pub fn expressionify_function_body(
    module: &Module,
    func: &Function,
    all_locals: &[(String, Type)],
    ops: &mut wasmparser::OperatorsReader,
) -> Result<Vec<Expr>> {
    let mut ctx = Ctx::new(module, func, all_locals, ops);
    ctx.run()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::FuncType;

    fn module_with(types: Vec<FuncType>, tags: Vec<FuncType>, functions: Vec<Function>) -> Module {
        Module {
            memory_initial_size: 0,
            table_initial_size: 0,
            types,
            tags,
            functions,
            exports: vec![],
            active_data: vec![],
            active_elements: vec![],
            globals: vec![],
            start_fn: None,
        }
    }

    fn function(index: usize, ty: FuncType) -> Function {
        Function {
            index,
            ty,
            name: None,
            body: None,
            internal_name: None,
        }
    }

    fn locals(names: &[(&str, Type)]) -> Vec<(String, Type)> {
        names.iter().map(|(n, t)| (n.to_string(), *t)).collect()
    }

    fn run(
        module: &Module,
        func: &Function,
        all_locals: &[(String, Type)],
        code: &[u8],
    ) -> Result<Vec<Expr>> {
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(code, 0));
        expressionify_function_body(module, func, all_locals, &mut ops)
    }

    fn err_message(result: Result<Vec<Expr>>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn single_expr_wraps_sequences() {
        assert!(matches!(
            Ctx::single_expr(vec![Expr::Const("1".into())]),
            Expr::Const(_)
        ));
        assert!(matches!(
            Ctx::single_expr(vec![]),
            Expr::Progn(ref e) if e.is_empty()
        ));
        assert!(matches!(
            Ctx::single_expr(vec![Expr::Const("1".into()), Expr::Const("2".into())]),
            Expr::Progn(ref e) if e.len() == 2
        ));
    }

    #[test]
    fn append_side_effect_pushes_to_empty_exprs() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let mut ctx = Ctx::new(&module, &func, &[], &mut ops);
        ctx.append_side_effect(Expr::Const("1".into()));
        assert_eq!(format!("{:?}", ctx.exprs), "[Const(\"1\")]");
        assert!(ctx.stack.is_empty());
    }

    #[test]
    fn append_side_effect_wraps_value_in_prog1() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let mut ctx = Ctx::new(&module, &func, &[], &mut ops);
        ctx.stack = vec![Expr::Const("1".into())];
        ctx.append_side_effect(Expr::Const("2".into()));
        assert!(ctx.exprs.is_empty());
        assert_eq!(
            format!("{:?}", ctx.stack),
            "[Prog1(Const(\"1\"), [Const(\"2\")])]"
        );
    }

    #[test]
    fn append_side_effect_appends_to_existing_prog1() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let mut ctx = Ctx::new(&module, &func, &[], &mut ops);
        ctx.stack = vec![Expr::Prog1(Box::new(Expr::Const("1".into())), vec![])];
        ctx.append_side_effect(Expr::Const("2".into()));
        assert!(ctx.exprs.is_empty());
        assert_eq!(
            format!("{:?}", ctx.stack),
            "[Prog1(Const(\"1\"), [Const(\"2\")])]"
        );
    }

    #[test]
    fn prim_op1_pops_single_operand() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let mut ctx = Ctx::new(&module, &func, &[], &mut ops);
        ctx.stack = vec![Expr::Const("5".into())];
        ctx.prim_op1(Primitive::I32Eqz);
        assert_eq!(format!("{:?}", ctx.stack), "[Prim(I32Eqz, [Const(\"5\")])]");
    }

    #[test]
    fn prim_op2_keeps_lhs_rhs_order() {
        // lhs pushed first, rhs popped first
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let mut ctx = Ctx::new(&module, &func, &[], &mut ops);
        ctx.stack = vec![Expr::Local("x".into()), Expr::Const("3".into())];
        ctx.prim_op2(Primitive::I32Add);
        assert_eq!(
            format!("{:?}", ctx.stack),
            "[Prim(I32Add, [Local(\"x\"), Const(\"3\")])]"
        );
    }

    #[test]
    fn block_result_count_uses_type_section() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![Type::I32, Type::ExnRef],
            }],
            vec![],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let mut ops = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(&[], 0));
        let ctx = Ctx::new(&module, &func, &[], &mut ops);
        use wasmparser::BlockType;
        assert_eq!(ctx.block_result_count(&BlockType::Empty), 0);
        assert_eq!(
            ctx.block_result_count(&BlockType::Type(wasmparser::ValType::I32)),
            1
        );
        assert_eq!(ctx.block_result_count(&BlockType::FuncType(0)), 2);
    }

    #[test]
    fn expressionify_i32_const() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        let exprs = run(&module, &func, &[], &[0x41, 0x2a, 0x0b]).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Const(\"42\")]");
    }

    #[test]
    fn expressionify_local_get_and_add() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        let l = locals(&[("param-0", Type::I32)]);
        let exprs = run(&module, &func, &l, &[0x20, 0x00, 0x41, 0x02, 0x6a, 0x0b]).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Prim(I32Add, [Local(\"param-0\"), Const(\"2\")])]"
        );
    }

    #[test]
    fn expressionify_block_br() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let exprs = run(&module, &func, &[], &[0x02, 0x40, 0x0c, 0x00, 0x0b, 0x0b]).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Block(\"block-0\", Progn([ReturnFrom(\"block-0\", Progn([]))]))]"
        );
    }

    #[test]
    fn expressionify_if_else() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // i32.const 1; if (i32); i32.const 10; else; i32.const 20; end; end
        let exprs = run(
            &module,
            &func,
            &[],
            &[
                0x41, 0x01, 0x04, 0x7f, 0x41, 0x0a, 0x05, 0x41, 0x14, 0x0b, 0x0b,
            ],
        )
        .unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[If(Const(\"1\"), Const(\"10\"), Const(\"20\"))]"
        );
    }

    #[test]
    fn expressionify_loop_br_if() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // block; loop; i32.const 1; br_if 0; end; end; end
        let exprs = run(
            &module,
            &func,
            &[],
            &[
                0x02, 0x40, 0x03, 0x40, 0x41, 0x01, 0x0d, 0x00, 0x0b, 0x0b, 0x0b,
            ],
        )
        .unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Progn([Tagbody(\"loop-1\", Progn([If(Const(\"1\"), Go(\"loop-1\"), Progn([]))]))])]"
        );
    }

    #[test]
    fn expressionify_call_uses_function_name() {
        let module = module_with(
            vec![],
            vec![],
            vec![
                function(
                    0,
                    FuncType {
                        params: vec![],
                        results: vec![],
                    },
                ),
                function(
                    1,
                    FuncType {
                        params: vec![Type::I32],
                        results: vec![Type::I32],
                    },
                ),
            ],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        let l = locals(&[("param-0", Type::I32)]);
        // local.get 0; call 1; end
        let exprs = run(&module, &func, &l, &[0x20, 0x00, 0x10, 0x01, 0x0b]).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Call(\"wasm-function-1\", [Local(\"param-0\")])]"
        );
    }

    fn landing_pad_code() -> Vec<u8> {
        // block (func 0); try_table catch_ref 0 0; i32.const 42; throw 0; end;
        // unreachable; end; local.set 4; local.set 3; end
        vec![
            0x02, 0x00, 0x1f, 0x40, 0x01, 0x01, 0x00, 0x00, 0x41, 0x2a, 0x08, 0x00, 0x0b, 0x00,
            0x0b, 0x21, 0x04, 0x21, 0x03, 0x0b,
        ]
    }

    #[test]
    fn expressionify_try_table_catch_ref_landing_pad() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![Type::I32, Type::ExnRef],
            }],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let l = locals(&[
            ("local-0", Type::I32),
            ("local-1", Type::I32),
            ("local-2", Type::I32),
            ("local-3", Type::I32),
            ("local-4", Type::ExnRef),
        ]);
        let exprs = run(&module, &func, &l, &landing_pad_code()).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[SetfValues { locals: [\"local-3\", \"local-4\"], value: \
             Block(\"block-0\", Progn([Try { name: \"try-1\", body: Throw(0, [Const(\"42\")]), \
             handlers: [CatchHandler { tag: Some(0), vars: [\"try-1-exn-0\"], \
             exnref_var: Some(\"try-1-exn-ref\"), body: \
             Values([Local(\"try-1-exn-0\"), Local(\"try-1-exn-ref\")]) }] }])) }]"
        );
    }

    #[test]
    fn expressionify_try_table_catch_single_value() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![Type::I32],
            }],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let l = locals(&[
            ("local-0", Type::I32),
            ("local-1", Type::I32),
            ("local-2", Type::I32),
            ("local-3", Type::I32),
        ]);
        // block (func 0); try_table catch 0 0; i32.const 42; throw 0; end;
        // unreachable; end; local.set 3; end
        let code = vec![
            0x02, 0x00, 0x1f, 0x40, 0x01, 0x00, 0x00, 0x00, 0x41, 0x2a, 0x08, 0x00, 0x0b, 0x00,
            0x0b, 0x21, 0x03, 0x0b,
        ];
        let exprs = run(&module, &func, &l, &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Setf(\"local-3\", Block(\"block-0\", Progn([Try { name: \"try-1\", \
             body: Throw(0, [Const(\"42\")]), handlers: [CatchHandler { tag: Some(0), \
             vars: [\"try-1-exn-0\"], exnref_var: None, body: Local(\"try-1-exn-0\") }] }])))]"
        );
    }

    #[test]
    fn expressionify_try_table_catch_all_ref() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![Type::ExnRef],
            }],
            vec![],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let l = locals(&[
            ("local-0", Type::I32),
            ("local-1", Type::I32),
            ("local-2", Type::I32),
            ("local-3", Type::I32),
            ("local-4", Type::ExnRef),
        ]);
        // block (func 0); try_table catch_all_ref 0; i32.const 0; drop; end;
        // unreachable; end; local.set 4; end
        let code = vec![
            0x02, 0x00, 0x1f, 0x40, 0x01, 0x03, 0x00, 0x41, 0x00, 0x1a, 0x0b, 0x00, 0x0b, 0x21,
            0x04, 0x0b,
        ];
        let exprs = run(&module, &func, &l, &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Setf(\"local-4\", Block(\"block-0\", Progn([Try { name: \"try-1\", \
             body: Const(\"0\"), handlers: [CatchHandler { tag: None, vars: [], \
             exnref_var: Some(\"try-1-exn-ref\"), body: Local(\"try-1-exn-ref\") }] }, \
             Prim(Unreachable, [])])))]"
        );
    }

    #[test]
    fn expressionify_throw_ref() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        let l = locals(&[("param-0", Type::ExnRef)]);
        // local.get 0; throw_ref; end
        let exprs = run(&module, &func, &l, &[0x20, 0x00, 0x0a, 0x0b]).unwrap();
        assert_eq!(format!("{exprs:?}"), "[ThrowRef(Local(\"param-0\"))]");
    }

    #[test]
    fn expressionify_throw_uses_tag_index() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // i32.const 42; throw 0; end
        let exprs = run(&module, &func, &[], &[0x41, 0x2a, 0x08, 0x00, 0x0b]).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Throw(0, [Const(\"42\")])]");
    }

    #[test]
    fn rethrow_without_try_bails() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // block; rethrow 0; end; end
        let code = [0x02, 0x40, 0x09, 0x00, 0x0b, 0x0b];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(
            err.contains("`rethrow` depth does not target a `try` block"),
            "{err}"
        );
    }

    #[test]
    fn multi_value_end_requires_local_set() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![Type::I32, Type::ExnRef],
            }],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // Same as the landing pad but the block result is consumed by a `drop`.
        let mut code = landing_pad_code();
        code[15] = 0x1a; // local.set 4 -> drop
        let err = err_message(run(&module, &func, &[], &code));
        assert!(
            err.contains(
                "multi-value block results must be consumed by consecutive local.set, got Drop"
            ),
            "{err}"
        );
    }

    #[test]
    fn try_table_catch_into_loop_bails() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // loop; try_table catch 0 0; i32.const 42; throw 0; end; end; end
        let code = [
            0x03, 0x40, 0x1f, 0x40, 0x01, 0x00, 0x00, 0x00, 0x41, 0x2a, 0x08, 0x00, 0x0b, 0x0b,
            0x0b,
        ];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(
            err.contains("try_table catch delivering a value into a loop unsupported"),
            "{err}"
        );
    }

    #[test]
    fn unsupported_operator_bails() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // ref.null func; end
        let code = [0xd0, 0x00, 0x0b];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(err.contains("unsupported operator"), "{err}");
    }

    #[test]
    fn unreachable_nested_block_swallowed() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // unreachable; block (empty); i32.const 1; end; end
        let code = [0x00, 0x02, 0x40, 0x41, 0x01, 0x0b, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Prim(Unreachable, [])]");
    }

    #[test]
    fn unreachable_if_else_swallowed() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // unreachable; if (empty); i32.const 1; else; i32.const 2; end; end; end
        let code = [
            0x00, 0x04, 0x40, 0x41, 0x01, 0x05, 0x41, 0x02, 0x0b, 0x0b, 0x0b,
        ];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Prim(Unreachable, [])]");
    }

    #[test]
    fn targeted_block_becomes_reachable() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // block (empty); br 0; end; i32.const 2; end
        let code = [0x02, 0x40, 0x0c, 0x00, 0x0b, 0x41, 0x02, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Block(\"block-0\", Progn([ReturnFrom(\"block-0\", Progn([]))])), Const(\"2\")]"
        );
    }

    #[test]
    fn loop_back_edge_stays_unreachable() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // unreachable; loop (empty); br 0; end; end
        let code = [0x00, 0x03, 0x40, 0x0c, 0x00, 0x0b, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Prim(Unreachable, [])]");
    }

    #[test]
    fn br_if_carries_block_value() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // block (i32); i32.const 5; i32.const 1; br_if 0; i32.const 5; end; end
        let code = [
            0x02, 0x7f, 0x41, 0x05, 0x41, 0x01, 0x0d, 0x00, 0x41, 0x05, 0x0b, 0x0b,
        ];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Block(\"block-0\", Progn([If(Const(\"1\"), ReturnFrom(\"block-0\", \
             Const(\"5\")), Progn([])), Const(\"5\")]))]"
        );
    }

    #[test]
    fn br_table_multi_target_switch() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // block (empty); block (empty); i32.const 0; br_table 0 1; end; end; end
        let code = [
            0x02, 0x40, 0x02, 0x40, 0x41, 0x00, 0x0e, 0x01, 0x00, 0x01, 0x0b, 0x0b, 0x0b,
        ];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Block(\"block-0\", Progn([Block(\"block-1\", \
             Progn([Switch(Const(\"0\"), ReturnFrom(\"block-0\", Progn([])), \
             [ReturnFrom(\"block-1\", Progn([]))])]))]))]"
        );
    }

    #[test]
    fn br_to_value_block_bails() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // block (i32); i32.const 1; br 0; end; end
        let code = [0x02, 0x7f, 0x41, 0x01, 0x0c, 0x00, 0x0b, 0x0b];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(err.contains("value-typed non-loop block"), "{err}");
    }

    #[test]
    fn try_catch_emits_try() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // try (empty); i32.const 1; throw 0; catch 0; drop; end; i32.const 9; end
        let code = [
            0x06, 0x40, 0x41, 0x01, 0x08, 0x00, 0x07, 0x00, 0x1a, 0x0b, 0x41, 0x09, 0x0b,
        ];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Try { name: \"try-0\", body: Throw(0, [Const(\"1\")]), handlers: \
             [CatchHandler { tag: Some(0), vars: [\"try-0-exn-0\"], exnref_var: None, \
             body: Local(\"try-0-exn-0\") }] }, Const(\"9\")]"
        );
    }

    #[test]
    fn try_catch_all_emits_try() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // try (empty); throw 0; catch_all; end; i32.const 9; end
        let code = [0x06, 0x40, 0x08, 0x00, 0x19, 0x0b, 0x41, 0x09, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Try { name: \"try-0\", body: Throw(0, []), handlers: [CatchHandler { tag: None, \
             vars: [], exnref_var: None, body: Progn([]) }] }, Const(\"9\")]"
        );
    }

    #[test]
    fn try_delegate_emits_progn() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // try (empty); throw 0; delegate 0; end
        let code = [0x06, 0x40, 0x08, 0x00, 0x18, 0x00, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(format!("{exprs:?}"), "[Progn([Throw(0, [])])]");
    }

    #[test]
    fn else_resets_unreachability() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // i32.const 1; if (i32); unreachable; else; i32.const 2; end; end
        let code = [0x41, 0x01, 0x04, 0x7f, 0x00, 0x05, 0x41, 0x02, 0x0b, 0x0b];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[If(Const(\"1\"), Prim(Unreachable, []), Const(\"2\"))]"
        );
    }

    #[test]
    fn try_handler_end_becomes_reachable() {
        let module = module_with(
            vec![],
            vec![FuncType {
                params: vec![Type::I32],
                results: vec![],
            }],
            vec![],
        );
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![Type::I32],
            },
        );
        // try (empty); i32.const 1; throw 0; catch 0; drop; unreachable; end;
        // i32.const 9; end
        let code = [
            0x06, 0x40, 0x41, 0x01, 0x08, 0x00, 0x07, 0x00, 0x1a, 0x00, 0x0b, 0x41, 0x09, 0x0b,
        ];
        let exprs = run(&module, &func, &[], &code).unwrap();
        assert_eq!(
            format!("{exprs:?}"),
            "[Try { name: \"try-0\", body: Throw(0, [Const(\"1\")]), handlers: \
             [CatchHandler { tag: Some(0), vars: [\"try-0-exn-0\"], exnref_var: None, \
             body: Progn([Local(\"try-0-exn-0\"), Prim(Unreachable, [])]) }] }, \
             Const(\"9\")]"
        );
    }

    #[test]
    fn function_end_stack_not_empty_bails() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // i32.const 1; end
        let code = [0x41, 0x01, 0x0b];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(
            err.contains("stack not empty at end of function body"),
            "{err}"
        );
    }

    #[test]
    fn block_end_stack_not_empty_bails() {
        let module = module_with(vec![], vec![], vec![]);
        let func = function(
            0,
            FuncType {
                params: vec![],
                results: vec![],
            },
        );
        // block (empty); i32.const 1; end; end
        let code = [0x02, 0x40, 0x41, 0x01, 0x0b, 0x0b];
        let err = err_message(run(&module, &func, &[], &code));
        assert!(err.contains("stack not empty at block `end`"), "{err}");
    }
}
