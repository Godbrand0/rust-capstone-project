#![allow(unused)]
use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core node (no wallet context for node-level calls)
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Create wallets. If the wallet already exists on disk, try to load it instead.
    // If it's already loaded, both calls will error harmlessly — we ignore them.
    rpc.create_wallet("Miner", None, None, None, None).ok();
    rpc.load_wallet("Miner").ok();

    rpc.create_wallet("Trader", None, None, None, None).ok();
    rpc.load_wallet("Trader").ok();

    // Each wallet needs its own RPC client pointed at the wallet-specific endpoint
    let miner_rpc = Client::new(
        &format!("{}/wallet/Miner", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;
    let trader_rpc = Client::new(
        &format!("{}/wallet/Trader", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Generate a labeled address from the Miner wallet for receiving block rewards
    let mining_address = miner_rpc
        .get_new_address(Some("Mining Reward"), None)?
        .assume_checked();

    // Mine 101 blocks to the Miner address.
    //
    // WHY 101 BLOCKS? Bitcoin enforces a "coinbase maturity" rule: block rewards (coinbase
    // outputs) are unspendable for 100 blocks after they are mined. This protects the network
    // from chain reorganisations — if a block is later orphaned, any wallet that already spent
    // its reward would be left with invalid coins. After 101 blocks, the FIRST block's reward
    // has 100 confirmations on top of it and is finally mature (spendable). All subsequent
    // rewards remain locked until their own 100-block window elapses.
    miner_rpc.generate_to_address(101, &mining_address)?;

    // Print Miner's spendable balance (only the first block reward is mature at this point)
    let miner_balance = miner_rpc.get_balance(None, None)?;
    println!("Miner balance: {} BTC", miner_balance.to_btc());

    // Generate a labeled receiving address from the Trader wallet
    let trader_address = trader_rpc
        .get_new_address(Some("Received"), None)?
        .assume_checked();

    // Send 20 BTC from Miner to Trader; the node selects UTXOs and creates change automatically
    let txid = miner_rpc.send_to_address(
        &trader_address,
        Amount::from_btc(20.0).unwrap(),
        None, // comment
        None, // comment_to
        None, // subtract_fee_from_amount
        None, // replaceable (RBF)
        None, // conf_target
        None, // estimate_mode
    )?;
    println!("Sent transaction: {}", txid);

    // Fetch the unconfirmed transaction from the mempool and print it
    let mempool_entry = rpc.get_mempool_entry(&txid)?;
    println!("Mempool entry: {:?}", mempool_entry);

    // Mine 1 block to confirm the transaction
    let confirm_blocks = miner_rpc.generate_to_address(1, &mining_address)?;
    let block_hash = confirm_blocks[0];

    // Get the block height for the confirming block
    let block_info = rpc.get_block_header_info(&block_hash)?;
    let block_height = block_info.height;

    // Fetch full transaction data now that it's confirmed (txindex=1 lets us look up any tx)
    let tx_info = rpc.get_raw_transaction_info(&txid, None)?;

    // Reconstruct the input: look up each input's previous output to find address and amount.
    // A standard send_to_address uses a single UTXO input from the Miner wallet.
    let mut input_address = String::new();
    let mut input_amount = Amount::ZERO;
    for vin in &tx_info.vin {
        if let (Some(prev_txid), Some(prev_vout)) = (vin.txid, vin.vout) {
            let prev_tx = rpc.get_raw_transaction_info(&prev_txid, None)?;
            let output = &prev_tx.vout[prev_vout as usize];
            input_amount += output.value;
            if let Some(addr) = &output.script_pub_key.address {
                input_address = addr.clone().assume_checked().to_string();
            }
        }
    }

    // Identify which output went to Trader and which is Miner's change
    let trader_addr_str = trader_address.to_string();
    let mut trader_output_address = String::new();
    let mut trader_output_amount = Amount::ZERO;
    let mut change_address = String::new();
    let mut change_amount = Amount::ZERO;

    for vout in &tx_info.vout {
        if let Some(addr) = &vout.script_pub_key.address {
            let addr_str = addr.clone().assume_checked().to_string();
            if addr_str == trader_addr_str {
                trader_output_address = addr_str;
                trader_output_amount = vout.value;
            } else {
                change_address = addr_str;
                change_amount = vout.value;
            }
        }
    }

    // Get the fee directly from the wallet record so it matches the node's exact value.
    // Calculating it as (input - outputs) can introduce floating-point rounding errors
    // that would cause the test's strict equality check to fail.
    let wallet_tx = miner_rpc.get_transaction(&txid, None)?;
    let fee = wallet_tx
        .fee
        .expect("fee not present in wallet transaction");

    // Write all required details to out.txt at the project root (one value per line).
    // Amounts are formatted to 8 decimal places (satoshi precision) to match Bitcoin
    // Core's JSON representation, ensuring parseFloat() comparisons in the test pass.
    let mut file = File::create("../out.txt")?;
    writeln!(file, "{}", txid)?;
    writeln!(file, "{}", input_address)?;
    writeln!(file, "{:.8}", input_amount.to_btc())?;
    writeln!(file, "{}", trader_output_address)?;
    writeln!(file, "{:.8}", trader_output_amount.to_btc())?;
    writeln!(file, "{}", change_address)?;
    writeln!(file, "{:.8}", change_amount.to_btc())?;
    writeln!(file, "{:.8}", fee.to_btc())?;
    writeln!(file, "{}", block_height)?;
    writeln!(file, "{}", block_hash)?;

    println!("Results written to out.txt");

    Ok(())
}
