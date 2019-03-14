use crate::check_id;
use crate::config_instruction::ConfigInstruction;
use crate::config_state::ConfigState;
use bincode::deserialize;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, KeypairUtil};
use solana_sdk::transaction::Transaction;
use solana_sdk::transaction_builder::TransactionBuilder;

pub struct ConfigTransaction {}

impl ConfigTransaction {
    /// Create a new, empty configuration account
    pub fn new_config(
        from_keypair: &Keypair,
        config_account_pubkey: &Pubkey,
        data_len: u64,
        recent_blockhash: Hash,
        lamports: u64,
        fee: u64,
    ) -> Transaction {
        TransactionBuilder::new(fee)
            .push(ConfigInstruction::new_config(
                &from_keypair.pubkey(),
                config_account_pubkey,
                lamports,
                ConfigState::max_size(data_len),
            ))
            .sign(&[from_keypair], recent_blockhash)
    }

    /// Store new state in a configuration account
    pub fn new_store(
        config_account_keypair: &Keypair,
        config_state: &ConfigState,
        recent_blockhash: Hash,
        fee: u64,
    ) -> Transaction {
        TransactionBuilder::new(fee)
            .push(ConfigInstruction::new_store(
                &config_account_keypair.pubkey(),
                config_state,
            ))
            .sign(&[config_account_keypair], recent_blockhash)
    }

    pub fn get_config(transaction: &Transaction, instruction_index: usize) -> Option<ConfigState> {
        if !check_id(&transaction.program_id(instruction_index)) {
            return None;
        }
        Some(deserialize(&transaction.data(instruction_index)).unwrap())
    }

    pub fn get_configs(transaction: &Transaction) -> Vec<ConfigState> {
        (0..transaction.instructions.len())
            .filter_map(|i| Self::get_config(transaction, i))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_configs_none() {
        let keypair = Keypair::new();
        let recent_blockhash = Hash::default();
        let transaction =
            ConfigTransaction::new_config(&keypair, &keypair.pubkey(), 42, recent_blockhash, 1, 0);
        assert_eq!(ConfigTransaction::get_configs(&transaction), vec![]);
    }

    #[test]
    fn test_get_configs_one() {
        let keypair = Keypair::new();
        let recent_blockhash = Hash::default();
        let config_state = ConfigState::new(42);
        let transaction =
            ConfigTransaction::new_store(&keypair, &config_state, recent_blockhash, 0);
        assert_eq!(
            ConfigTransaction::get_configs(&transaction),
            vec![ConfigState::new(42)]
        );
    }
}
