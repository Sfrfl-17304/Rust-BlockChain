use k256::ecdsa::{
    Signature, SigningKey, VerifyingKey,
    signature::{Signer, Verifier},
};
use rand_core::OsRng;
use ripemd::Ripemd160;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const BLOCK_REWARD: u64 = 50;
const CHAIN_PATH: &str = "data/chain.json";

#[derive(Clone, Debug)]
struct Wallet {
    signing_key: SigningKey,
    pubkey_hex: String,
    pubkey_hash_hex: String,
}

impl Wallet {
    fn new() -> Self {
        let signing_key = SigningKey::random(&mut OsRng);
        let verifying_key = *signing_key.verifying_key();
        let pubkey_bytes = verifying_key.to_encoded_point(true).as_bytes().to_vec();
        let pubkey_hex = hex::encode(&pubkey_bytes);
        let pubkey_hash_hex = hash160_hex(&pubkey_bytes);
        Self {
            signing_key,
            pubkey_hex,
            pubkey_hash_hex,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TxIn {
    txid: String,
    vout: usize,
    signature: String, // DER-encoded signature in hex
    pubkey: String,    // compressed pubkey hex
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TxOut {
    pubkey_hash: String, // HASH160(pubkey) hex
    amount: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Transaction {
    inputs: Vec<TxIn>,
    outputs: Vec<TxOut>,
    timestamp: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NodeState {
    chain: String,
    height: usize,
    tip: String,
    mempool: Vec<Transaction>,
    utxo_count: usize,
    difficulty: u32,
}

impl Transaction {
    fn new(inputs: Vec<TxIn>, outputs: Vec<TxOut>) -> Self {
        Self {
            inputs,
            outputs,
            timestamp: now_ts(),
        }
    }

    fn coinbase(to_pubkey_hash: &str, amount: u64) -> Self {
        Self {
            inputs: Vec::new(),
            outputs: vec![TxOut {
                pubkey_hash: to_pubkey_hash.to_string(),
                amount,
            }],
            timestamp: now_ts(),
        }
    }

    fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn deserialize(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }

    fn txid(&self) -> String {
        hash_str(&self.serialize())
    }

    // Canonical message for signing one input.
    // Clears all input sig/pubkeys and commits to prevout being spent.
    fn sighash_for_input(&self, input_index: usize, prev_pubkey_hash: &str) -> String {
        let mut tx = self.clone();
        for i in 0..tx.inputs.len() {
            tx.inputs[i].signature.clear();
            tx.inputs[i].pubkey.clear();
        }
        let base = serde_json::json!({
            "tx": tx,
            "input_index": input_index,
            "prev_pubkey_hash": prev_pubkey_hash,
        });
        hash_str(&base.to_string())
    }

    fn sign_input(&mut self, input_index: usize, wallet: &Wallet, prev_pubkey_hash: &str) -> bool {
        if input_index >= self.inputs.len() {
            return false;
        }
        let digest_hex = self.sighash_for_input(input_index, prev_pubkey_hash);
        let sig: Signature = wallet.signing_key.sign(digest_hex.as_bytes());
        self.inputs[input_index].signature = hex::encode(sig.to_der().as_bytes());
        self.inputs[input_index].pubkey = wallet.pubkey_hex.clone();
        true
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Block {
    index: u32,
    timestamp: u64,
    transactions: Vec<Transaction>,
    previous_hash: String,
    merkle_root: String,
    difficulty: u32, // leading zero bits target style (simple)
    hash: String,
    nonce: u64,
}

impl Block {
    fn calculate_hash(&self) -> String {
        let header = serde_json::json!({
            "index": self.index,
            "timestamp": self.timestamp,
            "previous_hash": self.previous_hash,
            "merkle_root": self.merkle_root,
            "difficulty": self.difficulty,
            "nonce": self.nonce
        });
        hash_str(&header.to_string())
    }

    fn new(
        index: u32,
        timestamp: u64,
        transactions: Vec<Transaction>,
        previous_hash: String,
        difficulty: u32,
    ) -> Self {
        let merkle_root = merkle_root_hex(&transactions);
        let mut block = Self {
            index,
            timestamp,
            transactions,
            previous_hash,
            merkle_root,
            difficulty,
            hash: String::new(),
            nonce: 0,
        };
        block.hash = block.calculate_hash();
        block
    }

    fn mine_block(&mut self) {
        loop {
            self.hash = self.calculate_hash();
            if has_leading_zero_bits_hex(&self.hash, self.difficulty) {
                break;
            }
            self.nonce = self.nonce.wrapping_add(1);
        }
    }

    fn serialize(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn deserialize(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

struct Blockchain {
    blocks: Vec<Block>,
    difficulty: u32,
    utxos: HashMap<(String, usize), TxOut>,
    miner_pubkey_hash: String,
}

impl Blockchain {
    fn new(difficulty: u32, miner_pubkey_hash: &str) -> Self {
        let genesis_tx = Transaction {
            inputs: vec![],
            outputs: vec![TxOut {
                pubkey_hash: "genesis".to_string(),
                amount: 0,
            }],
            timestamp: 0,
        };
        let mut genesis = Block::new(0, 0, vec![genesis_tx], "0".to_string(), difficulty);
        genesis.mine_block();

        let mut chain = Self {
            blocks: vec![genesis],
            difficulty,
            utxos: HashMap::new(),
            miner_pubkey_hash: miner_pubkey_hash.to_string(),
        };
        chain.rebuild_utxos();
        chain
    }

    fn mine_block_with_txs(&mut self, mut txs: Vec<Transaction>) -> Option<Block> {
        let coinbase = Transaction::coinbase(&self.miner_pubkey_hash, BLOCK_REWARD);
        txs.insert(0, coinbase);

        let prev_hash = self.blocks.last().unwrap().hash.clone();
        let index = self.blocks.len() as u32;
        let mut block = Block::new(index, now_ts(), txs, prev_hash, self.difficulty);
        block.mine_block();

        if self.try_add_block(block.clone()) {
            Some(block)
        } else {
            None
        }
    }

    fn try_add_block(&mut self, block: Block) -> bool {
        if block.index as usize != self.blocks.len() {
            return false;
        }
        if block.previous_hash != self.blocks.last().unwrap().hash {
            return false;
        }
        if !self.validate_block_structure(&block) {
            return false;
        }
        self.apply_block(&block);
        self.blocks.push(block);
        true
    }

    fn add_block(&mut self, block: Block) -> bool {
        self.try_add_block(block)
    }

    fn validate_block_structure(&self, block: &Block) -> bool {
        if block.hash != block.calculate_hash() {
            return false;
        }
        if !has_leading_zero_bits_hex(&block.hash, block.difficulty) {
            return false;
        }
        if block.difficulty != self.difficulty {
            return false;
        }
        let recomputed_merkle = merkle_root_hex(&block.transactions);
        if recomputed_merkle != block.merkle_root {
            return false;
        }

        let mut temp = self.utxos.clone();
        Self::validate_and_apply_transactions(&block.transactions, &mut temp)
    }

    fn validate_and_apply_transactions(
        txs: &[Transaction],
        utxos: &mut HashMap<(String, usize), TxOut>,
    ) -> bool {
        if txs.is_empty() {
            return false;
        }

        // coinbase rules
        if !txs[0].inputs.is_empty() {
            return false;
        }
        let coinbase_out_sum: u64 = txs[0].outputs.iter().map(|o| o.amount).sum();

        let mut spent_in_block: HashSet<(String, usize)> = HashSet::new();
        let mut total_fees = 0u64;

        for (i, tx) in txs.iter().enumerate() {
            if i == 0 {
                continue;
            }
            if tx.inputs.is_empty() || tx.outputs.is_empty() {
                return false;
            }

            let mut sum_in = 0u64;
            let mut sum_out = 0u64;

            for (input_idx, input) in tx.inputs.iter().enumerate() {
                let key = (input.txid.clone(), input.vout);

                if spent_in_block.contains(&key) {
                    return false;
                }

                let prev = match utxos.get(&key) {
                    Some(v) => v,
                    None => return false,
                };

                // P2PKH checks
                let pubkey_bytes = match hex::decode(&input.pubkey) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let computed_pkh = hash160_hex(&pubkey_bytes);
                if computed_pkh != prev.pubkey_hash {
                    return false;
                }

                let digest_hex = tx.sighash_for_input(input_idx, &prev.pubkey_hash);

                let vk = match VerifyingKey::from_sec1_bytes(&pubkey_bytes) {
                    Ok(k) => k,
                    Err(_) => return false,
                };
                let sig_bytes = match hex::decode(&input.signature) {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                let sig = match Signature::from_der(&sig_bytes) {
                    Ok(s) => s,
                    Err(_) => return false,
                };

                if vk.verify(digest_hex.as_bytes(), &sig).is_err() {
                    return false;
                }

                sum_in = match sum_in.checked_add(prev.amount) {
                    Some(v) => v,
                    None => return false,
                };
                spent_in_block.insert(key);
            }

            for out in &tx.outputs {
                if out.amount == 0 {
                    return false;
                }
                sum_out = match sum_out.checked_add(out.amount) {
                    Some(v) => v,
                    None => return false,
                };
            }

            if sum_in < sum_out {
                return false;
            }

            total_fees = match total_fees.checked_add(sum_in - sum_out) {
                Some(v) => v,
                None => return false,
            };
        }

        if coinbase_out_sum > BLOCK_REWARD + total_fees {
            return false;
        }

        // Apply txs
        for (i, tx) in txs.iter().enumerate() {
            if i != 0 {
                for input in &tx.inputs {
                    utxos.remove(&(input.txid.clone(), input.vout));
                }
            }
            let txid = tx.txid();
            for (vout, out) in tx.outputs.iter().enumerate() {
                utxos.insert((txid.clone(), vout), out.clone());
            }
        }

        true
    }

    fn apply_block(&mut self, block: &Block) {
        let _ = Self::validate_and_apply_transactions(&block.transactions, &mut self.utxos);
    }

    fn rebuild_utxos(&mut self) {
        self.utxos.clear();
        for b in &self.blocks {
            let _ = Self::validate_and_apply_transactions(&b.transactions, &mut self.utxos);
        }
    }

    fn chain_work(&self) -> u128 {
        // simple cumulative work approx: sum 2^difficulty
        self.blocks
            .iter()
            .map(|b| 1u128 << (b.difficulty.min(120)))
            .sum()
    }

    fn is_chain_valid(&self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        let mut temp_utxos = HashMap::new();

        for i in 0..self.blocks.len() {
            let b = &self.blocks[i];
            if b.hash != b.calculate_hash() {
                return false;
            }
            if !has_leading_zero_bits_hex(&b.hash, b.difficulty) {
                return false;
            }
            if merkle_root_hex(&b.transactions) != b.merkle_root {
                return false;
            }
            if i == 0 {
                if b.previous_hash != "0" {
                    return false;
                }
            } else if b.previous_hash != self.blocks[i - 1].hash {
                return false;
            }
            if !Self::validate_and_apply_transactions(&b.transactions, &mut temp_utxos) {
                return false;
            }
        }
        true
    }

    fn serialize_chain(&self) -> String {
        serde_json::to_string(&self.blocks).unwrap_or_default()
    }

    fn deserialize_chain(payload: &str, difficulty: u32, miner_pubkey_hash: &str) -> Option<Self> {
        let blocks: Vec<Block> = serde_json::from_str(payload).ok()?;
        if blocks.is_empty() {
            return None;
        }
        let mut chain = Self {
            blocks,
            difficulty,
            utxos: HashMap::new(),
            miner_pubkey_hash: miner_pubkey_hash.to_string(),
        };
        if !chain.is_chain_valid() {
            return None;
        }
        chain.rebuild_utxos();
        Some(chain)
    }

    fn remove_txs_from_mempool(&self, mempool: &mut Vec<Transaction>, block: &Block) {
        let set: HashSet<String> = block.transactions.iter().map(|t| t.txid()).collect();
        mempool.retain(|t| !set.contains(&t.txid()));
    }
}

fn load_chain_from_disk(
    path: &str,
    difficulty: u32,
    miner_pubkey_hash: &str,
) -> Option<Blockchain> {
    let data = fs::read_to_string(path).ok()?;
    Blockchain::deserialize_chain(&data, difficulty, miner_pubkey_hash)
}

fn save_chain_to_disk(path: &str, chain: &Blockchain) -> bool {
    if let Some(parent) = Path::new(path).parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    fs::write(path, chain.serialize_chain()).is_ok()
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time error")
        .as_secs()
}

fn hash_str(input: &str) -> String {
    let h = Sha256::digest(input.as_bytes());
    hex::encode(h)
}

fn hash160_hex(data: &[u8]) -> String {
    let sha = Sha256::digest(data);
    let ripe = Ripemd160::digest(sha);
    hex::encode(ripe)
}

fn has_leading_zero_bits_hex(hash_hex: &str, zero_bits: u32) -> bool {
    let bytes = match hex::decode(hash_hex) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut rem = zero_bits;
    for b in bytes {
        if rem == 0 {
            return true;
        }
        if rem >= 8 {
            if b != 0 {
                return false;
            }
            rem -= 8;
        } else {
            let mask = 0xFFu8 << (8 - rem);
            return (b & mask) == 0;
        }
    }
    rem == 0
}

fn merkle_root_hex(txs: &[Transaction]) -> String {
    if txs.is_empty() {
        return hash_str("");
    }
    let mut layer: Vec<String> = txs.iter().map(|t| t.txid()).collect();
    while layer.len() > 1 {
        if layer.len() % 2 == 1 {
            let last = layer.last().cloned().unwrap();
            layer.push(last);
        }
        let mut next = Vec::with_capacity(layer.len() / 2);
        let mut i = 0usize;
        while i < layer.len() {
            next.push(hash_str(&(layer[i].clone() + &layer[i + 1])));
            i += 2;
        }
        layer = next;
    }
    layer[0].clone()
}

fn build_transaction(
    wallet: &Wallet,
    to_pubkey_hash: &str,
    amount: u64,
    utxos: &HashMap<(String, usize), TxOut>,
) -> Option<Transaction> {
    let mut selected: Vec<((String, usize), TxOut)> = Vec::new();
    let mut total = 0u64;

    for (k, out) in utxos.iter() {
        if out.pubkey_hash == wallet.pubkey_hash_hex {
            selected.push((k.clone(), out.clone()));
            total = total.checked_add(out.amount)?;
            if total >= amount {
                break;
            }
        }
    }

    if total < amount {
        return None;
    }

    let mut inputs = Vec::new();
    for ((txid, vout), _) in &selected {
        inputs.push(TxIn {
            txid: txid.clone(),
            vout: *vout,
            signature: String::new(),
            pubkey: String::new(),
        });
    }

    let mut outputs = vec![TxOut {
        pubkey_hash: to_pubkey_hash.to_string(),
        amount,
    }];

    let change = total - amount;
    if change > 0 {
        outputs.push(TxOut {
            pubkey_hash: wallet.pubkey_hash_hex.clone(),
            amount: change,
        });
    }

    let mut tx = Transaction::new(inputs, outputs);

    for (i, ((txid, vout), _)) in selected.iter().enumerate() {
        let prev = utxos.get(&(txid.clone(), *vout))?;
        if !tx.sign_input(i, wallet, &prev.pubkey_hash) {
            return None;
        }
    }

    Some(tx)
}

fn send_message(peer: &str, msg: &str) {
    if let Ok(mut s) = TcpStream::connect(peer) {
        let _ = s.write_all(msg.as_bytes());
        let _ = s.write_all(b"\n");
        let _ = s.flush();
    }
}

fn read_message(stream: TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf).ok()?;
    let t = buf.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
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
    msg.strip_prefix("CHAIN|").map(|s| s.to_string())
}

fn broadcast(peers: &[String], msg: &str) {
    for p in peers {
        send_message(p, msg);
    }
}

fn sync_from_peers(
    chain: &Arc<Mutex<Blockchain>>,
    peers: &[String],
    difficulty: u32,
    miner_pubkey_hash: &str,
    chain_path: &str,
) {
    for peer in peers {
        if let Some(payload) = request_chain(peer) {
            if let Some(remote) =
                Blockchain::deserialize_chain(&payload, difficulty, miner_pubkey_hash)
            {
                let mut local = chain.lock().unwrap();
                if remote.is_chain_valid() && remote.chain_work() > local.chain_work() {
                    *local = remote;
                    let _ = save_chain_to_disk(chain_path, &local);
                    println!("synced to better chain");
                }
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let port = args
        .next()
        .expect("usage: blockchain_prototype <port> [peer1 peer2 ...] [--miner]");
    let mut is_miner = false;
    let mut peers = Vec::<String>::new();

    for arg in args {
        if arg == "--miner" {
            is_miner = true;
        } else {
            peers.push(arg);
        }
    }

    let difficulty: u32 = 16;
    let wallet = Wallet::new();

    println!("node pubkey_hash (address): {}", wallet.pubkey_hash_hex);

    let chain_data = load_chain_from_disk(CHAIN_PATH, difficulty, &wallet.pubkey_hash_hex)
        .unwrap_or_else(|| Blockchain::new(difficulty, &wallet.pubkey_hash_hex));
    let chain = Arc::new(Mutex::new(chain_data));
    {
        let c = chain.lock().unwrap();
        let _ = save_chain_to_disk(CHAIN_PATH, &c);
    }
    let mempool = Arc::new(Mutex::new(Vec::<Transaction>::new()));
    let mining_flag = Arc::new(Mutex::new(false));

    let listener_chain = Arc::clone(&chain);
    let listener_mempool = Arc::clone(&mempool);
    let listener_peers = peers.clone();
    let miner_pkh = wallet.pubkey_hash_hex.clone();
    let port_for_listener = port.clone();

    thread::spawn(move || {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port_for_listener))
            .expect("failed to bind listener");
        for incoming in listener.incoming() {
            if let Ok(mut stream) = incoming {
                let chain_arc = Arc::clone(&listener_chain);
                let mempool_arc = Arc::clone(&listener_mempool);
                let peers = listener_peers.clone();
                let miner_pkh = miner_pkh.clone();

                thread::spawn(move || {
                    if let Some(msg) = read_message(stream.try_clone().unwrap()) {
                        if msg == "REQ_CHAIN" {
                            let c = chain_arc.lock().unwrap();
                            let payload = c.serialize_chain();
                            let _ = stream.write_all(format!("CHAIN|{}\n", payload).as_bytes());
                            let _ = stream.flush();
                            return;
                        }

                        if msg == "REQ_STATE" {
                            let c = chain_arc.lock().unwrap();
                            let pool = mempool_arc.lock().unwrap();
                            let state = NodeState {
                                chain: c.serialize_chain(),
                                height: c.blocks.len(),
                                tip: c.blocks.last().map(|b| b.hash.clone()).unwrap_or_default(),
                                mempool: pool.clone(),
                                utxo_count: c.utxos.len(),
                                difficulty: c.difficulty,
                            };
                            let payload = serde_json::to_string(&state).unwrap_or_default();
                            let _ = stream.write_all(format!("STATE|{}\n", payload).as_bytes());
                            let _ = stream.flush();
                            return;
                        }

                        if let Some(rest) = msg.strip_prefix("TX|") {
                            if let Some(tx) = Transaction::deserialize(rest) {
                                let c = chain_arc.lock().unwrap();
                                let mut temp = c.utxos.clone();
                                let valid = Blockchain::validate_and_apply_transactions(
                                    &[Transaction::coinbase("dummy", 0), tx.clone()],
                                    &mut temp,
                                );
                                drop(c);

                                if valid {
                                    let mut pool = mempool_arc.lock().unwrap();
                                    if !pool.iter().any(|t| t.txid() == tx.txid()) {
                                        pool.push(tx);
                                    }
                                }
                            }
                            return;
                        }

                        if let Some(rest) = msg.strip_prefix("BLOCK|") {
                            if let Some(block) = Block::deserialize(rest) {
                                let mut c = chain_arc.lock().unwrap();
                                let ok = c.add_block(block.clone());
                                if ok {
                                    let mut pool = mempool_arc.lock().unwrap();
                                    c.remove_txs_from_mempool(&mut pool, &block);
                                    let _ = save_chain_to_disk(CHAIN_PATH, &c);
                                    drop(pool);
                                    drop(c);
                                    broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                                } else {
                                    drop(c);
                                    sync_from_peers(
                                        &chain_arc, &peers, difficulty, &miner_pkh, CHAIN_PATH,
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

    sync_from_peers(
        &chain,
        &peers,
        difficulty,
        &wallet.pubkey_hash_hex,
        CHAIN_PATH,
    );

    println!("Node running on {}", port);
    println!("Commands:");
    println!("  address");
    println!("  send <to_pubkey_hash_hex> <amount>");
    println!("  mine");
    println!("  balance");
    println!("  chain");

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

        match parts[0] {
            "address" => {
                println!("{}", wallet.pubkey_hash_hex);
            }
            "send" => {
                if parts.len() != 3 {
                    println!("usage: send <to_pubkey_hash_hex> <amount>");
                    continue;
                }
                let to = parts[1];
                let amount: u64 = match parts[2].parse() {
                    Ok(v) => v,
                    Err(_) => {
                        println!("invalid amount");
                        continue;
                    }
                };

                let utxos = chain.lock().unwrap().utxos.clone();
                if let Some(tx) = build_transaction(&wallet, to, amount, &utxos) {
                    let mut pool = mempool.lock().unwrap();
                    if !pool.iter().any(|t| t.txid() == tx.txid()) {
                        pool.push(tx.clone());
                    }
                    drop(pool);
                    broadcast(&peers, &format!("TX|{}", tx.serialize()));
                    println!("tx broadcast");
                } else {
                    println!("cannot build tx (funds/signing)");
                }
            }
            "mine" => {
                if !is_miner {
                    println!("mining disabled (use --miner)");
                    continue;
                }
                let flag = Arc::clone(&mining_flag);
                if *flag.lock().unwrap() {
                    println!("mining already in progress");
                    continue;
                }

                *flag.lock().unwrap() = true;
                let chain = Arc::clone(&chain);
                let mempool = Arc::clone(&mempool);
                let peers = peers.clone();
                thread::spawn(move || {
                    let txs = {
                        let mut pool = mempool.lock().unwrap();
                        pool.drain(..).collect::<Vec<_>>()
                    };

                    let mut c = chain.lock().unwrap();
                    let block = c.mine_block_with_txs(txs);
                    drop(c);

                    if let Some(block) = block {
                        let c = chain.lock().unwrap();
                        let _ = save_chain_to_disk(CHAIN_PATH, &c);
                        drop(c);
                        broadcast(&peers, &format!("BLOCK|{}", block.serialize()));
                        println!("mined and broadcast block");
                    } else {
                        println!("mined block rejected");
                    }

                    *flag.lock().unwrap() = false;
                });
            }
            "balance" => {
                let c = chain.lock().unwrap();
                let mut total = 0u64;
                for out in c.utxos.values() {
                    if out.pubkey_hash == wallet.pubkey_hash_hex {
                        total = total.saturating_add(out.amount);
                    }
                }
                println!("{}", total);
            }
            "chain" => {
                let c = chain.lock().unwrap();
                println!("height: {}", c.blocks.len());
                println!("valid: {}", c.is_chain_valid());
                println!("work: {}", c.chain_work());
                println!(
                    "tip: {}",
                    c.blocks.last().map(|b| b.hash.clone()).unwrap_or_default()
                );
            }
            _ => println!("unknown command"),
        }
    }
}
