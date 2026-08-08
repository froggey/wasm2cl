/// Emit Common Lisp source files from expression trees.

use anyhow::Result;
use std::{fs, io::BufWriter, path::Path};

use crate::expr::Expr;
use crate::expressionify::expressionify_function_body;
use crate::module::{Function, Module, Type};
use crate::symbolicate;

fn convert_type(t: Type) -> &'static str {
    match t {
        Type::I32 => "i32",
        Type::I64 => "i64",
        Type::F32 => "f32",
        Type::F64 => "f64",
        Type::V128 => "v128",
        Type::FuncRef => "func-ref",
        Type::ExternRef => "extern-ref",
        Type::ExnRef => "exnref",
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
        Type::ExnRef => "nil",
    }
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
        GlobalSet(idx, value) => format!(
            "(setf (global context {idx}) {})",
            convert_expr(
                value,
                indent + (format!("(setf (global context {idx}) ").len())
            )
        ),
        Local(s) => s.clone(),
        Setf(name, value) => format!(
            "(setf {name} {})",
            convert_expr(value, indent + (format!("(setf {name} ").len()))
        ),
        Call(name, args) => {
            let mut result = String::new();
            let mut indent = indent;
            result.push('(');
            result.push_str(name);
            result.push_str(" context");
            indent += result.len();
            for e in args.iter() {
                indent += 1;
                result.push(' ');
                let s = convert_expr(e, indent);
                indent += s.len();
                result.push_str(&s);
            }
            result.push(')');
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
                    result.push('\n');
                    result.push_str(&make_indent(indent));
                }
                result.push_str(&format!("({temp} "));
                result.push_str(&convert_expr(val, indent + 2 + temp.len()));
                result.push(')');
            }
            result.push_str(")\n");
            result.push_str(&make_indent(indent + 2));
            result.push_str(" (call-indirect ");
            result.push_str(&convert_expr(idx, indent + 4));
            result.push_str(" context");
            for e in temps.iter() {
                result.push(' ');
                result.push_str(e);
            }
            result.push_str("))");
            result
        }
        Prim(name, args) => {
            let mut result = String::new();
            let mut indent = indent;
            result.push('(');
            result.push_str(&format!("{name:?}"));
            indent += result.len();
            for e in args.iter() {
                indent += 1;
                result.push(' ');
                let s = convert_expr(e, indent);
                indent += s.len();
                result.push_str(&s);
            }
            result.push(')');
            result
        }
        If(test, tru, fals) => {
            let mut result = String::new();
            result.push_str("(if ");
            if let Some((fused_name, fused_args)) = test.fused_pred() {
                result.push('(');
                result.push_str(fused_name);
                for a in fused_args.iter() {
                    result.push(' ');
                    result.push_str(&convert_expr(a, indent));
                }
                result.push_str(")\n");
            } else {
                result.push_str("(not (zerop ");
                result.push_str(&convert_expr(test, indent));
                result.push_str("))\n");
            }
            result.push_str(&make_indent(indent + 4));
            result.push_str(&convert_expr(tru, indent + 4));
            result.push('\n');
            result.push_str(&make_indent(indent + 4));
            result.push_str(&convert_expr(fals, indent + 4));
            result.push(')');
            result
        }
        Select(tru, fals, test) => {
            let mut result = String::new();
            result.push_str("(select ");
            result.push_str(&convert_expr(tru, indent));
            result.push(' ');
            result.push_str(&convert_expr(fals, indent));
            result.push(' ');
            if let Some((fused_name, fused_args)) = test.fused_pred() {
                result.push('(');
                result.push_str(fused_name);
                for a in fused_args.iter() {
                    result.push(' ');
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
        Progn(exprs) => match exprs.len() {
            0 => "()".to_string(),
            1 => convert_expr(&exprs[0], indent),
            _ => {
                let mut result = String::new();
                result.push_str("(progn");
                for e in exprs {
                    result.push('\n');
                    result.push_str(&make_indent(indent + 2));
                    result.push_str(&convert_expr(e, indent + 2));
                }
                result.push(')');
                result
            }
        },
        Prog1(value, exprs) => match exprs.len() {
            0 => convert_expr(value, indent),
            _ => {
                let mut result = String::new();
                result.push_str("(prog1 ");
                result.push_str(&convert_expr(value, indent + 2));
                for e in exprs {
                    result.push('\n');
                    result.push_str(&make_indent(indent + 2));
                    result.push_str(&convert_expr(e, indent + 2));
                }
                result.push(')');
                result
            }
        },
        Block(name, e) => {
            let mut result = String::new();
            result.push_str("(block ");
            result.push_str(name);
            result.push('\n');
            result.push_str(&make_indent(indent + 2));
            result.push_str(&convert_expr(e, indent + 2));
            result.push(')');
            result
        }
        ReturnFrom(name, value) => {
            let mut result = String::new();
            result.push_str("(return-from ");
            result.push_str(name);
            result.push(' ');
            result.push_str(&convert_expr(value, indent + 2));
            result.push(')');
            result
        }
        Tagbody(name, e) => {
            let mut result = String::new();
            result.push_str("(tagbody ");
            result.push_str(name);
            result.push('\n');
            result.push_str(&make_indent(indent + 2));
            result.push_str(&convert_expr(e, indent + 2));
            result.push(')');
            result
        }
        Go(name) => {
            let mut result = String::new();
            result.push_str("(go ");
            result.push_str(name);
            result.push(')');
            result
        }
        Switch(index, default_target, targets) => {
            let mut result = String::new();
            result.push_str("(case ");
            result.push_str(&convert_expr(index, indent + 2));
            result.push('\n');
            for (i, e) in targets.iter().enumerate() {
                result.push_str(&make_indent(indent + 2));
                result.push_str(&format!("({i} "));
                result.push_str(&convert_expr(e, indent + 2));
                result.push_str(")\n");
            }
            result.push_str(&make_indent(indent + 2));
            result.push_str("(otherwise ");
            result.push_str(&convert_expr(default_target, indent + 2));
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
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(')');
            result
        }
        I32Store(info, addr, value, addend) => {
            let mut result = String::new();
            result.push_str("(i32store");
            if let Some(width) = info {
                result.push_str(&format!("{width}"));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(' ');
            result.push_str(&convert_expr(value, indent + 4));
            result.push(')');
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
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(')');
            result
        }
        I64Store(info, addr, value, addend) => {
            let mut result = String::new();
            result.push_str("(i64store");
            if let Some(width) = info {
                result.push_str(&format!("{width}"));
            }
            result.push_str(" context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(' ');
            result.push_str(&convert_expr(value, indent + 4));
            result.push(')');
            result
        }
        F32Load(addr, addend) => {
            let mut result = String::new();
            result.push_str("(f32load context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(')');
            result
        }
        F32Store(addr, value, addend) => {
            let mut result = String::new();
            result.push_str("(f32store context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(' ');
            result.push_str(&convert_expr(value, indent + 4));
            result.push(')');
            result
        }
        F64Load(addr, addend) => {
            let mut result = String::new();
            result.push_str("(f64load context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(')');
            result
        }
        F64Store(addr, value, addend) => {
            let mut result = String::new();
            result.push_str("(f64store context ");
            if *addend == 0 {
                result.push_str(&convert_expr(addr, indent + 4));
            } else {
                result.push_str("(i32add ");
                result.push_str(&convert_expr(addr, indent + 4));
                result.push_str(&format!(" {addend})"));
            }
            result.push(' ');
            result.push_str(&convert_expr(value, indent + 4));
            result.push(')');
            result
        }
        Try {
            name,
            body,
            handlers,
        } => {
            let exn_var = format!("{name}-exn");
            let mut result = String::new();
            result.push_str(&format!("(wasm-try ({exn_var})\n"));
            result.push_str(&make_indent(indent + 2));
            result.push_str(&convert_expr(body, indent + 2));
            for h in handlers {
                result.push('\n');
                result.push_str(&make_indent(indent + 2));
                if let Some(tag) = h.tag {
                    if let Some(ref_var) = h.exnref_var.as_ref() {
                        result.push_str(&format!(
                            "(wasm-catch-ref {tag} ({}) {ref_var}\n",
                            h.vars.join(" ")
                        ));
                    } else {
                        result.push_str(&format!("(wasm-catch {tag} ({})\n", h.vars.join(" ")));
                    }
                    result.push_str(&make_indent(indent + 4));
                    result.push_str(&convert_expr(&h.body, indent + 4));
                    result.push(')');
                } else {
                    if let Some(ref_var) = h.exnref_var.as_ref() {
                        result.push_str(&format!("(wasm-catch-all-ref {ref_var}\n"));
                    } else {
                        result.push_str("(wasm-catch-all\n");
                    }
                    result.push_str(&make_indent(indent + 4));
                    result.push_str(&convert_expr(&h.body, indent + 4));
                    result.push(')');
                }
            }
            result.push(')');
            result
        }
        Throw(tag, payload) => {
            let mut result = String::new();
            result.push_str(&format!("(wasm-throw-exception {tag}"));
            let mut indent = indent;
            indent += result.len();
            for e in payload {
                indent += 1;
                result.push(' ');
                let s = convert_expr(e, indent);
                indent += s.len();
                result.push_str(&s);
            }
            result.push(')');
            result
        }
        Rethrow(exn_var) => format!(
            "(error 'wasm-exception :tag (wasm-exception-tag {exn_var}) \
             :payload (wasm-exception-payload {exn_var}))"
        ),
        ThrowRef(exnref) => format!("(error {})", convert_expr(exnref, indent + 2)),
        Values(exprs) => {
            let mut result = String::new();
            result.push_str("(values");
            for e in exprs {
                result.push(' ');
                result.push_str(&convert_expr(e, indent + 8));
            }
            result.push(')');
            result
        }
        SetfValues { locals, value } => {
            let mut result = String::new();
            result.push_str("(setf (values ");
            result.push_str(&locals.join(" "));
            result.push_str(")\n");
            result.push_str(&make_indent(indent + 2));
            result.push_str(&convert_expr(value, indent + 2));
            result.push(')');
            result
        }
    }
}

fn convert_body_exprs(exprs: &[Expr], indent: usize) -> String {
    let mut result = String::new();

    for (i, e) in exprs.iter().enumerate() {
        if i != 0 {
            result.push('\n');
        }
        result.push_str(&make_indent(indent));
        result.push_str(&convert_expr(e, indent));
    }

    result
}

fn convert_parameters(params: &[Type]) -> String {
    let mut output = String::new();
    for (i, p) in params.iter().enumerate() {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(&format!("(param-{i} {})", convert_type(*p)));
    }
    output
}

fn convert_function(module: &Module, func: &Function) -> Result<String> {
    let Some(body) = func.body.as_ref() else {
        let (module, name) = func.name.as_ref().unwrap();
        return Ok(format!(
            "(define-wasm-import {} ({}) ({}) {} {})",
            func.name(),
            {
                let mut result = String::new();
                for ty in func.ty.params.iter() {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(convert_type(*ty));
                }
                result
            },
            {
                let mut result = String::new();
                for ty in func.ty.results.iter() {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(convert_type(*ty));
                }
                result
            },
            symbolicate(module),
            symbolicate(name)
        ));
    };

    let mut output = String::new();
    let mut all_locals = Vec::new();

    for (i, ty) in func.ty.params.iter().enumerate() {
        all_locals.push((format!("param-{i}"), *ty));
    }
    for (i, ty) in body.locals.iter().enumerate() {
        all_locals.push((format!("local-{}", i + func.ty.params.len()), *ty));
    }

    output.push_str(&format!(
        "(define-wasm-function {} ({}) ({})\n",
        func.name(),
        convert_parameters(&func.ty.params),
        {
            let mut result = String::new();
            for ty in func.ty.results.iter() {
                if !result.is_empty() {
                    result.push(' ');
                }
                result.push_str(convert_type(*ty));
            }
            result
        }
    ));
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

    /*if func.index == 1835 {
        let op_read = wasmparser::OperatorsReader::new(
            wasmparser::BinaryReader::new(&body.code_bytes, body.code_offset));
        for (i, op) in op_read.into_iter().enumerate() {
            let op = op?;
            println!("  {i}: {op:?}");
        }
    }*/

    let mut op_read = wasmparser::OperatorsReader::new(wasmparser::BinaryReader::new(
        &body.code_bytes,
        body.code_offset,
    ));

    let exprs = expressionify_function_body(module, func, &all_locals, &mut op_read)?;

    output.push_str(&convert_body_exprs(&exprs, 4));

    output.push_str("))");
    Ok(output)
}

pub fn emit_system(
    module: &Module,
    package_name: &str,
    path: &Path,
    functions_per_file: usize,
) -> Result<()> {
    use std::io::Write;

    let file = fs::File::create(path.join(format!("{package_name}.asd")))?;
    let mut out = BufWriter::new(file);

    writeln!(&mut out, "(defsystem :{package_name}")?;
    writeln!(&mut out, "  :depends-on (#:wasm2cl)")?;
    writeln!(&mut out, "  :serial t")?;
    writeln!(&mut out, "  :components ((:file \"main\")")?;

    for i in 0..(module.functions.len().div_ceil(functions_per_file)) {
        writeln!(&mut out, "               (:file \"functions-{i:04}\")")?;
    }
    writeln!(&mut out, "))")?;

    out.flush()?;
    Ok(())
}

pub fn emit_main(module: &Module, package: &str, path: &Path) -> Result<()> {
    use std::io::Write;

    let file = fs::File::create(path.join("main.lisp"))?;
    let mut out = BufWriter::new(file);

    writeln!(&mut out, "(defpackage :{package}")?;
    writeln!(&mut out, "  (:use :cl :wasm2cl)")?;
    write!(&mut out, "  (:export #:wasm2cl-create-context")?;
    for e in module.exports.iter() {
        writeln!(&mut out)?;
        write!(&mut out, "           #:{}", symbolicate(&e.name))?;
    }
    writeln!(&mut out, "))")?;
    writeln!(&mut out)?;

    writeln!(&mut out, "(in-package :{package})")?;
    writeln!(&mut out)?;

    for f in module.functions.iter() {
        write!(&mut out, "(declaim (ftype (function (wasm-context")?;
        for ty in f.ty.params.iter() {
            write!(&mut out, " {}", convert_type(*ty))?;
        }
        writeln!(
            &mut out,
            ") {}) {}))",
            if f.ty.results.is_empty() {
                "t"
            } else {
                convert_type(f.ty.results[0])
            },
            f.name()
        )?;
    }
    writeln!(&mut out)?;

    writeln!(&mut out, "(defun wasm2cl-create-context (personality)")?;
    writeln!(
        &mut out,
        "  (let ((memory (make-array {} :element-type '(unsigned-byte 8)))",
        module.memory_initial_size
    )?;
    writeln!(
        &mut out,
        "        (table (make-array {} :initial-element nil))",
        module.table_initial_size
    )?;
    write!(&mut out, "        (globals (vector")?;
    for global in module.globals.iter() {
        write!(&mut out, " {}", global.initializer)?;
    }
    writeln!(&mut out, ")))")?;
    for data in module.active_data.iter() {
        writeln!(&mut out, "    (replace memory #.(coerce '(")?;
        for (i, byte) in data.data.iter().enumerate() {
            if i != 0 && (i % 25) == 0 {
                writeln!(&mut out)?;
            }
            write!(&mut out, " {byte}")?;
        }
        writeln!(&mut out, ")")?;
        writeln!(
            &mut out,
            "                              '(simple-array (unsigned-byte 8) (*)))"
        )?;
        writeln!(&mut out, "             :start1 {})", data.address)?;
    }
    for elt in module.active_elements.iter() {
        for (i, val) in elt.data.iter().enumerate() {
            writeln!(
                &mut out,
                "    (setf (svref table {}) #'{})",
                elt.address + i,
                module.functions[*val].name()
            )?;
        }
    }
    writeln!(&mut out, "  (make-wasm-context :personality personality")?;
    if let Some(f) = module.start_fn {
        writeln!(
            &mut out,
            "                     :start-fn #'{}",
            module.functions[f].name()
        )?;
    }
    writeln!(&mut out, "                     :memory memory")?;
    writeln!(&mut out, "                     :globals globals")?;
    writeln!(&mut out, "                     :table table)))")?;
    writeln!(&mut out)?;

    for e in module.exports.iter() {
        let func = &module.functions[e.func_idx];
        writeln!(
            &mut out,
            "(define-wasm-export {} ({}) ({}) {})",
            func.name(),
            {
                let mut result = String::new();
                for ty in func.ty.params.iter() {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(convert_type(*ty));
                }
                result
            },
            {
                let mut result = String::new();
                for ty in func.ty.results.iter() {
                    if !result.is_empty() {
                        result.push(' ');
                    }
                    result.push_str(convert_type(*ty));
                }
                result
            },
            symbolicate(&e.name)
        )?;
    }

    out.flush()?;
    Ok(())
}

pub fn emit_functions(
    module: &Module,
    package: &str,
    path: &Path,
    functions_per_file: usize,
) -> Result<()> {
    use std::io::Write;

    for (i, fns) in module.functions.chunks(functions_per_file).enumerate() {
        let file = fs::File::create(path.join(format!("functions-{i:04}.lisp")))?;
        let mut out = BufWriter::new(file);

        writeln!(&mut out, "(in-package :{package})")?;
        writeln!(&mut out)?;

        for f in fns {
            writeln!(&mut out, "{}", convert_function(module, f)?)?;
            writeln!(&mut out)?;
        }
        out.flush()?;
    }

    Ok(())
}
