use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const BLOCK_REWARD: u64 = 50;

// --- Transactions (UTXO model) ---

#[derive(Clone, Debug)]
struct TxIn {
    txid: String,
    vout: usize,
}

#[derive(Clone, Debug)]
struct TxOut {
    address: String,
    amount: u64,
}

#[derive(Clone, Debug)]
struct Transaction {
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    timestamp: u64,
}

impl Transaction {
    fn new(inputs: Vec<TxIn>, outputs: Vec<TxOut>) -> Self {
        Transaction {
            inputs,
            outputs,
            timestamp: now_ts(),
        }
    }

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

    fn txid(&self) -> String {
        hash_str(&self.serialize())
    }
}

// --- Blocks ---

#[derive(Clone, Debug)]
struct Block {
    index: u32,
    timestamp: u64,
    transactions: Vec<Transaction>,
    previous_hash: String,
    hash: String,
    nonce: u64,
}

impl Block {
    fn calculate_hash(&self) -> String {
        let tx_data = self
            .transactions
            .iter()
            .map(|t| t.serialize())
            .collect::<Vec<_>>()
            .join("~~");
        let input = format!(
            "{};{};{};{};{}",
            self.index, self.timestamp, tx_data, self.previous_hash, self.nonce
        );
        hash_str(&input)
    }

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

// --- Blockchain ---

struct Blockchain {
    blocks: Vec<Block>,
    difficulty: usize,
    utxos: HashMap<(String, usize), TxOut>,
    miner_address: String,
}

impl Blockchain {
    fn new(difficulty: usize, miner_address: &str) -> Self {
        let coinbase = Transaction::coinbase(miner_address, BLOCK_REWARD);
        let mut genesis = Block::new(0, now_ts(), vec![coinbase], "0".to_string());
        genesis.mine_block(difficulty);

        let mut chain = Blockchain {
            blocks: vec![genesis],
            difficulty,
            utxos: HashMap::new(),
            miner_address: miner_address.to_string(),
        };
        chain.rebuild_utxos();
        chain
    }

    fn mine_block_with_txs(&mut self, mut transactions: Vec<Transaction>) -> Option<Block> {
        let coinbase = Transaction::coinbase(&self.miner_address, BLOCK_REWARD);
        transactions.insert(0, coinbase);

        let previous_hash = self.blocks.last().unwrap().hash.clone();
        let index = self.blocks.len() as u32;
        let timestamp = now_ts();

        let mut new_block = Block::new(index, timestamp, transactions, previous_hash);
        new_block.mine_block(self.difficulty);

        if !self.validate_block(&new_block) {
            return None;
        }

        self.apply_block(&new_block);
        self.blocks.push(new_block.clone());
        Some(new_block)
    }

    fn add_block(&mut self, block: Block) -> bool {
        if !self.validate_block(&block) {
            return false;
        }
        self.apply_block(&block);
        self.blocks.push(block);
        true
    }

    fn validate_block(&self, block: &Block) -> bool {
        if block.index as usize != self.blocks.len() {
            return false;
        }
        if block.previous_hash != self.blocks.last().unwrap().hash {
            return false;
        }
        if block.hash != block.calculate_hash() {
            return false;
        }
        if !block.hash.starts_with(&"0".repeat(self.difficulty)) {
            return false;
        }

        let mut temp_utxos = self.utxos.clone();
        self.validate_and_apply_transactions(&block.transactions, &mut temp_utxos)
    }

    fn validate_and_apply_transactions(
        &self,
        transactions: &[Transaction],
        utxos: &mut HashMap<(String, usize), TxOut>,
    ) -> bool {
        if transactions.is_empty() {
            return false;
        }

        if !transactions[0].inputs.is_empty() {
            return false;
        }
        for tx in transactions.iter().skip(1) {
            if tx.inputs.is_empty() {
                return false;
            }
        }

        let mut spent_in_block: HashSet<(String, usize)> = HashSet::new();

        for (i, tx) in transactions.iter().enumerate() {
            if i == 0 {
                for (vout, out) in tx.outputs.iter().enumerate() {
                    utxos.insert((tx.txid(), vout), out.clone());
                }
                continue;
            }

            let mut sum_in = 0u64;
            let mut sum_out = 0u64;

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

            for out in &tx.outputs {
                sum_out += out.amount;
            }

            if sum_in < sum_out {
                return false;
            }

            for input in &tx.inputs {
                utxos.remove(&(input.txid.clone(), input.vout));
            }
            for (vout, out) in tx.outputs.iter().enumerate() {
                utxos.insert((tx.txid(), vout), out.clone());
            }
        }

        true
    }

    fn apply_block(&mut self, block: &Block) {
        let _ = self.validate_and_apply_transactions(&block.transactions, &mut self.utxos);
    }

    fn rebuild_utxos(&mut self) {
        self.utxos.clear();
        for block in &self.blocks {
            let _ = self.validate_and_apply_transactions(&block.transactions, &mut self.utxos);
        }
    }

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
            if !block.hash.starts_with(&"0".repeat(self.difficulty)) {
                return false;
            }
            if i == 0 {
                if block.previous_hash != "0" {
                    return false;
                }
            } else if block.previous_hash != self.blocks[i - 1].hash {
                return false;
            }

            if !self.validate_and_apply_transactions(&block.transactions, &mut utxos) {
                return false;
            }
        }
        true
    }

    fn serialize_chain(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.serialize())
            .collect::<Vec<_>>()
            .join("##")
    }

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
}

// --- Helpers ---

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

fn build_transaction(
    from: &str,
    to: &str,
    amount: u64,
    utxos: &HashMap<(String, usize), TxOut>,
) -> Option<Transaction> {
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

// --- Networking ---

fn send_message(peer: &str, msg: &str) {
    if let Ok(mut stream) = TcpStream::connect(peer) {
        let _ = stream.write_all(msg.as_bytes());
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
    }
}

fn read_message(mut stream: TcpStream) -> Option<String> {
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn request_chain(peer: &str) -> Option<String> {
    let mut stream = TcpStream::connect(peer).ok()?;
    stream.write_all(b"REQ_CHAIN\n").ok()?;
    stream.flush().ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
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
                if new_chain.blocks.len() > local.blocks.len() {
                    *local = new_chain;
                }
            }
        }
    }
}

// --- Main ---

fn main() {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .expect("usage: blockchain_prototype <port> [peer1 peer2 ...]");
    let peers: Vec<String> = args.collect();

    let difficulty = 3;
    let my_addr = format!("miner:{}", port);

    let chain = Arc::new(Mutex::new(Blockchain::new(difficulty, &my_addr)));

    // Listener
    let listener_chain = Arc::clone(&chain);
    let listener_peers = peers.clone();
    let listener_addr = my_addr.clone();
    thread::spawn(move || {
        let listener =
            TcpListener::bind(format!("127.0.0.1:{}", port)).expect("failed to bind listener");
        for incoming in listener.incoming() {
            if let Ok(stream) = incoming {
                let chain = Arc::clone(&listener_chain);
                let peers = listener_peers.clone();
                let miner_address = listener_addr.clone();
                thread::spawn(move || {
                    if let Some(msg) = read_message(stream.try_clone().unwrap()) {
                        if msg == "REQ_CHAIN" {
                            let chain = chain.lock().unwrap();
                            let payload = chain.serialize_chain();
                            let _ = stream.write_all(format!("CHAIN|{}\n", payload).as_bytes());
                            let _ = stream.flush();
                            return;
                        }

                        if let Some(rest) = msg.strip_prefix("BLOCK|") {
                            if let Some(block) = Block::deserialize(rest) {
                                let mut chain = chain.lock().unwrap();
                                let ok = chain.add_block(block.clone());
                                drop(chain);
                                if ok {
                                    broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                                } else {
                                    sync_from_peers(
                                        &Arc::clone(&chain),
                                        &peers,
                                        difficulty,
                                        &miner_address,
                                    );
                                }
                            }
                            return;
                        }
                    }
                });
            }
        }
    });

    // Initial sync
    sync_from_peers(&chain, &peers, difficulty, &my_addr);

    // Simple CLI
    println!("Node {} running.", my_addr);
    println!("Commands: send <to> <amount> | mine | balance | chain | tamper");

    let stdin = io::stdin();
    loop {
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
                let mut chain = chain.lock().unwrap();
                if let Some(block) = chain.mine_block_with_txs(vec![tx]) {
                    drop(chain);
                    broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                    println!("block mined and broadcast");
                } else {
                    println!("block rejected");
                }
            } else {
                println!("insufficient funds");
            }
        } else if parts[0] == "mine" {
            let mut chain = chain.lock().unwrap();
            if let Some(block) = chain.mine_block_with_txs(Vec::new()) {
                drop(chain);
                broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                println!("block mined and broadcast");
            } else {
                println!("block rejected");
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
