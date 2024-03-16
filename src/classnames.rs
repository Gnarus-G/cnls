use std::path::PathBuf;

use swc_common::errors::{ColorConfig, Handler};
use swc_common::sync::Lrc;
use swc_common::{FileName, SourceMap, Span};
use swc_css::visit::{Visit, VisitWith};

use swc_css::{ast::Rule, parser::parse_file};

pub struct ClassName {
    pub value: cnls::Str,
    pub span: Span,
}

pub struct ClassNamesCollector {
    class_names: Vec<ClassName>,
    last_rule_span: Option<Span>,
}

impl ClassNamesCollector {
    pub fn new() -> Self {
        ClassNamesCollector {
            last_rule_span: None,
            class_names: vec![],
        }
    }

    pub fn find_class_name_by_value(&self, value: &str) -> Option<&ClassName> {
        self.class_names.iter().find(|c| &c.value == value)
    }

    pub fn parse(css_file: PathBuf) -> anyhow::Result<Self> {
        let code = std::fs::read_to_string(&css_file)?;

        let options = swc_css::parser::parser::ParserConfig::default();

        let cm: Lrc<SourceMap> = Default::default();
        let filename = FileName::Real(css_file);
        let cssfile = cm.new_source_file(filename.clone(), code);

        let handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));

        let mut errors = vec![];
        let c = parse_file::<Vec<Rule>>(&cssfile, None, options, &mut errors).unwrap();

        for e in errors {
            e.to_diagnostics(&handler).emit();
        }

        let mut ccns = ClassNamesCollector::new();

        c.visit_with(&mut ccns);

        Ok(ccns)
    }
}

impl Visit for ClassNamesCollector {
    fn visit_qualified_rule(&mut self, n: &swc_css::ast::QualifiedRule) {
        self.last_rule_span = Some(n.span);
        n.visit_children_with(self)
    }

    fn visit_compound_selector(&mut self, n: &swc_css::ast::CompoundSelector) {
        let selectors = &n.subclass_selectors;

        selectors
                .iter()
                .filter_map(|s| match s {
                    swc_css::ast::SubclassSelector::Class(selector) => Some(selector),
                    _ => None,
                })
                .for_each(|s| {
                    if s.text.value.contains(':') {
                        let cn = s.text.value.split(':').last().expect("should have at least one value after split, since empty selectors aren't allowed");

                        self.class_names.push(ClassName {
                            value: cn.into(),
                            span: self.last_rule_span.unwrap_or_default()
                        });
                    } else {
                        self.class_names.push(ClassName { value: s.text.value.as_str().into(), span: self.last_rule_span.unwrap_or_default()});
                    }
                });
    }
}
