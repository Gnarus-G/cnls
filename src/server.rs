use std::mem;
use std::ops::Deref;
use std::str::FromStr;

use crate::classnames::ClassNamesCollector;
use crate::source::SrcCodeMeta;
use anyhow::{anyhow, Context};
use cnls::fs;
use cnls::scope::Scope;
use dashmap::DashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{debug, error};

#[derive(Debug)]
struct Config {
    scopes: Vec<Scope>,
}

impl Default for Config {
    fn default() -> Self {
        let default_scopes =
            ["att:className,class", "fn:createElement"].map(|s| Scope::from_str(s).unwrap());
        Self {
            scopes: default_scopes.to_vec(),
        }
    }
}

#[derive(Debug)]
struct Backend {
    client: Client,
    config: tokio::sync::RwLock<Config>,
    documents: DashMap<Url, String>,
}

impl Backend {
    async fn workspace_uris(&self) -> Result<Option<Vec<Url>>> {
        let paths = self
            .client
            .workspace_folders()
            .await?
            .map(|folders| folders.into_iter().map(|f| f.uri).collect::<Vec<_>>());

        Ok(paths)
    }

    async fn find_class_name_on_cursor_at(
        &self,
        uri: &Url,
        position: tower_lsp::lsp_types::Position,
    ) -> Result<Option<(std::path::PathBuf, swc_common::Span)>> {
        let current_doc = self
            .documents
            .get(uri)
            .expect("failed to get document by uri");
        let code = current_doc.deref();
        let scopes = &self.config.read().await.scopes;
        let path = std::path::PathBuf::from(uri.path());

        let src = match SrcCodeMeta::build(path, code.to_owned(), position) {
            Ok(s) => s,
            Err(err) => {
                error!("{err:#}");
                return Ok(None);
            }
        };

        let classname_on_cursor = match src.get_classname_on_cursor(scopes) {
            Ok(Some(strs)) => strs,
            Ok(None) => return Ok(None),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, format!("{err:#}"))
                    .await;
                return Ok(None);
            }
        };

        let Ok(Some(uris)) = self.workspace_uris().await else {
            self.client
                .log_message(MessageType::ERROR, "must define the root_path for cnls")
                .await;

            return Ok(None);
        };

        let mut css_files = vec![];

        let root = uris[0].path();
        if let Err(err) = fs::find_all_css_files_in_dir(root, &mut css_files) {
            self.client
                .log_message(MessageType::ERROR, format!("{err:#}"))
                .await
        };

        let parsed = css_files
            .into_iter()
            .map(|file| (file.clone(), ClassNamesCollector::parse(file)))
            .collect::<Vec<_>>();

        for p in parsed {
            let (css_file, p) = p;

            match p {
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, format!("{err:#}"))
                        .await
                }
                Ok(collector) => {
                    if let Some(class) = collector.find_class_name_by_value(&classname_on_cursor) {
                        self.client
                            .log_message(
                                MessageType::INFO,
                                format!(
                                    "found class rule {classname_on_cursor:?} in css file {}",
                                    css_file.display()
                                ),
                            )
                            .await;

                        return Ok(Some((css_file, class.span)));
                    };
                }
            }
        }

        Ok(None)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        debug!("current source code: {}", params.text_document.uri.path());

        let code = params.text_document.text;

        self.documents.insert(params.text_document.uri, code);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.remove(&params.text_document.uri);
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        debug!("current source code: {}", uri.path());

        let code = mem::take(&mut params.content_changes[0].text);
        self.documents.insert(uri, code);
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let parsed_scopes_from_config = params.settings["cnls"]["scopes"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(cnls::scope::Scope::from_str)
                .collect::<Vec<_>>()
        });

        match parsed_scopes_from_config {
            Some(results) => {
                let mut config = self.config.write().await;

                config.scopes.clear();

                for r in results {
                    match r {
                        Ok(scope) => config.scopes.push(scope),
                        Err(err) => {
                            self.client
                                .log_message(MessageType::ERROR, format!("cnls.scopes: {err:#}"))
                                .await
                        }
                    }
                }
            }
            None => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        "cnls.scopes should be an array of strings",
                    )
                    .await;
            }
        };
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let current_position = params.text_document_position_params.position;

        if let Some((css_file, span)) = self
            .find_class_name_on_cursor_at(&uri, current_position)
            .await?
        {
            let result = std::fs::File::open(&css_file)
                .context("failed to open css source file")
                .with_context(|| format!("failed to open css source file: {}", css_file.display()))
                .and_then(|file| {
                    let rule_start_pos = span.lo.0 - 1; // swc's BytePos is
                                                        // 1-based
                    let byte_read_count = span.hi.0 - span.lo.0;
                    let mut buf = vec![0; byte_read_count as usize];
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::FileExt;
                        file.read_exact_at(&mut buf, rule_start_pos.into())
                            .with_context(|| {
                                format!("failed to read file in the span: {:?}", span)
                            })?;
                    }

                    #[cfg(not(unix))]
                    {
                        use std::os::windows::fs::FileExt;
                        file.seek_read(&mut buf, rule_start_pos.into())
                            .with_context(|| {
                                format!("failed to read file in the span: {:?}", span)
                            })?;
                    }

                    let s = String::from_utf8(buf).context("failed to read utf-8 string")?;
                    Ok(s)
                });

            let source_rule = match result {
                Ok(s) => s,
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, format!("{err:#}",))
                        .await;

                    return Ok(None);
                }
            };

            return Ok(Some(Hover {
                contents: HoverContents::Scalar(MarkedString::LanguageString(LanguageString {
                    language: "css".to_string(),
                    value: source_rule,
                })),
                range: None,
            }));
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let current_position = params.text_document_position_params.position;

        if let Some((css_file, span)) = self
            .find_class_name_on_cursor_at(&uri, current_position)
            .await?
        {
            fn get_location(
                css_file: std::path::PathBuf,
                span: swc_common::Span,
            ) -> anyhow::Result<Location> {
                let uri = Url::from_file_path(&css_file).map_err(|_| {
                    anyhow!(
                        "failed to get uri from css file path: {}",
                        css_file.display()
                    )
                })?;

                let (cssfile, _) = crate::classnames::css_source_file_from(css_file)
                    .context("failed to build a SourceFile from a css file")?;

                let start_ln_num = cssfile.lookup_line(span.lo).ok_or(anyhow!(
                    "failed to get line number of the span start: {:?}",
                    span
                ))?;
                let end_ln_num = cssfile.lookup_line(span.hi).ok_or(anyhow!(
                    "failed to get line number of the span end: {:?}",
                    span
                ))?;
                let range = Range::new(
                    Position {
                        line: start_ln_num as u32,
                        character: (span.lo - cssfile.line_begin_pos(span.lo)).0,
                    },
                    Position {
                        line: end_ln_num as u32,
                        character: (span.hi - cssfile.line_begin_pos(span.hi)).0,
                    },
                );

                Ok(Location::new(uri, range))
            }

            let location = match get_location(css_file, span) {
                Ok(l) => l,
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, format!("{err:#}"))
                        .await;

                    return Ok(None);
                }
            };

            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
        }

        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub async fn start() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        config: tokio::sync::RwLock::new(Config::default()),
        documents: DashMap::new(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
