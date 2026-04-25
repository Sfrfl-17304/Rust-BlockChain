use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block as UiBlock, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use std::{
    io::{self, BufRead, BufReader, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct TxIn {
    txid: String,
    vout: usize,
    signature: String,
    pubkey: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct TxOut {
    pubkey_hash: String,
    amount: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct Transaction {
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    timestamp: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct Block {
    index: u32,
    timestamp: u64,
    transactions: Vec<Transaction>,
    previous_hash: String,
    merkle_root: String,
    difficulty: u32,
    hash: String,
    nonce: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct NodeState {
    chain: String,
    height: usize,
    tip: String,
    mempool: Vec<Transaction>,
    utxo_count: usize,
    difficulty: u32,
}

struct AppState {
    selected: usize,
    status: String,
    blocks: Vec<Block>,
    node: Option<NodeState>,
    list_state: ListState,
}

fn request_state(addr: &str) -> Result<NodeState, String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    stream
        .write_all(b"REQ_STATE\n")
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf).map_err(|e| e.to_string())?;
    let msg = buf.trim();

    let payload = msg
        .strip_prefix("STATE|")
        .ok_or_else(|| "bad response".to_string())?;
    serde_json::from_str(payload).map_err(|e| e.to_string())
}

fn parse_chain(chain_json: &str) -> Vec<Block> {
    serde_json::from_str(chain_json).unwrap_or_default()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:4000".to_string());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = AppState {
        selected: 0,
        status: "connecting...".to_string(),
        blocks: Vec::new(),
        node: None,
        list_state: ListState::default(),
    };

    let mut last_fetch = Instant::now() - Duration::from_secs(5);

    loop {
        if last_fetch.elapsed() >= Duration::from_millis(800) {
            match request_state(&addr) {
                Ok(state) => {
                    app.blocks = parse_chain(&state.chain);
                    app.node = Some(state);
                    if app.blocks.is_empty() {
                        app.selected = 0;
                        app.list_state.select(None);
                    } else {
                        if app.selected >= app.blocks.len() {
                            app.selected = app.blocks.len() - 1;
                        }
                        app.list_state.select(Some(app.selected));
                    }
                    app.status = "ok".to_string();
                }
                Err(e) => {
                    app.status = format!("error: {}", e);
                }
            }
            last_fetch = Instant::now();
        }

        terminal.draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
                .split(size);

            let header = if let Some(state) = &app.node {
                Line::from(vec![
                    Span::styled("height ", Style::default().fg(Color::Yellow)),
                    Span::raw(state.height.to_string()),
                    Span::raw("  "),
                    Span::styled("utxos ", Style::default().fg(Color::Yellow)),
                    Span::raw(state.utxo_count.to_string()),
                    Span::raw("  "),
                    Span::styled("mempool ", Style::default().fg(Color::Yellow)),
                    Span::raw(state.mempool.len().to_string()),
                    Span::raw("  "),
                    Span::styled("difficulty ", Style::default().fg(Color::Yellow)),
                    Span::raw(state.difficulty.to_string()),
                    Span::raw("  "),
                    Span::styled("tip ", Style::default().fg(Color::Yellow)),
                    Span::raw(state.tip.chars().take(12).collect::<String>()),
                    Span::raw("  "),
                    Span::styled("status ", Style::default().fg(Color::Yellow)),
                    Span::raw(&app.status),
                ])
            } else {
                Line::from(vec![Span::raw(&app.status)])
            };

            let header_block = Paragraph::new(header)
                .block(UiBlock::default().borders(Borders::ALL).title("node"));
            f.render_widget(header_block, chunks[0]);

            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
                .split(chunks[1]);

            let items: Vec<ListItem> = app
                .blocks
                .iter()
                .map(|b| {
                    let label = format!(
                        "#{:<5} {} txs  {}",
                        b.index,
                        b.transactions.len(),
                        b.hash.chars().take(12).collect::<String>()
                    );
                    ListItem::new(label)
                })
                .collect();

            let list = List::new(items)
                .block(UiBlock::default().borders(Borders::ALL).title("blocks"))
                .highlight_style(
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Cyan),
                );
            f.render_stateful_widget(list, body[0], &mut app.list_state);

            let detail_lines = if let Some(block) = app.blocks.get(app.selected) {
                vec![
                    Line::from(format!("index: {}", block.index)),
                    Line::from(format!("hash: {}", block.hash)),
                    Line::from(format!("prev: {}", block.previous_hash)),
                    Line::from(format!("merkle: {}", block.merkle_root)),
                    Line::from(format!("nonce: {}", block.nonce)),
                    Line::from(format!("timestamp: {}", block.timestamp)),
                    Line::from(format!("txs: {}", block.transactions.len())),
                ]
            } else {
                vec![Line::from("no blocks")]
            };

            let detail = Paragraph::new(detail_lines)
                .block(UiBlock::default().borders(Borders::ALL).title("details"))
                .wrap(Wrap { trim: true });
            f.render_widget(detail, body[1]);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Down => {
                        if !app.blocks.is_empty() && app.selected + 1 < app.blocks.len() {
                            app.selected += 1;
                            app.list_state.select(Some(app.selected));
                        }
                    }
                    KeyCode::Up => {
                        if app.selected > 0 {
                            app.selected -= 1;
                            app.list_state.select(Some(app.selected));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
