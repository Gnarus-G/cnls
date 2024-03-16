mod classnames;
mod source;

use std::os::unix::fs::FileExt;

use anyhow::Context;
use classnames::ClassNamesCollector;
use cnls::fs;
use source::parse_classname_on_cursor;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Debug)]
struct Backend {
    client: Client,
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
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
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

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let path = params
            .text_document_position_params
            .text_document
            .uri
            .path();

        let current_filepath = std::path::Path::new(path);

        eprintln!(
            "[DEBUG] current source code: {}",
            current_filepath.display()
        );

        let classname_on_cursor = match parse_classname_on_cursor(
            current_filepath,
            params.text_document_position_params.position,
        ) {
            Ok(Some(cn)) => cn,
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

                        let result = std::fs::File::open(&css_file)
                            .context("failed to open css source file")
                            .with_context(|| {
                                format!("failed to open css source file: {}", css_file.display())
                            })
                            .and_then(|file| {
                                let rule_start_pos = class.span.lo.0 - 1; // swc's BytePos is
                                                                          // 1-based
                                let byte_read_count = class.span.hi.0 - class.span.lo.0;
                                let mut buf = vec![0; byte_read_count as usize];
                                file.read_exact_at(&mut buf, rule_start_pos.into())
                                    .with_context(|| {
                                        format!("failed to read file in the span: {:?}", class.span)
                                    })?;
                                let s = String::from_utf8(buf)
                                    .context("failed to read utf-8 string")?;
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
                            contents: HoverContents::Scalar(MarkedString::LanguageString(
                                LanguageString {
                                    language: "css".to_string(),
                                    value: source_rule,
                                },
                            )),
                            range: None,
                        }));
                    };
                }
            }
        }

        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend { client });

    Server::new(stdin, stdout, socket).serve(service).await;
}
