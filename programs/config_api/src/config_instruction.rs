use crate::config_state::ConfigState;
use crate::id;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::system_instruction::SystemInstruction;
use solana_sdk::transaction_builder::BuilderInstruction;

pub struct ConfigInstruction {}

impl ConfigInstruction {
    pub fn new_config(
        from_account_pubkey: &Pubkey,
        config_account_pubkey: &Pubkey,
        lamports: u64,
        data_len: u64,
    ) -> BuilderInstruction {
        SystemInstruction::new_program_account(
            from_account_pubkey,
            config_account_pubkey,
            lamports,
            ConfigState::max_size(data_len),
            &id(),
        )
    }

    pub fn new_store(
        config_account_pubkey: &Pubkey,
        config_state: &ConfigState,
    ) -> BuilderInstruction {
        BuilderInstruction::new(id(), config_state, vec![(*config_account_pubkey, true)])
    }
}
