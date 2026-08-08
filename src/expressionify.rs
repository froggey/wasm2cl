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

fn single_expr(mut exprs: Vec<Expr>) -> Expr {
    if exprs.len() == 1 {
        exprs.pop().unwrap()
    } else {
        Expr::Progn(exprs)
    }
}

fn block_result_count(blockty: &wasmparser::BlockType, module: &Module) -> usize {
    match blockty {
        wasmparser::BlockType::Empty => 0,
        wasmparser::BlockType::Type(_) => 1,
        wasmparser::BlockType::FuncType(idx) => module.types[*idx as usize].results.len(),
    }
}

fn append_side_effect(exprs: &mut Vec<Expr>, stack: &mut Vec<Expr>, new: Expr) {
    if stack.is_empty() {
        exprs.push(new);
    } else {
        let last = stack.len() - 1;
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
pub fn expressionify_function_body(
    module: &Module,
    func: &Function,
    all_locals: &[(String, Type)],
    ops: &mut wasmparser::OperatorsReader,
) -> Result<Vec<Expr>> {
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
            Try { .. } if unreachable => {
                unreachable_depth += 1;
            }
            TryTable { .. } if unreachable => {
                unreachable_depth += 1;
            }
            Else if unreachable && unreachable_depth != 0 => {}
            Catch { .. } if unreachable && unreachable_depth != 0 => {}
            CatchAll if unreachable && unreachable_depth != 0 => {}
            End if unreachable && unreachable_depth != 0 => {
                unreachable_depth -= 1;
            }
            Delegate { .. } if unreachable && unreachable_depth != 0 => {
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
                    handlers: vec![],
                    try_body: None,
                    current_handler: None,
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
                    handlers: vec![],
                    try_body: None,
                    current_handler: None,
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
                    handlers: vec![],
                    try_body: None,
                    current_handler: None,
                });
            }
            Try { blockty } => {
                block_stack.push(ActiveBlock {
                    kind: BlockKind::Try,
                    blockty,
                    old_exprs: std::mem::take(&mut exprs),
                    then: None,
                    name: format!("try-{pc}"),
                    targeted: false,
                    stack: std::mem::take(&mut stack),
                    handlers: vec![],
                    try_body: None,
                    current_handler: None,
                });
            }
            TryTable { try_table } => {
                // Each catch clause branches to an outer label (resolved before
                // this try_table's own frame is pushed), delivering the payload
                // as that label's value. The payload becomes the catch clause's
                // body, so `wasm-try` evaluates to it on the catch path.
                let name = format!("try-{pc}");
                let mut handlers = Vec::new();
                for catch in try_table.catches {
                    let handler = match catch {
                        wasmparser::Catch::One { tag, label } => {
                            let target_idx = block_stack.len() - 1 - (label as usize);
                            let ty = &module.tags[tag as usize];
                            if ty.params.len() > 1 {
                                bail!("try_table catch payload with multiple values unsupported");
                            }
                            let vars: Vec<String> = (0..ty.params.len())
                                .map(|i| format!("{name}-exn-{i}"))
                                .collect();
                            let body = if matches!(block_stack[target_idx].kind, BlockKind::Loop) {
                                if !ty.params.is_empty() {
                                    bail!(
                                        "try_table catch delivering a value into a loop unsupported"
                                    );
                                }
                                Expr::Go(block_stack[target_idx].name.clone())
                            } else if ty.params.is_empty() {
                                Expr::Progn(vec![])
                            } else {
                                Expr::Local(vars[0].clone())
                            };
                            block_stack[target_idx].targeted = true;
                            CatchHandler {
                                tag: Some(tag),
                                vars,
                                exnref_var: None,
                                body: Box::new(body),
                            }
                        }
                        wasmparser::Catch::All { label } => {
                            let target_idx = block_stack.len() - 1 - (label as usize);
                            let body = if matches!(block_stack[target_idx].kind, BlockKind::Loop) {
                                Expr::Go(block_stack[target_idx].name.clone())
                            } else {
                                Expr::Progn(vec![])
                            };
                            block_stack[target_idx].targeted = true;
                            CatchHandler {
                                tag: None,
                                vars: vec![],
                                exnref_var: None,
                                body: Box::new(body),
                            }
                        }
                        wasmparser::Catch::OneRef { tag, label } => {
                            let target_idx = block_stack.len() - 1 - (label as usize);
                            let ty = &module.tags[tag as usize];
                            if ty.params.len() > 1 {
                                bail!(
                                    "try_table catch_ref payload with multiple values unsupported"
                                );
                            }
                            if matches!(block_stack[target_idx].kind, BlockKind::Loop) {
                                bail!(
                                    "try_table catch_ref delivering a value into a loop unsupported"
                                );
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
                            block_stack[target_idx].targeted = true;
                            CatchHandler {
                                tag: Some(tag),
                                vars,
                                exnref_var: Some(ref_var),
                                body: Box::new(body),
                            }
                        }
                        wasmparser::Catch::AllRef { label } => {
                            let target_idx = block_stack.len() - 1 - (label as usize);
                            if matches!(block_stack[target_idx].kind, BlockKind::Loop) {
                                bail!(
                                    "try_table catch_all_ref delivering a value into a loop unsupported"
                                );
                            }
                            let ref_var = format!("{name}-exn-ref");
                            block_stack[target_idx].targeted = true;
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
                block_stack.push(ActiveBlock {
                    kind: BlockKind::TryTable,
                    blockty: try_table.ty,
                    old_exprs: std::mem::take(&mut exprs),
                    then: None,
                    name,
                    targeted: false,
                    stack: std::mem::take(&mut stack),
                    handlers,
                    try_body: None,
                    current_handler: None,
                });
            }
            Else => {
                let was_unreachable = unreachable;
                unreachable = false;
                let idx = block_stack.len() - 1;
                if !matches!(block_stack[idx].kind, BlockKind::If) {
                    bail!("`else` outside of an `if` block");
                }
                if block_stack[idx].then.is_some() {
                    bail!("`else` after a previous `else`");
                }
                if !was_unreachable
                    && !matches!(block_stack[idx].blockty, wasmparser::BlockType::Empty)
                {
                    exprs.push(stack.pop().unwrap());
                }
                if !was_unreachable && !stack.is_empty() {
                    bail!("stack not empty at `else`");
                }
                block_stack[idx].then = Some(std::mem::take(&mut exprs));
            }
            End => {
                let Some(entry) = block_stack.pop() else {
                    // End of the function
                    break;
                };

                let was_unreachable = unreachable;

                // A non-loop block targeted by Br means the End is reachable (forward jump).
                // A loop targeted by Br means the back-edge exists, but the fall-through End
                // is still unreachable.
                // A try that has a handler is reachable at its End even if the current handler
                // ended in a br/throw: the try body's normal-completion path reaches the End.
                let becomes_reachable = match entry.kind {
                    BlockKind::Loop => false,
                    BlockKind::Try => {
                        was_unreachable && (entry.targeted || entry.try_body.is_some())
                    }
                    BlockKind::TryTable => was_unreachable && entry.targeted,
                    _ => was_unreachable && entry.targeted,
                };

                unreachable = was_unreachable && !becomes_reachable;

                // End of the current block.
                // Only pop the block's result from the stack on reachable fall-through.
                if !was_unreachable && !matches!(entry.blockty, wasmparser::BlockType::Empty) {
                    exprs.push(stack.pop().unwrap());
                }
                if !was_unreachable && !stack.is_empty() {
                    bail!("stack not empty at block `end`");
                }
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
                            }),
                        )
                    }
                    BlockKind::Block => Expr::Progn(std::mem::replace(&mut exprs, entry.old_exprs)),
                    BlockKind::Loop => Expr::Progn(std::mem::replace(&mut exprs, entry.old_exprs)),
                    BlockKind::Try => {
                        let try_body = match entry.try_body {
                            Some(body) => body,
                            None => std::mem::take(&mut exprs),
                        };
                        let mut handlers = entry.handlers;
                        if let Some((tag, vars)) = entry.current_handler {
                            handlers.push(CatchHandler {
                                tag,
                                vars,
                                exnref_var: None,
                                body: Box::new(single_expr(std::mem::replace(
                                    &mut exprs,
                                    entry.old_exprs,
                                ))),
                            });
                        } else {
                            exprs = entry.old_exprs;
                        }
                        Expr::Try {
                            name: entry.name.clone(),
                            body: Box::new(single_expr(try_body)),
                            handlers,
                        }
                    }
                    BlockKind::TryTable => {
                        let try_body = std::mem::take(&mut exprs);
                        exprs = entry.old_exprs;
                        Expr::Try {
                            name: entry.name.clone(),
                            body: Box::new(single_expr(try_body)),
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
                let n_results = block_result_count(&entry.blockty, module);
                if n_results > 1 {
                    // Narrow multi-value: the block's results must be consumed by
                    // n_results immediate consecutive `local.set` ops (the shape
                    // clang emits for catch_ref landing pads). Wasm pops results
                    // off the block stack top-first (the exnref that was pushed
                    // last), while `(setf (values ...))` assigns first-value-first,
                    // so the locals are collected in stream order then reversed.
                    if unreachable {
                        bail!("multi-value block result in unreachable code unsupported");
                    }
                    let mut locals = Vec::with_capacity(n_results);
                    for _ in 0..n_results {
                        let op = ops.read()?;
                        match op {
                            wasmparser::Operator::LocalSet { local_index } => {
                                locals.push(all_locals[local_index as usize].0.clone());
                            }
                            op => bail!(
                                "multi-value block results must be consumed by consecutive \
                                 local.set, got {op:?}"
                            ),
                        }
                    }
                    locals.reverse();
                    append_side_effect(
                        &mut exprs,
                        &mut stack,
                        Expr::SetfValues {
                            locals,
                            value: Box::new(final_expr),
                        },
                    );
                } else if unreachable || n_results == 0 {
                    append_side_effect(&mut exprs, &mut stack, final_expr);
                } else {
                    stack.push(final_expr);
                }
            }
            Catch { tag_index } => {
                let was_unreachable = unreachable;
                unreachable = false;
                let idx = block_stack.len() - 1;
                if !matches!(block_stack[idx].kind, BlockKind::Try) {
                    bail!("`catch` outside of a `try` block");
                }
                if !was_unreachable
                    && !matches!(block_stack[idx].blockty, wasmparser::BlockType::Empty)
                {
                    exprs.push(stack.pop().unwrap());
                }
                if !was_unreachable && !stack.is_empty() {
                    bail!("stack not empty at `catch`");
                }
                let entry = &mut block_stack[idx];
                if entry.try_body.is_none() {
                    entry.try_body = Some(std::mem::take(&mut exprs));
                } else {
                    let (tag, vars) = entry.current_handler.take().unwrap();
                    entry.handlers.push(CatchHandler {
                        tag,
                        vars,
                        exnref_var: None,
                        body: Box::new(single_expr(std::mem::take(&mut exprs))),
                    });
                }
                stack.clear();
                let ty = &module.tags[tag_index as usize];
                let mut vars = vec![];
                for i in 0..ty.params.len() {
                    let var = format!("{}-exn-{i}", entry.name);
                    vars.push(var.clone());
                    stack.push(Expr::Local(var));
                }
                entry.current_handler = Some((Some(tag_index), vars));
            }
            CatchAll => {
                let was_unreachable = unreachable;
                unreachable = false;
                let idx = block_stack.len() - 1;
                if !matches!(block_stack[idx].kind, BlockKind::Try) {
                    bail!("`catch_all` outside of a `try` block");
                }
                if !was_unreachable
                    && !matches!(block_stack[idx].blockty, wasmparser::BlockType::Empty)
                {
                    exprs.push(stack.pop().unwrap());
                }
                if !was_unreachable && !stack.is_empty() {
                    bail!("stack not empty at `catch_all`");
                }
                let entry = &mut block_stack[idx];
                if entry.try_body.is_none() {
                    entry.try_body = Some(std::mem::take(&mut exprs));
                } else {
                    let (tag, vars) = entry.current_handler.take().unwrap();
                    entry.handlers.push(CatchHandler {
                        tag,
                        vars,
                        exnref_var: None,
                        body: Box::new(single_expr(std::mem::take(&mut exprs))),
                    });
                }
                stack.clear();
                entry.current_handler = Some((None, vec![]));
            }
            Delegate { relative_depth: _ } => {
                let entry = block_stack.pop().unwrap();
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
                let was_unreachable = unreachable;
                if !was_unreachable && !matches!(entry.blockty, wasmparser::BlockType::Empty) {
                    exprs.push(stack.pop().unwrap());
                }
                if !was_unreachable && !stack.is_empty() {
                    bail!("stack not empty at `delegate`");
                }
                stack = entry.stack;
                let final_expr = Expr::Progn(std::mem::replace(&mut exprs, entry.old_exprs));
                if unreachable || matches!(entry.blockty, wasmparser::BlockType::Empty) {
                    append_side_effect(&mut exprs, &mut stack, final_expr);
                } else {
                    stack.push(final_expr);
                }
            }

            _ if unreachable => {} // Nothing

            // Normal execution.
            Throw { tag_index } => {
                let ty = &module.tags[tag_index as usize];
                let payload = stack.split_off(stack.len() - ty.params.len());
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Throw(tag_index as usize, payload),
                );
                unreachable = true;
            }
            Rethrow { relative_depth } => {
                let target = block_stack.len() - 1 - (relative_depth as usize);
                if !matches!(block_stack[target].kind, BlockKind::Try) {
                    bail!("`rethrow` depth does not target a `try` block");
                }
                let exn_var = format!("{}-exn", block_stack[target].name);
                append_side_effect(&mut exprs, &mut stack, Expr::Rethrow(exn_var));
                unreachable = true;
            }
            ThrowRef => {
                let exnref = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack, Expr::ThrowRef(Box::new(exnref)));
                unreachable = true;
            }
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
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::GlobalSet(global_index as usize, Box::new(value)),
                );
            }
            LocalGet { local_index } => {
                stack.push(Expr::Local(all_locals[local_index as usize].0.clone()));
            }
            LocalSet { local_index } => {
                let value = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Setf(all_locals[local_index as usize].0.clone(), Box::new(value)),
                );
            }
            LocalTee { local_index } => {
                let value = stack.pop().unwrap();
                stack.push(Expr::Setf(
                    all_locals[local_index as usize].0.clone(),
                    Box::new(value),
                ));
            }
            Drop => {
                let value = stack.pop().unwrap();
                append_side_effect(&mut exprs, &mut stack, value);
            }
            Unreachable => {
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Prim(Primitive::Unreachable, vec![]),
                );
                unreachable = true;
            }
            Return => {
                let value = if func.ty.results.is_empty() {
                    Expr::Progn(vec![])
                } else {
                    stack.pop().unwrap()
                };
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::ReturnFrom("nil".to_string(), Box::new(value)),
                );
                unreachable = true;
            }
            Call { function_index } => {
                let target = &module.functions[function_index as usize];
                let args = stack.split_off(stack.len() - target.ty.params.len());
                match target.ty.results.len() {
                    // call for effect
                    0 => {
                        append_side_effect(&mut exprs, &mut stack, Expr::Call(target.name(), args))
                    }
                    // call for single value
                    1 => stack.push(Expr::Call(target.name(), args)),
                    n => bail!("call producing multiple values ({n})"),
                }
            }
            CallIndirect {
                type_index,
                table_index,
            } => {
                if table_index != 0 {
                    bail!("call_indirect with non-zero table index ({table_index})");
                }
                let ty = &module.types[type_index as usize];
                let idx = stack.pop().unwrap();
                let args = stack.split_off(stack.len() - ty.params.len());
                match ty.results.len() {
                    // call for effect
                    0 => append_side_effect(
                        &mut exprs,
                        &mut stack,
                        Expr::CallIndirect(Box::new(idx), args),
                    ),
                    // call for single value
                    1 => stack.push(Expr::CallIndirect(Box::new(idx), args)),
                    n => bail!("call_indirect producing multiple values ({n})"),
                }
            }
            Br { relative_depth } => {
                let target = block_stack.len() - 1 - (relative_depth as usize);
                unreachable = true;
                block_stack[target].targeted = true;
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    if matches!(block_stack[target].kind, BlockKind::Loop) {
                        Expr::Go(block_stack[target].name.clone())
                    } else {
                        if !matches!(block_stack[target].blockty, wasmparser::BlockType::Empty) {
                            bail!(
                                "`br` to a value-typed non-loop block unsupported \
                                 (target {:?})",
                                block_stack[target].blockty
                            );
                        }
                        Expr::ReturnFrom(
                            block_stack[target].name.clone(),
                            Box::new(Expr::Progn(vec![])),
                        )
                    },
                );
            }
            BrIf { relative_depth } => {
                let target = block_stack.len() - 1 - (relative_depth as usize);
                let test = stack.pop().unwrap();
                block_stack[target].targeted = true;
                let value = if !matches!(block_stack[target].kind, BlockKind::Loop)
                    && !matches!(block_stack[target].blockty, wasmparser::BlockType::Empty)
                {
                    stack.pop().unwrap()
                } else {
                    Expr::Progn(vec![])
                };
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::If(
                        Box::new(test),
                        Box::new(if matches!(block_stack[target].kind, BlockKind::Loop) {
                            Expr::Go(block_stack[target].name.clone())
                        } else {
                            Expr::ReturnFrom(block_stack[target].name.clone(), Box::new(value))
                        }),
                        Box::new(Expr::Progn(vec![])),
                    ),
                );
            }
            BrTable { targets } => {
                let idx = stack.pop().unwrap();
                let mut target_code = vec![];
                for target_idx in targets.targets() {
                    let target_idx = target_idx?;
                    let target = block_stack.len() - 1 - (target_idx as usize);
                    if !matches!(block_stack[target].blockty, wasmparser::BlockType::Empty) {
                        bail!("br_table target with a value-typed block unsupported");
                    }
                    block_stack[target].targeted = true;
                    target_code.push(if matches!(block_stack[target].kind, BlockKind::Loop) {
                        Expr::Go(block_stack[target].name.clone())
                    } else {
                        Expr::ReturnFrom(
                            block_stack[target].name.clone(),
                            Box::new(Expr::Progn(vec![])),
                        )
                    });
                }
                let default_target = block_stack.len() - 1 - (targets.default() as usize);
                if !matches!(
                    block_stack[default_target].blockty,
                    wasmparser::BlockType::Empty
                ) {
                    bail!("br_table default target with a value-typed block unsupported");
                }
                block_stack[default_target].targeted = true;
                let default_code = if matches!(block_stack[default_target].kind, BlockKind::Loop) {
                    Expr::Go(block_stack[default_target].name.clone())
                } else {
                    Expr::ReturnFrom(
                        block_stack[default_target].name.clone(),
                        Box::new(Expr::Progn(vec![])),
                    )
                };
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Switch(Box::new(idx), Box::new(default_code), target_code),
                );
            }
            Select => {
                let cond = stack.pop().unwrap();
                let rhs = stack.pop().unwrap();
                let lhs = stack.pop().unwrap();
                stack.push(Expr::Select(Box::new(lhs), Box::new(rhs), Box::new(cond)));
            }
            MemoryCopy { dst_mem, src_mem } => {
                if dst_mem != 0 || src_mem != 0 {
                    bail!("memory.copy with non-zero memory index unsupported");
                }
                let n = stack.pop().unwrap();
                let src = stack.pop().unwrap();
                let dst = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Call("memory-copy".to_string(), vec![dst, src, n]),
                );
            }
            MemoryFill { mem } => {
                if mem != 0 {
                    bail!("memory.fill with non-zero memory index unsupported");
                }
                let n = stack.pop().unwrap();
                let value = stack.pop().unwrap();
                let dst = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::Call("memory-fill".to_string(), vec![dst, value, n]),
                );
            }
            MemorySize { mem } => {
                if mem != 0 {
                    bail!("memory.size with non-zero memory index unsupported");
                }
                stack.push(Expr::Call("memory-size".to_string(), vec![]));
            }
            MemoryGrow { mem } => {
                if mem != 0 {
                    bail!("memory.grow with non-zero memory index unsupported");
                }
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
                stack.push(Expr::I32Load(
                    Some((8, false)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I32Load8S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(
                    Some((8, true)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I32Load16U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(
                    Some((16, false)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I32Load16S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I32Load(
                    Some((16, true)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I32Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I32Store(
                        None,
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
            }
            I32Store8 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I32Store(
                        Some(8),
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
            }
            I32Store16 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I32Store(
                        Some(16),
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
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
            I32Popcnt => prim_op1(Primitive::I32Popcnt, &mut stack),
            I32WrapI64 => prim_op1(Primitive::I32WrapI64, &mut stack),
            I32Extend8S => prim_op1(Primitive::I32Extend8S, &mut stack),
            I32Extend16S => prim_op1(Primitive::I32Extend16S, &mut stack),
            I32TruncF32U => prim_op1(Primitive::I32TruncF32U, &mut stack),
            I32TruncF32S => prim_op1(Primitive::I32TruncF32S, &mut stack),
            I32TruncF64U => prim_op1(Primitive::I32TruncF64U, &mut stack),
            I32TruncF64S => prim_op1(Primitive::I32TruncF64S, &mut stack),
            I32TruncSatF32U => prim_op1(Primitive::I32TruncSatF32U, &mut stack),
            I32TruncSatF32S => prim_op1(Primitive::I32TruncSatF32S, &mut stack),
            I32TruncSatF64U => prim_op1(Primitive::I32TruncSatF64U, &mut stack),
            I32TruncSatF64S => prim_op1(Primitive::I32TruncSatF64S, &mut stack),
            I32ReinterpretF32 => prim_op1(Primitive::I32ReinterpretF32, &mut stack),
            /* I64 */
            I64Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(None, Box::new(addr), memarg.offset as usize));
            }
            I64Load8U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((8, false)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Load8S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((8, true)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Load16U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((16, false)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Load16S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((16, true)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Load32U { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((32, false)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Load32S { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::I64Load(
                    Some((32, true)),
                    Box::new(addr),
                    memarg.offset as usize,
                ));
            }
            I64Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I64Store(
                        None,
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
            }
            I64Store8 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I64Store(
                        Some(8),
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
            }
            I64Store16 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I64Store(
                        Some(16),
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
            }
            I64Store32 { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::I64Store(
                        Some(32),
                        Box::new(addr),
                        Box::new(value),
                        memarg.offset as usize,
                    ),
                );
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
            I64Popcnt => prim_op1(Primitive::I64Popcnt, &mut stack),
            I64Extend8S => prim_op1(Primitive::I64Extend8S, &mut stack),
            I64Extend16S => prim_op1(Primitive::I64Extend16S, &mut stack),
            I64Extend32S => prim_op1(Primitive::I64Extend32S, &mut stack),
            I64ExtendI32U => prim_op1(Primitive::I64ExtendI32U, &mut stack),
            I64ExtendI32S => prim_op1(Primitive::I64ExtendI32S, &mut stack),
            I64TruncF32U => prim_op1(Primitive::I64TruncF32U, &mut stack),
            I64TruncF32S => prim_op1(Primitive::I64TruncF32S, &mut stack),
            I64TruncF64U => prim_op1(Primitive::I64TruncF64U, &mut stack),
            I64TruncF64S => prim_op1(Primitive::I64TruncF64S, &mut stack),
            I64TruncSatF32U => prim_op1(Primitive::I64TruncSatF32U, &mut stack),
            I64TruncSatF32S => prim_op1(Primitive::I64TruncSatF32S, &mut stack),
            I64TruncSatF64U => prim_op1(Primitive::I64TruncSatF64U, &mut stack),
            I64TruncSatF64S => prim_op1(Primitive::I64TruncSatF64S, &mut stack),
            I64ReinterpretF64 => prim_op1(Primitive::I64ReinterpretF64, &mut stack),
            /* F32 */
            F32Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::F32Load(Box::new(addr), memarg.offset as usize));
            }
            F32Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::F32Store(Box::new(addr), Box::new(value), memarg.offset as usize),
                );
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
            F32Abs => prim_op1(Primitive::F32Abs, &mut stack),
            F32Neg => prim_op1(Primitive::F32Neg, &mut stack),
            F32Sqrt => prim_op1(Primitive::F32Sqrt, &mut stack),
            F32Floor => prim_op1(Primitive::F32Floor, &mut stack),
            F32Ceil => prim_op1(Primitive::F32Ceil, &mut stack),
            F32Trunc => prim_op1(Primitive::F32Trunc, &mut stack),
            F32Nearest => prim_op1(Primitive::F32Nearest, &mut stack),
            F32Min => prim_op2(Primitive::F32Min, &mut stack),
            F32Max => prim_op2(Primitive::F32Max, &mut stack),
            F32Copysign => prim_op2(Primitive::F32Copysign, &mut stack),
            F32ConvertI32U => prim_op1(Primitive::F32ConvertI32U, &mut stack),
            F32ConvertI32S => prim_op1(Primitive::F32ConvertI32S, &mut stack),
            F32ConvertI64U => prim_op1(Primitive::F32ConvertI64U, &mut stack),
            F32ConvertI64S => prim_op1(Primitive::F32ConvertI64S, &mut stack),
            F32ReinterpretI32 => prim_op1(Primitive::F32ReinterpretI32, &mut stack),
            F32DemoteF64 => prim_op1(Primitive::F32DemoteF64, &mut stack),
            /* F64 */
            F64Load { memarg } => {
                let addr = stack.pop().unwrap();
                stack.push(Expr::F64Load(Box::new(addr), memarg.offset as usize));
            }
            F64Store { memarg } => {
                let value = stack.pop().unwrap();
                let addr = stack.pop().unwrap();
                append_side_effect(
                    &mut exprs,
                    &mut stack,
                    Expr::F64Store(Box::new(addr), Box::new(value), memarg.offset as usize),
                );
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
            F64Abs => prim_op1(Primitive::F64Abs, &mut stack),
            F64Neg => prim_op1(Primitive::F64Neg, &mut stack),
            F64Sqrt => prim_op1(Primitive::F64Sqrt, &mut stack),
            F64Floor => prim_op1(Primitive::F64Floor, &mut stack),
            F64Ceil => prim_op1(Primitive::F64Ceil, &mut stack),
            F64Trunc => prim_op1(Primitive::F64Trunc, &mut stack),
            F64Nearest => prim_op1(Primitive::F64Nearest, &mut stack),
            F64Min => prim_op2(Primitive::F64Min, &mut stack),
            F64Max => prim_op2(Primitive::F64Max, &mut stack),
            F64Copysign => prim_op2(Primitive::F64Copysign, &mut stack),
            F64ConvertI32U => prim_op1(Primitive::F64ConvertI32U, &mut stack),
            F64ConvertI32S => prim_op1(Primitive::F64ConvertI32S, &mut stack),
            F64ConvertI64U => prim_op1(Primitive::F64ConvertI64U, &mut stack),
            F64ConvertI64S => prim_op1(Primitive::F64ConvertI64S, &mut stack),
            F64ReinterpretI64 => prim_op1(Primitive::F64ReinterpretI64, &mut stack),
            F64PromoteF32 => prim_op1(Primitive::F64PromoteF32, &mut stack),
            op => bail!("unsupported operator: {op:?}"),
        }
        pc += 1;
    }

    if !unreachable {
        if !func.ty.results.is_empty() {
            exprs.push(stack.pop().unwrap())
        }

        if !stack.is_empty() {
            bail!("stack not empty at end of function body");
        }
    }

    Ok(exprs)
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
            single_expr(vec![Expr::Const("1".into())]),
            Expr::Const(_)
        ));
        assert!(matches!(single_expr(vec![]), Expr::Progn(ref e) if e.is_empty()));
        assert!(matches!(
            single_expr(vec![Expr::Const("1".into()), Expr::Const("2".into())]),
            Expr::Progn(ref e) if e.len() == 2
        ));
    }

    #[test]
    fn append_side_effect_pushes_to_empty_exprs() {
        let mut exprs = vec![];
        let mut stack = vec![];
        append_side_effect(&mut exprs, &mut stack, Expr::Const("1".into()));
        assert_eq!(format!("{exprs:?}"), "[Const(\"1\")]");
        assert!(stack.is_empty());
    }

    #[test]
    fn append_side_effect_wraps_value_in_prog1() {
        let mut exprs = vec![];
        let mut stack = vec![Expr::Const("1".into())];
        append_side_effect(&mut exprs, &mut stack, Expr::Const("2".into()));
        assert!(exprs.is_empty());
        assert_eq!(
            format!("{stack:?}"),
            "[Prog1(Const(\"1\"), [Const(\"2\")])]"
        );
    }

    #[test]
    fn append_side_effect_appends_to_existing_prog1() {
        let mut exprs = vec![];
        let mut stack = vec![Expr::Prog1(Box::new(Expr::Const("1".into())), vec![])];
        append_side_effect(&mut exprs, &mut stack, Expr::Const("2".into()));
        assert!(exprs.is_empty());
        assert_eq!(
            format!("{stack:?}"),
            "[Prog1(Const(\"1\"), [Const(\"2\")])]"
        );
    }

    #[test]
    fn prim_op1_pops_single_operand() {
        let mut stack = vec![Expr::Const("5".into())];
        prim_op1(Primitive::I32Eqz, &mut stack);
        assert_eq!(format!("{stack:?}"), "[Prim(I32Eqz, [Const(\"5\")])]");
    }

    #[test]
    fn prim_op2_keeps_lhs_rhs_order() {
        // lhs pushed first, rhs popped first
        let mut stack = vec![Expr::Local("x".into()), Expr::Const("3".into())];
        prim_op2(Primitive::I32Add, &mut stack);
        assert_eq!(
            format!("{stack:?}"),
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
        use wasmparser::BlockType;
        assert_eq!(block_result_count(&BlockType::Empty, &module), 0);
        assert_eq!(
            block_result_count(&BlockType::Type(wasmparser::ValType::I32), &module),
            1
        );
        assert_eq!(block_result_count(&BlockType::FuncType(0), &module), 2);
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
}
