// Simple educational blockchain with:
// - UTXO transactions
// - Proof-of-Work mining
// - Basic networking between nodes
// - Minimal CLI
// - FIXED: Proper fork handling and block validation
// Every section is commented so you can follow step-by-step.

use sha2::{Digest, Sha256}; // SHA-256 hashing for tx/block ids
use std::collections::{HashMap, HashSet}; // UTXO map + double-spend tracking
use std::io::{self, BufRead, BufReader, Write}; // CLI + network I/O
use std::net::{TcpListener, TcpStream}; // TCP networking
use std::sync::{Arc, Mutex}; // Shared state across threads
use std::thread; // Spawn threads
use std::time::{SystemTime, UNIX_EPOCH}; // Timestamps

const BLOCK_REWARD: u64 = 50; // Fixed block reward (coinbase)

// -------------------------
// Transactions (UTXO model)
// -------------------------

// Transaction input = references a previous unspent output.
#[derive(Clone, Debug)]
struct TxIn {
    txid: String, // transaction hash we are spending
    vout: usize,  // output index inside that transaction
}

// Transaction output = new coin ownership.
#[derive(Clone, Debug)]
struct TxOut {
    address: String, // who owns it
    amount: u64,     // how many coins
}

// Transaction = inputs + outputs + timestamp.
#[derive(Clone, Debug)]
struct Transaction {
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    timestamp: u64,
}

impl Transaction {
    // Create normal transaction.
    fn new(inputs: Vec<TxIn>, outputs: Vec<TxOut>) -> Self {
        Transaction {
            inputs,
            outputs,
            timestamp: now_ts(),
        }
    }

    // Create coinbase transaction (no inputs).
    fn coinbase(to: &str, amount: u64) -> Self {
        Transaction {
            inputs: Vec::new(),
            outputs: vec![TxOut {
                address: to.to_string(),
                amount,
            }],
            timestamp: now_ts(),
        }
    }

    // Serialize into a simple string (easy but not robust).
    fn serialize(&self) -> String {
        let inputs = if self.inputs.is_empty() {
            "-".to_string()
        } else {
            self.inputs
                .iter()
                .map(|i| format!("{}:{}", i.txid, i.vout))
                .collect::<Vec<_>>()
                .join(",")
        };
        let outputs = if self.outputs.is_empty() {
            "-".to_string()
        } else {
            self.outputs
                .iter()
                .map(|o| format!("{}:{}", o.address, o.amount))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!("{}|{}|{}", inputs, outputs, self.timestamp)
    }

    // Deserialize from the string format above.
    fn deserialize(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('|').collect();
        if parts.len() != 3 {
            return None;
        }

        let inputs = if parts[0] == "-" {
            Vec::new()
        } else {
            parts[0]
                .split(',')
                .filter_map(|p| {
                    let kv: Vec<&str> = p.split(':').collect();
                    if kv.len() != 2 {
                        return None;
                    }
                    Some(TxIn {
                        txid: kv[0].to_string(),
                        vout: kv[1].parse().ok()?,
                    })
                })
                .collect()
        };

        let outputs = if parts[1] == "-" {
            Vec::new()
        } else {
            parts[1]
                .split(',')
                .filter_map(|p| {
                    let kv: Vec<&str> = p.split(':').collect();
                    if kv.len() != 2 {
                        return None;
                    }
                    Some(TxOut {
                        address: kv[0].to_string(),
                        amount: kv[1].parse().ok()?,
                    })
                })
                .collect()
        };

        let timestamp = parts[2].parse().ok()?;
        Some(Transaction {
            inputs,
            outputs,
            timestamp,
        })
    }

    // Transaction id = SHA-256 of serialized data.
    fn txid(&self) -> String {
        hash_str(&self.serialize())
    }
}

// ----- Blocks -----

#[derive(Clone, Debug)]
struct Block {
    index: u32,                     // block height
    timestamp: u64,                 // creation time
    transactions: Vec<Transaction>, // list of transactions
    previous_hash: String,          // hash of previous block
    hash: String,                   // hash of this block
    nonce: u64,                     // PoW nonce
}

impl Block {
    // Build the hash input string and hash it.
    fn calculate_hash(&self) -> String {
        let tx_data = self
            .transactions
            .iter()
            .map(|t| t.serialize())
            .collect::<Vec<_>>()
            .join("~~"); // simple separator
        let input = format!(
            "{};{};{};{};{}",
            self.index, self.timestamp, tx_data, self.previous_hash, self.nonce
        );
        hash_str(&input)
    }

    // Create a new block (not yet mined).
    fn new(
        index: u32,
        timestamp: u64,
        transactions: Vec<Transaction>,
        previous_hash: String,
    ) -> Self {
        let mut block = Block {
            index,
            timestamp,
            transactions,
            previous_hash,
            hash: String::new(),
            nonce: 0,
        };
        block.hash = block.calculate_hash();
        block
    }

    // Proof-of-Work: find nonce so hash starts with N zeros.
    fn mine_block(&mut self, difficulty: usize) {
        let target = "0".repeat(difficulty);
        loop {
            self.hash = self.calculate_hash();
            if self.hash.starts_with(&target) {
                break;
            }
            self.nonce += 1;
        }
    }

    // Serialize block to a string.
    fn serialize(&self) -> String {
        let txs = if self.transactions.is_empty() {
            "-".to_string()
        } else {
            self.transactions
                .iter()
                .map(|t| t.serialize())
                .collect::<Vec<_>>()
                .join("~~")
        };
        format!(
            "{};{};{};{};{}",
            self.index, self.timestamp, self.previous_hash, self.nonce, txs
        )
    }

    // Deserialize a block from string.
    fn deserialize(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(';').collect();
        if parts.len() != 5 {
            return None;
        }
        let index = parts[0].parse().ok()?;
        let timestamp = parts[1].parse().ok()?;
        let previous_hash = parts[2].to_string();
        let nonce = parts[3].parse().ok()?;
        let transactions = if parts[4] == "-" {
            Vec::new()
        } else {
            parts[4]
                .split("~~")
                .filter_map(Transaction::deserialize)
                .collect()
        };

        let mut block = Block {
            index,
            timestamp,
            transactions,
            previous_hash,
            hash: String::new(),
            nonce,
        };
        block.hash = block.calculate_hash();
        Some(block)
    }
}

// ----------------
// Blockchain state
// ----------------

struct Blockchain {
    blocks: Vec<Block>,                     // chain of blocks
    difficulty: usize,                      // PoW difficulty
    utxos: HashMap<(String, usize), TxOut>, // UTXO set
    miner_address: String,                  // where coinbase goes
}

impl Blockchain {
    // Create chain with a mined genesis block.
    fn new(difficulty: usize, miner_address: &str) -> Self {
        // Deterministic genesis so all nodes start from the same chain.
        let coinbase = Transaction {
            inputs: Vec::new(),
            outputs: vec![TxOut {
                address: "genesis".to_string(),
                amount: 0,
            }],
            timestamp: 0,
        };
        let genesis = Block::new(0, 0, vec![coinbase], "0".to_string());

        let mut chain = Blockchain {
            blocks: vec![genesis],
            difficulty,
            utxos: HashMap::new(),
            miner_address: miner_address.to_string(),
        };
        chain.rebuild_utxos();
        chain
    }

    // Mine a block locally and add it (returns the block).
    fn mine_block_with_txs(&mut self, mut transactions: Vec<Transaction>) -> Option<Block> {
        // Always insert coinbase first.
        let coinbase = Transaction::coinbase(&self.miner_address, BLOCK_REWARD);
        transactions.insert(0, coinbase);

        let previous_hash = self.blocks.last().unwrap().hash.clone();
        let index = self.blocks.len() as u32;
        let timestamp = now_ts();

        let mut new_block = Block::new(index, timestamp, transactions, previous_hash);
        new_block.mine_block(self.difficulty);

        // FIX: Use validate_and_add_block instead of separate validate/apply
        if self.try_add_block(new_block.clone()) {
            Some(new_block)
        } else {
            None
        }
    }

    // FIX: New unified method for adding blocks (handles forks correctly)
    fn try_add_block(&mut self, block: Block) -> bool {
        // Case 1: Block extends our chain
        if block.index as usize == self.blocks.len()
            && block.previous_hash == self.blocks.last().unwrap().hash
        {
            if self.validate_block_structure(&block) {
                self.apply_block(&block);
                self.blocks.push(block);
                return true;
            }
            return false;
        }

        // Case 2: Block is from a competing fork - reject it
        // (A full node would handle reorgs, but we keep it simple)
        false
    }

    // Add a block received from the network.
    fn add_block(&mut self, block: Block) -> bool {
        self.try_add_block(block)
    }

    // FIX: Renamed and simplified - validates structure, PoW, and transactions
    fn validate_block_structure(&self, block: &Block) -> bool {
        // Hash must be correct
        if block.hash != block.calculate_hash() {
            println!("  -> hash mismatch");
            return false;
        }

        // PoW must satisfy difficulty (skip genesis)
        if block.index > 0 && !block.hash.starts_with(&"0".repeat(self.difficulty)) {
            println!("  -> PoW insufficient");
            return false;
        }

        // Validate transactions against a temporary UTXO set
        let mut temp_utxos = self.utxos.clone();
        if !Self::validate_and_apply_transactions(&block.transactions, &mut temp_utxos) {
            println!("  -> transaction validation failed");
            return false;
        }

        true
    }

    // Validate and apply txs to a UTXO set.
    fn validate_and_apply_transactions(
        transactions: &[Transaction],
        utxos: &mut HashMap<(String, usize), TxOut>,
    ) -> bool {
        if transactions.is_empty() {
            return false;
        }

        // First tx must be coinbase (no inputs).
        if !transactions[0].inputs.is_empty() {
            return false;
        }
        // All other txs must have inputs.
        for tx in transactions.iter().skip(1) {
            if tx.inputs.is_empty() {
                return false;
            }
        }

        // Track double-spends within this block.
        let mut spent_in_block: HashSet<(String, usize)> = HashSet::new();

        for (i, tx) in transactions.iter().enumerate() {
            if i == 0 {
                // Coinbase: just add outputs.
                for (vout, out) in tx.outputs.iter().enumerate() {
                    utxos.insert((tx.txid(), vout), out.clone());
                }
                continue;
            }

            let mut sum_in = 0u64;
            let mut sum_out = 0u64;

            // Check all inputs exist and not double-spent.
            for input in &tx.inputs {
                let key = (input.txid.clone(), input.vout);
                if spent_in_block.contains(&key) {
                    return false;
                }
                let prev = match utxos.get(&key) {
                    Some(o) => o,
                    None => return false,
                };
                sum_in += prev.amount;
                spent_in_block.insert(key);
            }

            // Sum outputs.
            for out in &tx.outputs {
                sum_out += out.amount;
            }

            // Must not create money.
            if sum_in < sum_out {
                return false;
            }

            // Apply: remove inputs, add outputs.
            for input in &tx.inputs {
                utxos.remove(&(input.txid.clone(), input.vout));
            }
            for (vout, out) in tx.outputs.iter().enumerate() {
                utxos.insert((tx.txid(), vout), out.clone());
            }
        }

        true
    }

    // Apply a block to the real UTXO set.
    fn apply_block(&mut self, block: &Block) {
        let _ = Self::validate_and_apply_transactions(&block.transactions, &mut self.utxos);
    }

    // Recompute UTXO set from scratch (used on startup).
    fn rebuild_utxos(&mut self) {
        self.utxos.clear();
        for block in &self.blocks {
            let _ = Self::validate_and_apply_transactions(&block.transactions, &mut self.utxos);
        }
    }

    // Full chain validation (hashes + PoW + UTXO).
    fn is_chain_valid(&self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }

        let mut utxos = HashMap::new();
        for i in 0..self.blocks.len() {
            let block = &self.blocks[i];
            if block.hash != block.calculate_hash() {
                return false;
            }
            if i != 0 && !block.hash.starts_with(&"0".repeat(self.difficulty)) {
                return false;
            }
            if i == 0 {
                if block.previous_hash != "0" {
                    return false;
                }
            } else if block.previous_hash != self.blocks[i - 1].hash {
                return false;
            }

            if !Self::validate_and_apply_transactions(&block.transactions, &mut utxos) {
                return false;
            }
        }
        true
    }

    // Serialize the full chain for syncing.
    fn serialize_chain(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.serialize())
            .collect::<Vec<_>>()
            .join("##")
    }

    // Deserialize a chain and validate it.
    fn deserialize_chain(s: &str, difficulty: usize, miner_address: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let blocks: Vec<Block> = s.split("##").filter_map(Block::deserialize).collect();
        if blocks.is_empty() {
            return None;
        }
        let mut chain = Blockchain {
            blocks,
            difficulty,
            utxos: HashMap::new(),
            miner_address: miner_address.to_string(),
        };
        if !chain.is_chain_valid() {
            return None;
        }
        chain.rebuild_utxos();
        Some(chain)
    }

    // FIX: Remove transactions from mempool that are in a block
    fn remove_txs_from_mempool(&self, mempool: &mut Vec<Transaction>, block: &Block) {
        let block_txids: HashSet<String> = block.transactions.iter().map(|tx| tx.txid()).collect();
        mempool.retain(|tx| !block_txids.contains(&tx.txid()));
    }
}

// --------- Helpers ---------

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

fn hash_str(input: &str) -> String {
    let hash_bytes = Sha256::digest(input.as_bytes());
    format!("{:x}", hash_bytes)
}

// Addresses must not contain our serialization separators.
fn is_address_safe(addr: &str) -> bool {
    !addr
        .chars()
        .any(|c| c == ':' || c == ',' || c == '|' || c == '~' || c == ';')
}

// Build a simple transaction by selecting UTXOs for "from".
fn build_transaction(
    from: &str,
    to: &str,
    amount: u64,
    utxos: &HashMap<(String, usize), TxOut>,
) -> Option<Transaction> {
    if !is_address_safe(from) || !is_address_safe(to) {
        return None;
    }
    let mut selected = Vec::new();
    let mut total = 0u64;

    for ((txid, vout), out) in utxos.iter() {
        if out.address == from {
            selected.push(TxIn {
                txid: txid.clone(),
                vout: *vout,
            });
            total += out.amount;
            if total >= amount {
                break;
            }
        }
    }

    if total < amount {
        return None;
    }

    let mut outputs = vec![TxOut {
        address: to.to_string(),
        amount,
    }];

    let change = total - amount;
    if change > 0 {
        outputs.push(TxOut {
            address: from.to_string(),
            amount: change,
        });
    }

    Some(Transaction::new(selected, outputs))
}

// ------------- Networking -------------

fn send_message(peer: &str, msg: &str) {
    if let Ok(mut stream) = TcpStream::connect(peer) {
        let _ = stream.write_all(msg.as_bytes());
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
    }
}

fn read_message(stream: TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf).ok()?;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn request_chain(peer: &str) -> Option<String> {
    let mut stream = TcpStream::connect(peer).ok()?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
    stream.write_all(b"REQ_CHAIN\n").ok()?;
    stream.flush().ok()?;
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf).ok()?;
    let msg = buf.trim().to_string();
    if let Some(rest) = msg.strip_prefix("CHAIN|") {
        Some(rest.to_string())
    } else {
        None
    }
}

fn broadcast(peers: &[String], msg: &str) {
    for p in peers {
        send_message(p, msg);
    }
}

// Pull a longer chain from peers (very simple consensus).
fn sync_from_peers(
    chain: &Arc<Mutex<Blockchain>>,
    peers: &[String],
    difficulty: usize,
    miner_address: &str,
) {
    for peer in peers {
        if let Some(payload) = request_chain(peer) {
            if let Some(new_chain) =
                Blockchain::deserialize_chain(&payload, difficulty, miner_address)
            {
                let mut local = chain.lock().unwrap();
                let local_len = local.blocks.len();
                let new_len = new_chain.blocks.len();

                // FIX: Only replace if strictly longer (prevent fork confusion)
                if new_len > local_len {
                    println!("synced to longer chain (height {})", new_len);
                    *local = new_chain;
                }
            }
        }
    }
}

// ------------- Main -------------

fn main() {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .expect("usage: blockchain_prototype <port> [peer1 peer2 ...] [--miner]");
    let mut is_miner = false;
    let mut peers: Vec<String> = Vec::new();
    for arg in args {
        if arg == "--miner" {
            is_miner = true;
        } else {
            peers.push(arg);
        }
    }

    let difficulty = 2;
    let my_addr = format!("miner-{}", port);

    // Shared chain state for listener thread + CLI thread.
    let chain = Arc::new(Mutex::new(Blockchain::new(difficulty, &my_addr)));
    // Shared mempool (pending transactions).
    let mempool = Arc::new(Mutex::new(Vec::<Transaction>::new()));
    // Flag to avoid starting multiple miners at once.
    let mining_flag = Arc::new(Mutex::new(false));

    // Listener thread handles incoming network messages.
    let listener_chain = Arc::clone(&chain);
    let listener_mempool = Arc::clone(&mempool);
    let listener_peers = peers.clone();
    let listener_addr = my_addr.clone();
    thread::spawn(move || {
        let listener =
            TcpListener::bind(format!("127.0.0.1:{}", port)).expect("failed to bind listener");
        for incoming in listener.incoming() {
            if let Ok(mut stream) = incoming {
                let chain_arc = Arc::clone(&listener_chain);
                let mempool_arc = Arc::clone(&listener_mempool);
                let peers = listener_peers.clone();
                let miner_address = listener_addr.clone();
                thread::spawn(move || {
                    if let Some(msg) = read_message(stream.try_clone().unwrap()) {
                        if msg == "REQ_CHAIN" {
                            let chain = chain_arc.lock().unwrap();
                            let payload = chain.serialize_chain();
                            let _ = stream.write_all(format!("CHAIN|{}\n", payload).as_bytes());
                            let _ = stream.flush();
                            return;
                        }

                        if let Some(rest) = msg.strip_prefix("TX|") {
                            if let Some(tx) = Transaction::deserialize(rest) {
                                println!("received tx");
                                let mut pool = mempool_arc.lock().unwrap();
                                if !pool.iter().any(|t| t.txid() == tx.txid()) {
                                    pool.push(tx);
                                }
                            }
                            return;
                        }

                        if let Some(rest) = msg.strip_prefix("BLOCK|") {
                            if let Some(block) = Block::deserialize(rest) {
                                println!(
                                    "received block (index={}, prev={}...)",
                                    block.index,
                                    &block.previous_hash[..8]
                                );

                                let mut chain = chain_arc.lock().unwrap();
                                let current_height = chain.blocks.len();
                                let current_tip = chain
                                    .blocks
                                    .last()
                                    .map(|b| b.hash.clone())
                                    .unwrap_or_default();

                                let ok = chain.add_block(block.clone());

                                // FIX: Clean mempool when we accept a block
                                if ok {
                                    let mut pool = mempool_arc.lock().unwrap();
                                    chain.remove_txs_from_mempool(&mut pool, &block);
                                }

                                drop(chain);

                                if ok {
                                    println!("  -> accepted (extended chain)");
                                    broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                                } else {
                                    println!(
                                        "  -> rejected (current height={}, tip={}...)",
                                        current_height,
                                        &current_tip[..8]
                                    );
                                    // Only sync if we might be behind
                                    if block.index as usize >= current_height {
                                        println!("  -> syncing from peers");
                                        sync_from_peers(
                                            &chain_arc,
                                            &peers,
                                            difficulty,
                                            &miner_address,
                                        );
                                    }
                                }
                            }
                            return;
                        }
                    }
                });
            }
        }
    });

    // Initial sync at startup.
    sync_from_peers(&chain, &peers, difficulty, &my_addr);

    // CLI commands.
    println!("Node {} running.", my_addr);
    println!("Commands: send <to> <amount> | mine | balance | chain | tamper");
    println!("Note: mining requires --miner");

    let stdin = io::stdin();
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line).is_err() {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts[0] == "send" && parts.len() == 3 {
            let to = parts[1];
            let amount: u64 = match parts[2].parse() {
                Ok(a) => a,
                Err(_) => {
                    println!("invalid amount");
                    continue;
                }
            };

            let utxos = chain.lock().unwrap().utxos.clone();
            if let Some(tx) = build_transaction(&my_addr, to, amount, &utxos) {
                let mut pool = mempool.lock().unwrap();
                if !pool.iter().any(|t| t.txid() == tx.txid()) {
                    pool.push(tx.clone());
                }
                drop(pool);
                broadcast(&peers, &format!("TX|{}", tx.serialize()));
                println!("tx broadcast");
            } else {
                println!("insufficient funds or invalid address");
            }
        } else if parts[0] == "mine" {
            if !is_miner {
                println!("mining disabled (start with --miner)");
            } else {
                let flag = Arc::clone(&mining_flag);
                if *flag.lock().unwrap() {
                    println!("mining in progress");
                } else {
                    *flag.lock().unwrap() = true;
                    let chain = Arc::clone(&chain);
                    let peers = peers.clone();
                    let mempool = Arc::clone(&mempool);
                    thread::spawn(move || {
                        let txs = {
                            let mut pool = mempool.lock().unwrap();
                            let txs = pool.drain(..).collect::<Vec<_>>();
                            txs
                        };
                        let mut chain = chain.lock().unwrap();
                        let block = chain.mine_block_with_txs(txs);
                        drop(chain);
                        if let Some(block) = block {
                            broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                            println!("block mined and broadcast");
                        } else {
                            println!("block rejected");
                        }
                        *flag.lock().unwrap() = false;
                    });
                }
            }
        } else if parts[0] == "balance" {
            let chain = chain.lock().unwrap();
            let mut total = 0u64;
            for out in chain.utxos.values() {
                if out.address == my_addr {
                    total += out.amount;
                }
            }
            println!("balance: {}", total);
        } else if parts[0] == "chain" {
            let chain = chain.lock().unwrap();
            println!("height: {}", chain.blocks.len());
            println!("valid: {}", chain.is_chain_valid());
            println!("tip: {}", chain.blocks.last().unwrap().hash);
        } else if parts[0] == "tamper" {
            let mut chain = chain.lock().unwrap();
            if chain.blocks.len() > 1 {
                chain.blocks[1].transactions[0].outputs[0].amount += 1;
                println!("tampered block 1");
            } else {
                println!("no block to tamper");
            }
        } else {
            println!("unknown command");
        }
    }
}
