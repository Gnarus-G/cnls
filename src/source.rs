use anyhow::{anyhow, Context};
use cnls::scope::{Scope, ScopeVariant};
use std::path::Path;
use swc_common::sync::Lrc;
use swc_common::BytePos;
use swc_common::{
    errors::{ColorConfig, Handler},
    SourceMap,
};
use swc_ecma_ast::{Callee, EsVersion, Expr, Ident, JSXAttrName, PropName};
use swc_ecma_parser::{parse_file_as_module, Syntax};
use swc_ecma_visit::{Visit, VisitWith};

pub fn parse_classname_on_cursor(
    path: &Path,
    position: tower_lsp::lsp_types::Position,
    scopes: &[Scope],
) -> anyhow::Result<Option<String>> {
    let cm: Lrc<SourceMap> = Default::default();
    let error_handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));

    let fm = cm.load_file(path).context("failed to load source file")?;

    let mut errors = vec![];

    let module = parse_file_as_module(
        &fm,
        get_syntax_of_file(path)?,
        EsVersion::latest(),
        None,
        &mut errors,
    )
    .map_err(|e| e.into_diagnostic(&error_handler).emit())
    .expect("failed to parser module");

    eprintln!("[INFO] parsed source code");

    let (start_pos, _) = fm.line_bounds(position.line as usize);

    eprintln!(
        "[INFO] current line {} found to start on byte {}",
        position.line, start_pos.0
    );

    let cursor_position = BytePos(start_pos.0 + position.character);

    eprintln!("[DEBUG] resolved cursor byte pos to {}", cursor_position.0);

    let mut finder = FindClassNames {
        cursor_position,
        scopes,
        is_in_scope: false,
        found_class_name_on_cursor: None,
    };

    finder.visit_module(&module);

    Ok(finder.found_class_name_on_cursor)
}

struct FindClassNames<'scopes> {
    cursor_position: BytePos,
    scopes: &'scopes [Scope],
    is_in_scope: bool,
    found_class_name_on_cursor: Option<String>,
}

impl<'scopes> FindClassNames<'scopes> {
    fn starts_a_valid_scope(&self, ident: &Ident, variant: ScopeVariant) -> bool {
        let ident = ident.sym.as_str();
        self.scopes
            .iter()
            .any(|scope| scope.matches(ident, variant))
    }
}

impl<'scopes> Visit for FindClassNames<'scopes> {
    fn visit_jsx_attr(&mut self, n: &swc_ecma_ast::JSXAttr) {
        if let JSXAttrName::Ident(name) = &n.name {
            if self.starts_a_valid_scope(name, ScopeVariant::AttrNames) {
                self.is_in_scope = true;
                n.value.visit_with(self);
                self.is_in_scope = false;
            }
        }

        n.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, n: &swc_ecma_ast::CallExpr) {
        if let Callee::Expr(expr) = &n.callee {
            if let Expr::Ident(name) = expr.as_ref() {
                if self.starts_a_valid_scope(name, ScopeVariant::FnCall) {
                    self.is_in_scope = true;
                    n.args.visit_with(self);
                    self.is_in_scope = false;
                }
            }
        }

        n.visit_children_with(self);
    }

    fn visit_key_value_prop(&mut self, n: &swc_ecma_ast::KeyValueProp) {
        if let PropName::Ident(ident) = &n.key {
            if self.starts_a_valid_scope(ident, ScopeVariant::RecordEntries) {
                self.is_in_scope = true;
                n.value.visit_with(self);
                self.is_in_scope = false;
            }
        }

        n.visit_children_with(self);
    }

    fn visit_str(&mut self, n: &swc_ecma_ast::Str) {
        if !self.is_in_scope {
            return;
        }

        let start_of_str = n.span.lo;
        let contains_cursor = n.span.lo < self.cursor_position && self.cursor_position < n.span.hi;
        if !contains_cursor {
            return;
        }

        eprintln!("[INFO] found string around current cursor: {:?}", n.value);

        let mut buf = String::new();
        let mut cursor_is_on_substring = false;

        for (offset, b) in n.value.as_bytes().iter().enumerate() {
            if b.is_ascii_whitespace() {
                if cursor_is_on_substring {
                    break;
                }
                buf.clear();
            } else {
                buf.push(*b as char);
                let b_byte_pos = start_of_str.0 + offset as u32;
                if b_byte_pos >= self.cursor_position.0 {
                    cursor_is_on_substring = true;
                }
            }
        }

        eprintln!("[INFO] resolved substring on current cursor: {:?}", buf);

        self.found_class_name_on_cursor = Some(buf);
    }
}

fn get_syntax_of_file(source_file: &Path) -> anyhow::Result<Syntax> {
    let syntax = match source_file.extension().and_then(|e| e.to_str()) {
        Some("js") | Some("jsx") => Syntax::Es(swc_ecma_parser::EsConfig {
            jsx: true,
            ..Default::default()
        }),
        Some("ts") => Syntax::Typescript(Default::default()),
        Some("tsx") => Syntax::Typescript(swc_ecma_parser::TsConfig {
            tsx: true,
            ..Default::default()
        }),
        None => {
            return Err(anyhow!(
                "unknown filetype, missing extension: {}",
                source_file.display()
            ))
        }
        ext => return Err(anyhow!("unknown filetype: {ext:?}")),
    };

    return Ok(syntax);
}
