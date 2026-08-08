//! Parses a wasm binary into a `Module`.

use anyhow::{Context, Result, bail};
use wasmparser::Parser;

use crate::symbolicate;

const WASM_PAGE_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Type {
    I32,
    I64,
    F32,
    F64,
    V128,
    FuncRef,
    ExternRef,
    ExnRef,
}

#[derive(Debug, Clone)]
pub struct FuncType {
    pub params: Vec<Type>,
    pub results: Vec<Type>,
}

#[derive(Debug)]
pub struct Function {
    pub index: usize,
    pub ty: FuncType,
    pub name: Option<(String, String)>,
    pub body: Option<Body>,
    pub internal_name: Option<String>,
}

impl Function {
    pub fn name(&self) -> String {
        if let Some(name) = self.internal_name.as_ref() {
            format!("wasm-{}-{}", self.index, symbolicate(name))
        } else if let Some((module, name)) = self.name.as_ref() {
            // Import
            format!(
                "wasm-import-{}-{}-{}",
                symbolicate(module),
                symbolicate(name),
                self.index
            )
        } else {
            // Internal function
            format!("wasm-function-{}", self.index)
        }
    }
}

#[derive(Debug)]
pub struct Body {
    pub locals: Vec<Type>,
    pub code_bytes: Vec<u8>,
    pub code_offset: usize,
}

#[derive(Debug)]
pub struct Export {
    pub name: String,
    pub func_idx: usize,
}

#[derive(Debug)]
pub struct ActiveData {
    pub address: usize,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct Global {
    pub initializer: String,
}

#[derive(Debug)]
pub struct ActiveElement {
    pub address: usize,
    pub data: Vec<usize>, // function indices
}

#[derive(Debug)]
pub struct Module {
    pub memory_initial_size: usize,
    pub table_initial_size: usize,
    pub types: Vec<FuncType>,
    pub tags: Vec<FuncType>,
    pub functions: Vec<Function>,
    pub exports: Vec<Export>,
    pub active_data: Vec<ActiveData>,
    pub active_elements: Vec<ActiveElement>,
    pub globals: Vec<Global>,
    pub start_fn: Option<usize>,
}

fn is_exn_ref(r: &wasmparser::RefType) -> bool {
    matches!(
        r.heap_type(),
        wasmparser::HeapType::Abstract {
            shared: false,
            ty: wasmparser::AbstractHeapType::Exn
        }
    )
}

fn parse_type(ty: &wasmparser::ValType) -> Result<Type> {
    Ok(match ty {
        wasmparser::ValType::I32 => Type::I32,
        wasmparser::ValType::I64 => Type::I64,
        wasmparser::ValType::F32 => Type::F32,
        wasmparser::ValType::F64 => Type::F64,
        wasmparser::ValType::V128 => Type::V128,
        wasmparser::ValType::Ref(r) if r.is_func_ref() => Type::FuncRef,
        wasmparser::ValType::Ref(r) if r.is_extern_ref() => Type::ExternRef,
        wasmparser::ValType::Ref(r) if is_exn_ref(r) => Type::ExnRef,
        other => bail!("unsupported valtype: {other:?}"),
    })
}

pub fn parse(bytes: &[u8]) -> Result<Module> {
    use wasmparser::Payload::*;

    let mut types = vec![];
    let mut tags = vec![];
    let mut functions = vec![];
    let mut current_function = 0;
    let mut exports = vec![];
    let mut active_data = vec![];
    let mut active_elements = vec![];
    let mut memory_initial_size = 0;
    let mut globals = vec![];
    let mut table_initial_size = 0;
    let mut start_fn = None;

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
                                params: ft
                                    .params()
                                    .iter()
                                    .map(parse_type)
                                    .collect::<Result<_>>()?,
                                results: ft
                                    .results()
                                    .iter()
                                    .map(parse_type)
                                    .collect::<Result<_>>()?,
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
                        wasmparser::Imports::Single(
                            _,
                            wasmparser::Import {
                                module,
                                name,
                                ty: wasmparser::TypeRef::Func(ty),
                            },
                        ) => {
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
                    if mem.memory64 {
                        bail!("unsupported: memory64 (only 32-bit memories supported)");
                    }
                    if mem.shared {
                        bail!("unsupported: shared memory");
                    }
                    if mem.page_size_log2.is_some() {
                        bail!("unsupported: custom memory page size");
                    }
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
                    if let wasmparser::ElementKind::Active {
                        table_index,
                        offset_expr,
                    } = seg.kind
                    {
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
                start_fn = Some(func as usize);
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
                let code_bytes = ops_bin_reader
                    .read_bytes(ops_bin_reader.bytes_remaining())?
                    .to_vec();

                functions[current_function].body = Some(Body {
                    locals,
                    code_bytes,
                    code_offset,
                });
                current_function += 1;
            }
            TagSection(reader) => {
                println!("TagSection");
                for tag in reader {
                    let tag = tag?;
                    println!(" {tag:?}");
                    tags.push(types[tag.func_type_idx as usize].clone());
                }
            }
            DataSection(reader) => {
                println!("DataSection");
                for seg in reader {
                    let seg = seg?;
                    if let wasmparser::DataKind::Active {
                        memory_index,
                        offset_expr,
                    } = seg.kind
                    {
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
                        active_data.push(ActiveData {
                            address,
                            data: seg.data.to_owned(),
                        });
                    } else {
                        bail!("Unsupported data segment kind {:?}", seg.kind);
                    }
                    //println!(" {seg:?}");
                }
            }
            End(_) => break,
            CustomSection(r) => match r.as_known() {
                wasmparser::KnownCustom::Name(reader) => {
                    for subsection in reader {
                        match subsection? {
                            wasmparser::Name::Function(map) => {
                                println!("FunctionNameSection");
                                for naming in map {
                                    let naming = naming?;
                                    functions[naming.index as usize].internal_name =
                                        Some(naming.name.to_string());
                                }
                            }
                            _ => println!("Unknown name section"),
                        }
                    }
                }
                _ => println!("Unknown custom section {r:?}"),
            },
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
        tags,
        functions,
        exports,
        active_data,
        active_elements,
        globals,
        start_fn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::{RefType, ValType};

    #[test]
    fn parse_type_maps_all_supported_valtypes() {
        assert_eq!(parse_type(&ValType::I32).unwrap(), Type::I32);
        assert_eq!(parse_type(&ValType::I64).unwrap(), Type::I64);
        assert_eq!(parse_type(&ValType::F32).unwrap(), Type::F32);
        assert_eq!(parse_type(&ValType::F64).unwrap(), Type::F64);
        assert_eq!(parse_type(&ValType::V128).unwrap(), Type::V128);
        assert_eq!(parse_type(&ValType::FUNCREF).unwrap(), Type::FuncRef);
        assert_eq!(parse_type(&ValType::EXTERNREF).unwrap(), Type::ExternRef);
        assert_eq!(parse_type(&ValType::EXNREF).unwrap(), Type::ExnRef);
    }

    #[test]
    fn parse_type_rejects_unsupported_valtypes() {
        // `(ref noexn)` is the "no exception" ref type; only exn refs
        // are meaningful to us.
        let err = parse_type(&ValType::Ref(RefType::NOEXN)).unwrap_err();
        assert!(err.to_string().contains("unsupported valtype"), "{err}");
    }

    #[test]
    fn is_exn_ref_detects_exn_heap_types() {
        assert!(is_exn_ref(&RefType::EXNREF));
        assert!(is_exn_ref(&RefType::EXN));
        assert!(!is_exn_ref(&RefType::NOEXN));
        assert!(!is_exn_ref(&RefType::FUNCREF));
        assert!(!is_exn_ref(&RefType::EXTERNREF));
    }

    fn function(
        index: usize,
        name: Option<(String, String)>,
        internal_name: Option<String>,
    ) -> Function {
        Function {
            index,
            ty: FuncType {
                params: vec![],
                results: vec![],
            },
            name,
            body: None,
            internal_name,
        }
    }

    #[test]
    fn function_name_prefers_internal_name() {
        // The name section wins over import/export names.
        let f = function(
            7,
            Some(("env".into(), "proc_exit".into())),
            Some("foo".into()),
        );
        assert_eq!(f.name(), "wasm-7-|foo|");
    }

    #[test]
    fn function_name_import() {
        let f = function(
            3,
            Some(("wasi_snapshot_preview1".into(), "proc_exit".into())),
            None,
        );
        assert_eq!(
            f.name(),
            "wasm-import-|wasi_snapshot_preview1|-|proc_exit|-3"
        );
    }

    #[test]
    fn function_name_fallback() {
        let f = function(5, None, None);
        assert_eq!(f.name(), "wasm-function-5");
    }

    #[test]
    fn parse_empty_module() {
        let module = parse(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(module.types.len(), 0);
        assert_eq!(module.functions.len(), 0);
        assert_eq!(module.exports.len(), 0);
        assert_eq!(module.start_fn, None);
    }

    #[test]
    fn parse_parses_all_supported_sections() {
        let bytes = wat::parse_str(
            r#"
            (module
              (type $t0 (func (param i32) (result i32)))
              (type $t1 (func (param i32)))
              (type $t2 (func))
              (import "env" "log" (func $log (type $t1)))
              (func $f (type $t0) (param i32) (result i32)
                local.get 0)
              (func $main (type $t2)
                (call $log (call $f (i32.const 1))))
              (memory 1)
              (global $g (mut i32) (i32.const 3))
              (export "main" (func $main))
              (data (i32.const 8) "hello")
              (start $main)
            )
            "#,
        )
        .unwrap();
        let module = parse(&bytes).unwrap();

        assert_eq!(module.types.len(), 3);
        assert_eq!(module.functions.len(), 3);
        assert_eq!(module.tags.len(), 0);
        assert_eq!(module.memory_initial_size, 64 * 1024);
        assert_eq!(module.globals.len(), 1);
        assert_eq!(module.active_data.len(), 1);
        assert_eq!(module.active_data[0].address, 8);
        assert_eq!(module.active_data[0].data, b"hello");
        assert_eq!(module.start_fn, Some(2));
        // Only function exports are recorded.
        assert_eq!(module.exports.len(), 1);
        assert_eq!(module.exports[0].name, "main");
        assert_eq!(module.exports[0].func_idx, 2);

        // The import gets (module, name); defined funcs get bodies.
        assert_eq!(
            module.functions[0].name,
            Some(("env".to_string(), "log".to_string()))
        );
        assert!(module.functions[0].body.is_none());
        assert!(module.functions[1].body.is_some());
        assert!(module.functions[2].body.is_some());
    }
}
