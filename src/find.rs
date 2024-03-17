use anyhow::anyhow;
use cnls::scope::{Scope, ScopeVariant};
use std::path::Path;
use swc_common::sync::Lrc;
use swc_common::{
    errors::{ColorConfig, Handler},
    SourceMap,
};
use swc_common::{BytePos, FileName, SourceFile};
use swc_ecma_ast::{Callee, EsVersion, Expr, Ident, JSXAttrName, PropName};
use swc_ecma_parser::{parse_file_as_module, Syntax};
use swc_ecma_visit::{Visit, VisitWith};
use tracing::{debug, info};

struct StringsWithClassNamesFinder<'scopes> {
    cursor_position: BytePos,
    scopes: &'scopes [Scope],
    is_in_scope: bool,
    found_classname_on_cursor: Option<String>,
}

impl<'scopes> StringsWithClassNamesFinder<'scopes> {
    fn new(scopes: &'scopes [Scope], cursor_position: BytePos) -> Self {
        Self {
            cursor_position,
            scopes,
            is_in_scope: false,
            found_classname_on_cursor: None,
        }
    }

    fn starts_a_valid_scope(&self, ident: &Ident, variant: ScopeVariant) -> bool {
        let ident = ident.sym.as_str();
        self.scopes
            .iter()
            .any(|scope| scope.matches(ident, variant))
    }
}

impl<'scopes> Visit for StringsWithClassNamesFinder<'scopes> {
    fn visit_jsx_attr(&mut self, n: &swc_ecma_ast::JSXAttr) {
        if let JSXAttrName::Ident(name) = &n.name {
            if self.starts_a_valid_scope(name, ScopeVariant::AttrNames) {
                self.is_in_scope = true;
                n.value.visit_with(self);
                self.is_in_scope = false;
            }
        }

        if self.found_classname_on_cursor.is_none() {
            n.visit_children_with(self);
        }
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

        if self.found_classname_on_cursor.is_none() {
            n.visit_children_with(self);
        }
    }

    fn visit_key_value_prop(&mut self, n: &swc_ecma_ast::KeyValueProp) {
        if let PropName::Ident(ident) = &n.key {
            if self.starts_a_valid_scope(ident, ScopeVariant::RecordEntries) {
                self.is_in_scope = true;
                n.value.visit_with(self);
                self.is_in_scope = false;
            }
        }

        if self.found_classname_on_cursor.is_none() {
            n.visit_children_with(self);
        }
    }

    fn visit_str(&mut self, n: &swc_ecma_ast::Str) {
        if !self.is_in_scope {
            return;
        }

        if self.found_classname_on_cursor.is_none() {
            self.found_classname_on_cursor = find_class_name_in_str(n, self.cursor_position)
        }
    }
}

pub struct SrcCodeMeta {
    path: std::path::PathBuf,
    cursor_byte_position: BytePos,
    file: Lrc<SourceFile>,
    source_map: Lrc<SourceMap>,
}

impl SrcCodeMeta {
    pub fn build(
        path: std::path::PathBuf,
        code: String,
        curr_cursor_position: tower_lsp::lsp_types::Position,
    ) -> anyhow::Result<Self> {
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(FileName::Real(path.clone()), code);

        let (start_pos, _) = fm.line_bounds(curr_cursor_position.line as usize);

        info!(
            "current line {} found to start on byte {}",
            curr_cursor_position.line, start_pos.0
        );

        let cursor_position = BytePos(start_pos.0 + curr_cursor_position.character);

        debug!("resolved cursor byte pos to {}", cursor_position.0);

        Ok(Self {
            path,
            cursor_byte_position: cursor_position,
            file: fm,
            source_map: cm,
        })
    }

    pub fn get_classname_on_cursor(self, scopes: &[Scope]) -> anyhow::Result<Option<String>> {
        let path = self.path;
        let error_handler =
            Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(self.source_map));

        let mut errors = vec![];

        let module = parse_file_as_module(
            &self.file,
            get_syntax_of_file(&path)?,
            EsVersion::latest(),
            None,
            &mut errors,
        )
        .map_err(|e| e.into_diagnostic(&error_handler).emit())
        .expect("failed to parser module");

        info!("parsed source code");

        let mut finder = StringsWithClassNamesFinder::new(scopes, self.cursor_byte_position);

        finder.visit_module(&module);

        Ok(finder.found_classname_on_cursor)
    }
}

fn find_class_name_in_str(s: &swc_ecma_ast::Str, cursor_position: BytePos) -> Option<String> {
    if s.is_empty() {
        return None;
    }

    let start_of_str = s.span.lo.0 + 1; // not counting the quote;
    let contains_cursor = s.span.lo < cursor_position && cursor_position < s.span.hi;
    if !contains_cursor {
        return None;
    }

    info!(
        "found class_name strings around current cursor: {:?} at bytepos {}",
        s.value, start_of_str
    );

    let mut substrings = vec![]; // inclusive incluse ranges for slices of the ast::Str that are
                                 // substrings
    let mut start = None;

    for (offset, b) in s.value.as_bytes().iter().enumerate() {
        if b.is_ascii_whitespace() {
            if start.is_some() && offset > 0 {
                substrings.push((start.unwrap(), offset - 1)); // -1 to keep end inclusive
                start = None;
            }
        } else {
            start.is_none().then(|| start = Some(offset));
        }
    }

    if let Some(start) = start {
        substrings.push((start, s.value.len() - 1));
    }

    let class_name = substrings.into_iter().find_map(|(start, end)| {
        let b_byte_start_pos = start_of_str + start as u32;
        let b_byte_end_pos = start_of_str + end as u32;

        if b_byte_start_pos <= cursor_position.0 && cursor_position.0 <= b_byte_end_pos {
            let value = &s.value[start..=end];
            info!("resolved substring on current cursor: {:?}", value);
            return Some(value.to_string());
        }

        None
    });

    return class_name;
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

#[cfg(test)]
mod tests {
    use super::find_class_name_in_str;
    use swc_common::{BytePos, SyntaxContext};
    use swc_ecma_ast::Str;

    /// `offset` marks the byte position of the opening quote of the string `value`.
    fn mock_str(value: &str, offset: u32) -> Str {
        let raw = format!("{value:?}");
        return Str {
            span: swc_common::Span {
                lo: BytePos(offset),
                hi: BytePos(raw.len() as u32 + offset),
                ctxt: SyntaxContext::default(),
            },
            value: value.into(),
            raw: Some(raw.into()),
        };
    }

    #[test]
    fn it_finds_name_at_position() {
        let s = mock_str("h-10 w-10 test", 2);
        assert_eq!(
            find_class_name_in_str(&s, BytePos(13)),
            Some("test".to_owned())
        );

        let s = mock_str("h-10 w-10 test", 2);
        assert_eq!(
            find_class_name_in_str(&s, BytePos(8)),
            Some("w-10".to_owned())
        );

        let s = mock_str(" h-10 w-10 test", 4);
        assert_eq!(
            find_class_name_in_str(&s, BytePos(6)),
            Some("h-10".to_owned())
        );

        let s = mock_str(
            r#" h-10 w-10 
test"#,
            8,
        );
        assert_eq!(
            find_class_name_in_str(&s, BytePos(21)),
            Some("test".to_owned())
        );

        let s = mock_str("short small   tall", 16);
        assert_eq!(
            find_class_name_in_str(&s, BytePos(21)),
            Some("short".to_owned())
        );
    }

    #[test]
    fn it_finds_nothing_at_whitespaces() {
        let s = mock_str("h-10 w-10 test", 1);
        assert_eq!(find_class_name_in_str(&s, BytePos(11)), None);

        let s = mock_str("h-10 w-10 ", 1);
        assert_eq!(find_class_name_in_str(&s, BytePos(11)), None);

        let s = mock_str("h-10 w-10   ", 1);
        assert_eq!(find_class_name_in_str(&s, BytePos(11)), None);

        let s = mock_str(" h-10 w-10 test", 3);
        assert_eq!(find_class_name_in_str(&s, BytePos(4)), None);

        let s = mock_str(
            r#" h-10 w-10 
                test"#,
            7,
        );
        assert_eq!(find_class_name_in_str(&s, BytePos(19)), None);
    }

    #[test]
    fn it_finds_nothing_at_empty_str() {
        let s = mock_str("", 1);
        assert_eq!(find_class_name_in_str(&s, BytePos(2)), None);

        let s = mock_str("     ", 1);
        assert_eq!(find_class_name_in_str(&s, BytePos(2)), None);
    }
}
