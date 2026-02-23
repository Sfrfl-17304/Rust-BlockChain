use sha2::{Digest, Sha256};
use std::time::{self, SystemTime, UNIX_EPOCH};

//Block struct
struct Block {
    index: u32,
    timestamp: u64,
    transactions: Vec<String>,
    previous_hash: String,
    hash: String,
    nonce: u32,
}

// Implementing the Block struct and its associated functions
impl Block {
    fn calculate_hash(&self) -> String {
        let tx_data = self.transactions.join(",");
        let input = format!(
            "{}{}{}{}{}",
            self.index, self.timestamp, tx_data, self.previous_hash, self.nonce
        );
        let hash_bytes = Sha256::digest(input.as_bytes());
        let hash_hex = format!("{:x}", hash_bytes);
        hash_hex
    }

    fn new(index: u32, timestamp: u64, transactions: Vec<String>, previous_hash: String) -> Self {
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
        while &self.hash[..difficulty] != target {
            self.nonce += 1;
            self.hash = self.calculate_hash();
        }
    }
}

//Main function to demonstrate the blockchain prototype

fn main() {
    let mut blockchain: Vec<Block> = Vec::new();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();

    let mut genesis_block = Block::new(0, timestamp, "Genesis Block".to_string(), "0".to_string());

    let mut difficulty = 5;

    genesis_block.mine_block(difficulty);
    blockchain.push(genesis_block);

    let mut new_block: Block = Block::new(
        1,
        timestamp,
        "Transaction: Alice pays Bob 10 coins".to_string(),
        blockchain.last().unwrap().hash.clone(),
    );

    println!("block before mining");
    println!("Index: {}", new_block.index);
    println!("Timestamp: {}", new_block.timestamp);
    println!("Transaction: {}", new_block.transaction);
    println!("Previous Hash: {}", new_block.previous_hash);
    println!("Hash: {}", new_block.hash);
    println!("Nonce: {}", new_block.nonce);

    new_block.mine_block(5);

    println!("block after mining!");
    println!("Index: {}", new_block.index);
    println!("Timestamp: {}", new_block.timestamp);
    println!("Transaction: {}", new_block.transaction);
    println!("Previous Hash: {}", new_block.previous_hash);
    println!("Hash: {}", new_block.hash);
    println!("Nonce: {}", new_block.nonce);

    blockchain.push(new_block);

    println!("Blockchain:");
    for block in &blockchain {
        println!("Index: {}", block.index);
        println!("Timestamp: {}", block.timestamp);
        println!("Transaction: {}", block.transaction);
        println!("Previous Hash: {}", block.previous_hash);
        println!("Hash: {}", block.hash);
        println!("Nonce: {}", block.nonce);
    }
}
