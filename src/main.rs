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

//Blockchain struct
struct Blockchain {
    blocks: Vec<Block>,
    difficulty: usize,
}


// Implementing the Blockchain struct and its associated functions
impl Blockchain {
    fn new(difficulty: usize) -> Self {
        let mut blockchain = Blockchain {
            blocks: Vec::new(),
            difficulty,
        };
        let genesis_block = Block::new(0, SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs(), vec!["Genesis Block".to_string()], "0".to_string());
        blockchain.blocks.push(genesis_block);
        blockchain
    }

    fn add_block(&mut self, transactions: Vec<String>) {
        let previous_hash = self.blocks.last().unwrap().hash.clone();
        let index = self.blocks.len() as u32;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();
        let mut new_block = Block::new(index, timestamp, transactions, previous_hash);
        new_block.mine_block(self.difficulty);
        self.blocks.push(new_block);

    fn is_chain_valid(&self) -> bool {
        for i in 1..self.blocks.len() {
            let current_block = &self.blocks[i];
            let previous_block = &self.blocks[i - 1];

            if current_block.hash != current_block.calculate_hash() {
                return false;
            }

            if current_block.previous_hash != previous_block.hash {
                return false;
            }
        }
        true
    }
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
    let difficulty = 4; // Adjust as needed
    let mut blockchain = Blockchain::new(difficulty);

    // Add a new block with a transaction
    let transactions = vec!["Alice pays Bob 10 coins".to_string()];
    blockchain.add_block(transactions);

    // Validate the chain
    println!("Blockchain valid? {}", blockchain.is_chain_valid());

    // Print all blocks
    for block in &blockchain.blocks {
        println!("Block #{}", block.index);
        println!("Timestamp: {}", block.timestamp);
        println!("Transactions: {:?}", block.transactions);
        println!("Previous Hash: {}", block.previous_hash);
        println!("Hash: {}", block.hash);
        println!("Nonce: {}", block.nonce);
        println!();
    }
}
