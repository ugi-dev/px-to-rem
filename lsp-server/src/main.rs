use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tower_lsp::{
    jsonrpc::Result,
    lsp_types::*,
    Client, LanguageServer, LspService, Server,
};

// ---------------------------------------------------------------------------
// Conversion math — mirrors extension.js exactly
//
// Original JS:
//   px2Rem: parseFloat((px / pxPerRem).toFixed(maxDecimals))
//   rem2Px: parseFloat((rem * pxPerRem).toFixed(maxDecimals))
//
// parseFloat(n.toFixed(k)) removes trailing zeros, e.g. "1.0000" → 1.
// We replicate that by trimming trailing zeros from the formatted string.
// ---------------------------------------------------------------------------

fn format_number(value: f64, decimal_places: u32) -> String {
    let s = format!("{:.prec$}", value, prec = decimal_places as usize);
    if s.contains('.') {
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    } else {
        s
    }
}

fn px_to_rem(px: f64, px_per_rem: f64, decimal_places: u32) -> String {
    if px_per_rem == 0.0 {
        return "0".to_string();
    }
    format_number(px / px_per_rem, decimal_places)
}

fn rem_to_px(rem: f64, px_per_rem: f64, decimal_places: u32) -> String {
    format_number(rem * px_per_rem, decimal_places)
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Config {
    px_per_rem: f64,
    decimal_places: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            px_per_rem: 16.0,
            decimal_places: 4,
        }
    }
}

impl Config {
    fn apply_json(&mut self, value: &serde_json::Value) {
        if let Some(v) = value.get("px_per_rem").and_then(|v| v.as_f64()) {
            if v > 0.0 {
                self.px_per_rem = v;
            }
        }
        if let Some(v) = value.get("decimal_places").and_then(|v| v.as_u64()) {
            self.decimal_places = v as u32;
        }
    }
}

// ---------------------------------------------------------------------------
// Document store (full-sync)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DocumentStore {
    docs: HashMap<Url, String>,
}

// ---------------------------------------------------------------------------
// Code action computation
// ---------------------------------------------------------------------------

/// Regex patterns (identical to extension.js)
fn build_patterns() -> (regex::Regex, regex::Regex) {
    let px_re = regex::Regex::new(r"(?i)([0-9]*\.?[0-9]+)px").unwrap();
    let rem_re = regex::Regex::new(r"(?i)([0-9]*\.?[0-9]+)rem").unwrap();
    (px_re, rem_re)
}

fn compute_code_actions(
    text: &str,
    uri: &Url,
    range: Range,
    config: &Config,
) -> Vec<CodeActionOrCommand> {
    let (px_re, rem_re) = build_patterns();
    let lines: Vec<&str> = text.lines().collect();
    let start_line = range.start.line as usize;
    let end_line = (range.end.line as usize).min(lines.len().saturating_sub(1));

    let mut actions: Vec<CodeActionOrCommand> = Vec::new();

    // Per-occurrence actions + collect all px matches for a batch action
    let mut all_px_edits: Vec<TextEdit> = Vec::new();
    let mut all_rem_edits: Vec<TextEdit> = Vec::new();

    for line_idx in start_line..=end_line {
        let Some(line) = lines.get(line_idx) else {
            continue;
        };

        // Skip characters outside the selection on the first/last line
        let col_start = if line_idx == start_line {
            range.start.character as usize
        } else {
            0
        };
        let col_end = if line_idx == end_line {
            (range.end.character as usize).min(line.len())
        } else {
            line.len()
        };
        // Expand col_start/col_end to catch a value the cursor sits inside
        let scan_start = col_start.saturating_sub(20).min(col_start);
        let scan_end = (col_end + 20).min(line.len());
        let scan_slice = &line[scan_start..scan_end];

        for cap in px_re.captures_iter(scan_slice) {
            let full = cap.get(0).unwrap();
            let value: f64 = cap[1].parse().unwrap_or(0.0);
            let converted = px_to_rem(value, config.px_per_rem, config.decimal_places);
            let new_text = format!("{converted}rem");

            let char_start = (scan_start + full.start()) as u32;
            let char_end = (scan_start + full.end()) as u32;
            let edit_range = Range {
                start: Position { line: line_idx as u32, character: char_start },
                end: Position { line: line_idx as u32, character: char_end },
            };

            let edit = TextEdit { range: edit_range, new_text: new_text.clone() };
            all_px_edits.push(edit.clone());

            actions.push(make_action(
                format!("Convert {} → {}", full.as_str(), new_text),
                uri,
                vec![edit],
            ));
        }

        for cap in rem_re.captures_iter(scan_slice) {
            let full = cap.get(0).unwrap();
            let value: f64 = cap[1].parse().unwrap_or(0.0);
            let converted = rem_to_px(value, config.px_per_rem, config.decimal_places);
            let new_text = format!("{converted}px");

            let char_start = (scan_start + full.start()) as u32;
            let char_end = (scan_start + full.end()) as u32;
            let edit_range = Range {
                start: Position { line: line_idx as u32, character: char_start },
                end: Position { line: line_idx as u32, character: char_end },
            };

            let edit = TextEdit { range: edit_range, new_text: new_text.clone() };
            all_rem_edits.push(edit.clone());

            actions.push(make_action(
                format!("Convert {} → {}", full.as_str(), new_text),
                uri,
                vec![edit],
            ));
        }
    }

    // Batch "Convert ALL" actions (only shown when there are multiple matches)
    if all_px_edits.len() > 1 {
        actions.push(make_action(
            format!("Convert ALL {} px values → rem in selection", all_px_edits.len()),
            uri,
            all_px_edits,
        ));
    }
    if all_rem_edits.len() > 1 {
        actions.push(make_action(
            format!("Convert ALL {} rem values → px in selection", all_rem_edits.len()),
            uri,
            all_rem_edits,
        ));
    }

    actions
}

fn make_action(title: String, uri: &Url, edits: Vec<TextEdit>) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::REFACTOR),
        edit: Some(WorkspaceEdit {
            changes: Some({
                let mut m = HashMap::new();
                m.insert(uri.clone(), edits);
                m
            }),
            ..Default::default()
        }),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// LSP Backend
// ---------------------------------------------------------------------------

struct Backend {
    client: Client,
    config: Arc<RwLock<Config>>,
    documents: Arc<RwLock<DocumentStore>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(opts) = params.initialization_options {
            self.config.write().await.apply_json(&opts);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::REFACTOR]),
                        resolve_provider: Some(false),
                        work_done_progress_options: Default::default(),
                    },
                )),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "px-to-rem-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "px-to-rem-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    // --- Document sync ---

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.documents
            .write()
            .await
            .docs
            .insert(params.text_document.uri, params.text_document.text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .write()
                .await
                .docs
                .insert(params.text_document.uri, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .docs
            .remove(&params.text_document.uri);
    }

    // --- Code actions ---

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let docs = self.documents.read().await;
        let Some(text) = docs.docs.get(&params.text_document.uri) else {
            return Ok(None);
        };
        let config = self.config.read().await;
        let actions = compute_code_actions(text, &params.text_document.uri, params.range, &config);

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    // --- Live configuration updates ---
    // Users can set "settings" (not "initialization_options") for live reload:
    //   "lsp": { "px-to-rem-lsp": { "settings": { "px_per_rem": 10 } } }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.config.write().await.apply_json(&params.settings);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let config = Arc::new(RwLock::new(Config::default()));
    let documents = Arc::new(RwLock::new(DocumentStore::default()));

    let (service, socket) = LspService::new(|client| Backend {
        client,
        config,
        documents,
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
